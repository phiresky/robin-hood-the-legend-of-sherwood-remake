use serde::{Deserialize, Serialize};

use crate::{
    element::{Entity, EntityId},
    engine::{LevelAssets, ShieldState, WeatherState},
    entities::Entities,
    fast_find_grid::FastFindGrid,
    pathfinder::PathFinder,
    sight_obstacle::SightObstacle,
};

/// Authoritative entity storage and the spatial state indexed alongside it.
///
/// The parallel collections remain stored in their original order. Validation
/// checks their relationships at attachment/snapshot boundaries; it never
/// rebuilds or reorders them from entity or level scans.
#[derive(Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct WorldState {
    pub(crate) entities: Entities,
    pub(crate) pc_ids: Vec<EntityId>,
    pub(crate) fast_grid: FastFindGrid,
    pub(crate) pathfinder: PathFinder,
    pub(crate) weather: WeatherState,
    pub(crate) shield: ShieldState,
    pub(crate) dynamic_sight_obstacles: Vec<SightObstacle>,
    pub(crate) static_sight_obstacle_active: Vec<bool>,
    pub(crate) mobile_elements: Vec<crate::mobile::MobileElement>,
}

impl WorldState {
    pub(crate) fn new() -> Self {
        Self {
            entities: Entities::new(),
            pc_ids: Vec::new(),
            fast_grid: FastFindGrid::default(),
            pathfinder: PathFinder::default(),
            weather: WeatherState::new(),
            shield: ShieldState::default(),
            dynamic_sight_obstacles: Vec::new(),
            static_sight_obstacle_active: Vec::new(),
            mobile_elements: Vec::new(),
        }
    }

    /// Reattach immutable level topology and sprite runtimes after decoding.
    ///
    /// The caller must first run `preflight_level_assets` across the
    /// complete candidate; this phase is then infallible and mutation-only.
    pub(crate) fn attach_preflighted_level_assets(&mut self, assets: &LevelAssets) {
        self.fast_grid.attach_level_grid(assets.level_grid.clone());

        for (_, entity) in self.entities.occupied_mut() {
            entity
                .sprite_mut()
                .attach_preflighted_runtime_from_cache(&assets.sprite_scriptor);
        }
    }

    pub(crate) fn validate_level_attachments(
        &self,
        assets: &LevelAssets,
        script_zone_count: usize,
    ) {
        if let Err(detail) = self.preflight_level_assets(assets, script_zone_count) {
            panic!("{detail}");
        }
    }

    pub(crate) fn preflight_level_assets(
        &self,
        assets: &LevelAssets,
        script_zone_count: usize,
    ) -> Result<(), String> {
        self.validate_pc_index_inner()?;

        if script_zone_count != assets.script_zone_grid_indices.len() {
            return Err(format!(
                "script-zone runtime length {} does not match level zone-index length {}",
                script_zone_count,
                assets.script_zone_grid_indices.len(),
            ));
        }
        for (zone_idx, &grid_idx) in assets.script_zone_grid_indices.iter().enumerate() {
            if (grid_idx as usize) >= assets.level_grid.sectors.len() {
                return Err(format!(
                    "script zone {zone_idx} references grid sector {grid_idx}, but the level has {} sectors",
                    assets.level_grid.sectors.len(),
                ));
            }
        }

        if self.static_sight_obstacle_active.len() != assets.static_sight_obstacles.len() {
            return Err(format!(
                "static sight-obstacle runtime length {} does not match level obstacle length {}",
                self.static_sight_obstacle_active.len(),
                assets.static_sight_obstacles.len(),
            ));
        }
        if self.mobile_elements.len() != assets.mobile_element_count {
            return Err(format!(
                "snapshot mobile-element count {} does not match loaded level count {}",
                self.mobile_elements.len(),
                assets.mobile_element_count,
            ));
        }

        self.validate_pathfinder_states_inner(assets)?;
        self.validate_fast_grid_runtime_lengths(assets)?;
        self.validate_fast_grid_indices_against(assets.level_grid.sectors.len())?;

        for (id, entity) in self.entities.occupied() {
            entity
                .sprite()
                .validate_runtime_cache(&assets.sprite_scriptor)
                .map_err(|detail| {
                    format!("entity {} sprite attachment failed: {detail}", id.index())
                })?;
        }
        for (mobile_index, mobile) in self.mobile_elements.iter().enumerate() {
            for &sprite_id in &mobile.sprite_ids {
                if !matches!(self.entities.get(sprite_id), Some(Entity::Fx(_))) {
                    return Err(format!(
                        "mobile {mobile_index} references missing or non-FX sprite entity {sprite_id}"
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_pc_index_inner(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::with_capacity(self.pc_ids.len());
        for &id in &self.pc_ids {
            if !seen.insert(id) {
                return Err(format!("pc_ids contains duplicate entity {id}"));
            }
            if !matches!(self.entities.get(id), Some(Entity::Pc(_))) {
                return Err(format!("pc_ids references missing or non-PC entity {id}"));
            }
        }

        let entity_pc_count = self.entities.pcs().count();
        if self.pc_ids.len() != entity_pc_count {
            return Err(format!(
                "pc_ids contains {} entries but entity storage contains {entity_pc_count} PCs",
                self.pc_ids.len(),
            ));
        }
        Ok(())
    }

    fn validate_pathfinder_states_inner(&self, assets: &LevelAssets) -> Result<(), String> {
        if self.pathfinder.states.len() != assets.pathfinder_graph.states.len() {
            return Err(format!(
                "pathfinder state layer count {} does not match level graph layer count {}",
                self.pathfinder.states.len(),
                assets.pathfinder_graph.states.len(),
            ));
        }
        for (layer_idx, (runtime, level)) in self
            .pathfinder
            .states
            .iter()
            .zip(&assets.pathfinder_graph.states)
            .enumerate()
        {
            if runtime.len() != level.len() {
                return Err(format!(
                    "pathfinder state area count for layer {layer_idx} is {} but level graph has {}",
                    runtime.len(),
                    level.len(),
                ));
            }
        }
        Ok(())
    }

    fn validate_fast_grid_runtime_lengths(&self, assets: &LevelAssets) -> Result<(), String> {
        let lengths = [
            (
                "line",
                self.fast_grid.line_active.len(),
                assets.level_grid.lines.len(),
            ),
            (
                "sector",
                self.fast_grid.sector_active.len(),
                assets.level_grid.sectors.len(),
            ),
            (
                "mask",
                self.fast_grid.mask_active.len(),
                assets.level_grid.masks.len(),
            ),
        ];
        for (name, runtime_len, level_len) in lengths {
            if runtime_len != level_len {
                return Err(format!(
                    "fast-grid {name} runtime length {runtime_len} does not match level {name}s {level_len}"
                ));
            }
        }
        Ok(())
    }

    fn validate_fast_grid_indices_against(&self, sector_count: usize) -> Result<(), String> {
        for &sector_idx in self.fast_grid.lift_state.keys() {
            if (sector_idx as usize) >= sector_count {
                return Err(format!(
                    "fast-grid lift runtime references sector {sector_idx}, but the level has {sector_count} sectors"
                ));
            }
        }
        for &sector_idx in self.fast_grid.sector_type_overlay.keys() {
            if (sector_idx as usize) >= sector_count {
                return Err(format!(
                    "fast-grid sector-type overlay references sector {sector_idx}, but the level has {sector_count} sectors"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_world_accepts_empty_level_attachments() {
        WorldState::new().validate_level_attachments(&LevelAssets::new(), 0);
    }

    #[test]
    #[should_panic(
        expected = "script-zone runtime length 1 does not match level zone-index length 0"
    )]
    fn script_zone_parallel_length_mismatch_fails_loudly() {
        let world = WorldState::new();
        world.validate_level_attachments(&LevelAssets::new(), 1);
    }

    #[test]
    #[should_panic(
        expected = "static sight-obstacle runtime length 1 does not match level obstacle length 0"
    )]
    fn static_obstacle_parallel_length_mismatch_fails_loudly() {
        let mut world = WorldState::new();
        world.static_sight_obstacle_active.push(true);
        world.validate_level_attachments(&LevelAssets::new(), 0);
    }

    #[test]
    #[should_panic(
        expected = "pathfinder state layer count 1 does not match level graph layer count 0"
    )]
    fn pathfinder_parallel_length_mismatch_fails_loudly() {
        let mut world = WorldState::new();
        world.pathfinder.states.push(Vec::new());
        world.validate_level_attachments(&LevelAssets::new(), 0);
    }

    #[test]
    #[should_panic(expected = "pc_ids references missing or non-PC entity")]
    fn missing_pc_index_target_fails_loudly() {
        let mut world = WorldState::new();
        world.pc_ids.push(EntityId::Pc(crate::entity_id::PcId(0)));
        world.validate_level_attachments(&LevelAssets::new(), 0);
    }

    #[test]
    #[should_panic(
        expected = "script zone 0 references grid sector 0, but the level has 0 sectors"
    )]
    fn out_of_bounds_script_zone_index_fails_loudly() {
        let world = WorldState::new();
        let mut assets = LevelAssets::new();
        assets.script_zone_grid_indices = std::sync::Arc::new(vec![0]);
        world.validate_level_attachments(&assets, 1);
    }
}
