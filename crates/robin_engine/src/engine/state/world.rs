use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    element::{Entity, EntityId},
    engine::{LevelAssets, ShieldState, WeatherState},
    entities::Entities,
    fast_find_grid::FastFindGrid,
    pathfinder::PathFinder,
    sight_obstacle::SightObstacle,
};

/// Encode a map keyed by [`EntityId`] as a pair list.
///
/// A JSON object key must be a string, and `EntityId` is an enum; a pair list
/// also keeps the length known up front, which the binary rollback snapshot
/// encoder requires.
mod entity_creation_order_pairs {
    use super::{BTreeMap, EntityId};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(
        map: &BTreeMap<EntityId, u32>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter()
            .map(|(id, order)| (*id, *order))
            .collect::<Vec<(EntityId, u32)>>()
            .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<EntityId, u32>, D::Error> {
        Ok(Vec::<(EntityId, u32)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}

/// Authoritative entity storage and the spatial state indexed alongside it.
///
/// The parallel collections remain stored in their original order. Validation
/// checks their relationships at attachment/snapshot boundaries; it never
/// rebuilds or reorders them from entity or level scans.
#[derive(
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct WorldState {
    pub(crate) entities: Entities,
    pub(crate) soldier_registry: SoldierRegistry,
    /// NPC publication order, including inactive soldiers and civilians.
    pub(crate) npc_registry_ids: Vec<EntityId>,
    /// Canonical actor publication order for collision and projectile scans.
    pub(crate) actor_registry_ids: Vec<EntityId>,
    /// PC and soldier publication order; camp queries retain this subsequence order.
    pub(crate) fighter_registry_ids: Vec<EntityId>,
    /// Portrait/UI order, sorted by character-profile priority after loading.
    pub(crate) pc_ids: Vec<EntityId>,
    /// Exact original-game player-character insertion order.
    ///
    /// Original gameplay loops use this registry independently of the
    /// priority-sorted portrait bar. Keep it as authoritative hashed state so
    /// first-match scans and synchronous callbacks survive rollback exactly.
    pub(crate) original_pc_registry_ids: Vec<EntityId>,
    /// Spatial grid shared with rollback snapshots, which hold
    /// `Arc` clones frozen at their creation instant. Mutations
    /// go through [`Self::fast_grid_mut`] so a shared grid
    /// is copied exactly once before diverging.
    pub(crate) fast_grid: std::sync::Arc<FastFindGrid>,
    pub(crate) pathfinder: PathFinder,
    pub(crate) weather: WeatherState,
    pub(crate) shield: ShieldState,
    pub(crate) dynamic_sight_obstacles: Vec<SightObstacle>,
    pub(crate) static_sight_obstacle_active: Vec<bool>,
    pub(crate) mobile_elements: Vec<crate::mobile::MobileElement>,
    /// Exact original-game creation-order identity for every entity created
    /// by the Original-compatible simulation.
    ///
    /// This cannot be reconstructed from the Rust entity-table index:
    /// Original mobile masters consume creation orders without occupying a
    /// Rust `Entities` slot, and Rust batches authored entity categories
    /// differently while loading a mission.
    ///
    /// `EntityId` is an enum, so it cannot be a JSON object key. Encode the
    /// map as a length-prefixed pair list, which both the JSON engine dump
    /// and the length-strict binary rollback snapshot accept.
    #[serde(with = "entity_creation_order_pairs")]
    pub(crate) original_creation_order_by_entity: BTreeMap<EntityId, u32>,
    /// Original-game process-wide creation counter.
    pub(crate) next_original_creation_order: u32,
    /// Original-game process-wide repulsive-point counter.
    ///
    /// This is distinct from `AiGlobalState::next_repulsive_point_id`: the
    /// latter is a Rust script/native registry counter, while the Original
    /// initialization assigns this identity to every repulsive point.
    pub(crate) original_repulsive_point_counter: u32,
}

impl WorldState {
    pub(crate) fn persisted_clone(&self) -> Self {
        let value = self;
        let WorldState {
            entities: _,
            soldier_registry: _,
            npc_registry_ids: _,
            actor_registry_ids: _,
            fighter_registry_ids: _,
            pc_ids: _,
            original_pc_registry_ids: _,
            fast_grid: _,
            pathfinder: _,
            weather: _,
            shield: _,
            dynamic_sight_obstacles: _,
            static_sight_obstacle_active: _,
            mobile_elements: _,
            original_creation_order_by_entity: _,
            next_original_creation_order: _,
            original_repulsive_point_counter: _,
        } = value;
        Self {
            entities: value.entities.persisted_projection(),
            soldier_registry: value.soldier_registry.clone(),
            npc_registry_ids: value.npc_registry_ids.clone(),
            actor_registry_ids: value.actor_registry_ids.clone(),
            fighter_registry_ids: value.fighter_registry_ids.clone(),
            pc_ids: value.pc_ids.clone(),
            original_pc_registry_ids: value.original_pc_registry_ids.clone(),
            fast_grid: std::sync::Arc::new(value.fast_grid.persisted_clone()),
            pathfinder: value.pathfinder.clone(),
            weather: value.weather.clone(),
            shield: value.shield.clone(),
            dynamic_sight_obstacles: value.dynamic_sight_obstacles.clone(),
            static_sight_obstacle_active: value.static_sight_obstacle_active.clone(),
            mobile_elements: value.mobile_elements.clone(),
            original_creation_order_by_entity: value.original_creation_order_by_entity.clone(),
            next_original_creation_order: value.next_original_creation_order,
            original_repulsive_point_counter: value.original_repulsive_point_counter,
        }
    }
}

/// Ordered membership maintained at soldier publication and removal boundaries.
#[derive(
    Debug,
    Default,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct SoldierRegistry {
    all: Vec<u32>,
    camps: BTreeMap<u16, Vec<u32>>,
}

impl SoldierRegistry {
    pub(crate) fn all(&self) -> &[u32] {
        &self.all
    }

    pub(crate) fn camp(&self, camp: crate::element::Camp) -> &[u32] {
        let camp = camp
            .allegiance_id()
            .expect("soldier roster requires a valid camp");
        self.camps.get(&camp).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn register(&mut self, id: EntityId, camp: crate::element::Camp) {
        assert!(
            matches!(id, EntityId::Soldier(_)),
            "soldier roster requires a soldier ID"
        );
        assert!(!self.all.contains(&id.index()), "soldier registered twice");
        let camp = camp
            .allegiance_id()
            .expect("soldier publication requires a valid camp");
        self.all.push(id.index());
        self.camps.entry(camp).or_default().push(id.index());
    }

    pub(crate) fn remove(&mut self, id: EntityId) {
        if !matches!(id, EntityId::Soldier(_)) {
            return;
        }
        self.all.retain(|&handle| handle != id.index());
        self.camps.retain(|_, members| {
            members.retain(|&handle| handle != id.index());
            !members.is_empty()
        });
    }

    fn validate(&self, entities: &Entities) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        let mut camp_positions = BTreeMap::<u16, usize>::new();
        for &handle in &self.all {
            if !seen.insert(handle) {
                return Err(format!("soldier registry contains duplicate {handle}"));
            }
            let id = crate::entity_id::SoldierId(handle);
            let soldier = entities
                .get_soldier(id)
                .ok_or_else(|| format!("soldier registry references missing soldier {handle}"))?;
            let camp = soldier
                .soldier
                .cached_camp
                .allegiance_id()
                .ok_or_else(|| format!("registered soldier {handle} has an invalid camp"))?;
            let index = camp_positions.entry(camp).or_default();
            if self
                .camps
                .get(&camp)
                .and_then(|members| members.get(*index))
                != Some(&handle)
            {
                return Err(format!(
                    "soldier {handle} has inconsistent camp registration order"
                ));
            }
            *index += 1;
        }
        if entities.soldiers().count() != self.all.len()
            || self.camps.iter().any(|(camp, members)| {
                members.is_empty() || camp_positions.get(camp).copied() != Some(members.len())
            })
        {
            return Err("soldier registry membership does not match the world".into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn rebuild_from_order(
        &mut self,
        entities: &Entities,
        order: impl IntoIterator<Item = EntityId>,
    ) {
        *self = Self::default();
        for id in order {
            self.register(
                id,
                entities
                    .expect_entity(id, format_args!("soldier roster fixture"))
                    .camp(),
            );
        }
    }
}

impl WorldState {
    /// Player-character registration order used by Original first-match scans.
    /// This is independent of portrait priority and Rust entity slots. Callers
    /// needing a mutable world during iteration must explicitly copy this view.
    pub(in crate::engine) fn original_pc_registry(&self) -> &[EntityId] {
        &self.original_pc_registry_ids
    }

    /// Combined static + dynamic sight obstacles. Static come from
    /// `LevelAssets::static_sight_obstacles` (Arc-shared, populated at
    /// level load); dynamic are this frame's shields. Returns a
    /// `ObstacleList` view that exposes the flat global indexing used
    /// by patches and per-actor obstacle references.
    pub(crate) fn sight_obstacles<'a>(
        &'a self,
        assets: &'a LevelAssets,
    ) -> crate::sight_obstacle::ObstacleList<'a> {
        crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.environment.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.dynamic_sight_obstacles,
            static_active: &self.static_sight_obstacle_active,
        }
    }

    /// The mutable entity table beside the read-only sight-obstacle view and
    /// spatial grid, for systems that ray-cast while mutating an entity.
    pub(crate) fn entities_mut_with_sight<'a>(
        &'a mut self,
        assets: &'a LevelAssets,
    ) -> (
        &'a mut Entities,
        crate::sight_obstacle::ObstacleList<'a>,
        &'a std::sync::Arc<FastFindGrid>,
        &'a [EntityId],
    ) {
        let Self {
            entities,
            fast_grid,
            dynamic_sight_obstacles,
            static_sight_obstacle_active,
            actor_registry_ids,
            ..
        } = self;
        (
            entities,
            crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.environment.static_sight_obstacles.as_slice(),
                dynamic_obstacles: dynamic_sight_obstacles,
                static_active: static_sight_obstacle_active,
            },
            fast_grid,
            actor_registry_ids,
        )
    }

    /// The projectile helper plus thirty Original object masters are
    /// constructed before the first mission element.
    pub(crate) const FIRST_MISSION_CREATION_ORDER: u32 = 31;

    pub(crate) fn new() -> Self {
        Self {
            entities: Entities::new(),
            soldier_registry: SoldierRegistry::default(),
            npc_registry_ids: Vec::new(),
            actor_registry_ids: Vec::new(),
            fighter_registry_ids: Vec::new(),
            pc_ids: Vec::new(),
            original_pc_registry_ids: Vec::new(),
            fast_grid: std::sync::Arc::new(FastFindGrid::default()),
            pathfinder: PathFinder::default(),
            weather: WeatherState::new(),
            shield: ShieldState::default(),
            dynamic_sight_obstacles: Vec::new(),
            static_sight_obstacle_active: Vec::new(),
            mobile_elements: Vec::new(),
            original_creation_order_by_entity: BTreeMap::new(),
            next_original_creation_order: Self::FIRST_MISSION_CREATION_ORDER,
            original_repulsive_point_counter: 0,
        }
    }

    /// Split the exact world-owned leaves used by the path scheduling barrier.
    ///
    /// Keeping this split on the aggregate owner prevents the scheduler from
    /// receiving authority over weather, shields, sight-obstacle overlays, or
    /// mobile elements merely to advance the pathfinder queue.
    pub(in crate::engine) fn path_schedule_parts(
        &mut self,
    ) -> (&Entities, &FastFindGrid, &mut PathFinder) {
        (
            &self.entities,
            self.fast_grid.as_ref(),
            &mut self.pathfinder,
        )
    }

    /// Copy-on-write mutable access to the spatial grid.
    ///
    /// The grid is `Arc`-shared with rollback snapshots.
    /// Mutating through this accessor copies the runtime grid
    /// state only when such a shared snapshot is still alive, which keeps
    /// every snapshot frozen at the state it observed when it was taken.
    #[inline]
    pub(crate) fn fast_grid_mut(&mut self) -> &mut FastFindGrid {
        std::sync::Arc::make_mut(&mut self.fast_grid)
    }

    pub(crate) fn assign_next_original_creation_order(&mut self, entity_id: EntityId) {
        let creation_order = self.reserve_next_original_creation_order();
        self.assign_reserved_original_creation_order(entity_id, creation_order);
    }

    /// Consume an original-game creation identity during initialization before the
    /// element is published. A purse element is constructed before its
    /// first update, but that update can add child coins before
    /// Original-game element insertion places the purse in the element collection.
    pub(crate) fn reserve_next_original_creation_order(&mut self) -> u32 {
        let creation_order = self.next_original_creation_order;
        self.next_original_creation_order = creation_order
            .checked_add(1)
            .expect("Original element creation counter overflow");
        creation_order
    }

    /// Attach a previously consumed constructor identity to the eventual
    /// entity-array occupant.
    pub(crate) fn assign_reserved_original_creation_order(
        &mut self,
        entity_id: EntityId,
        creation_order: u32,
    ) {
        tracing::trace!(
            target: "robin_engine::creation_order",
            "assign creation order {creation_order} to {entity_id}"
        );
        if let Some(previous) = self
            .original_creation_order_by_entity
            .insert(entity_id, creation_order)
        {
            panic!(
                "entity {entity_id} already had Original creation order {previous} while assigning {creation_order}"
            );
        }
    }

    /// Replace provisional load-time identities with the exact authored
    /// sequence after all static mission elements and mobile masters exist.
    pub(crate) fn install_original_creation_orders(
        &mut self,
        creation_order_by_entity: BTreeMap<EntityId, u32>,
        next_original_creation_order: u32,
    ) {
        self.original_creation_order_by_entity = creation_order_by_entity;
        self.next_original_creation_order = next_original_creation_order;
        // Rust constructs authored categories in loader-friendly batches, but
        // Original-game element insertion populates players in construction order.
        // Normalize the provisional registry once authoritative topology is
        // installed; subsequent runtime additions retain stable append order.
        let creation_orders = &self.original_creation_order_by_entity;
        for registry in [&mut self.actor_registry_ids, &mut self.fighter_registry_ids] {
            registry.sort_by_key(|id| {
                *creation_orders
                    .get(id)
                    .expect("published actor lacks creation order")
            });
        }
        self.npc_registry_ids.sort_by_key(|&entity_id| {
            creation_orders.get(&entity_id).copied().unwrap_or_else(|| {
                panic!("NPC {entity_id} lacks a creation order while installing its registry")
            })
        });
        self.original_pc_registry_ids.sort_by_key(|&entity_id| {
            creation_orders.get(&entity_id).copied().unwrap_or_else(|| {
                panic!(
                    "PC {entity_id} has no authoritative Original creation order while installing the engine PC registry"
                )
            })
        });
    }

    pub(crate) fn original_creation_order(&self, entity_id: EntityId) -> u32 {
        self.original_creation_order_by_entity
            .get(&entity_id)
            .copied()
            .unwrap_or_else(|| {
                panic!("entity {entity_id} has no authoritative Original creation order")
            })
    }

    /// Reattach immutable level topology and sprite runtimes after decoding.
    ///
    /// The caller must first run `preflight_level_assets` across the
    /// complete candidate; this phase is then infallible and mutation-only.
    pub(crate) fn attach_preflighted_level_assets(&mut self, assets: &LevelAssets) {
        self.fast_grid_mut()
            .attach_level_grid(assets.navigation.level_grid.clone());

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
        self.soldier_registry.validate(&self.entities)?;
        for (name, registry, fighter_only) in [
            ("actor", &self.actor_registry_ids, false),
            ("fighter", &self.fighter_registry_ids, true),
        ] {
            let eligible = |entity: &crate::element::Entity| {
                if fighter_only {
                    matches!(
                        entity,
                        crate::element::Entity::Pc(_) | crate::element::Entity::Soldier(_)
                    )
                } else {
                    entity.actor_data().is_some()
                }
            };
            let mut members = std::collections::BTreeSet::new();
            for &id in registry {
                if !members.insert(id) || !self.entities.get(id).is_some_and(&eligible) {
                    return Err(format!(
                        "{name} registry contains duplicate or invalid member {id}"
                    ));
                }
            }
            if members.len()
                != self
                    .entities
                    .occupied()
                    .filter(|(_, entity)| eligible(entity))
                    .count()
            {
                return Err(format!(
                    "{name} registry does not include every published member"
                ));
            }
        }
        let mut npc_members = std::collections::BTreeSet::new();
        for &id in &self.npc_registry_ids {
            if !npc_members.insert(id)
                || !self
                    .entities
                    .get(id)
                    .is_some_and(|entity| entity.npc_data().is_some())
            {
                return Err(format!(
                    "NPC registry contains duplicate or invalid member {id}"
                ));
            }
        }
        if npc_members.len() != self.entities.npc_ids().count() {
            return Err("NPC registry does not include every published NPC".into());
        }

        if script_zone_count != assets.scripts.zone_grid_indices.len() {
            return Err(format!(
                "script-zone runtime length {} does not match level zone-index length {}",
                script_zone_count,
                assets.scripts.zone_grid_indices.len(),
            ));
        }
        for (zone_idx, &grid_idx) in assets.scripts.zone_grid_indices.iter().enumerate() {
            if (grid_idx as usize) >= assets.navigation.level_grid.sectors.len() {
                return Err(format!(
                    "script zone {zone_idx} references grid sector {grid_idx}, but the level has {} sectors",
                    assets.navigation.level_grid.sectors.len(),
                ));
            }
        }

        if self.static_sight_obstacle_active.len()
            != assets.environment.static_sight_obstacles.len()
        {
            return Err(format!(
                "static sight-obstacle runtime length {} does not match level obstacle length {}",
                self.static_sight_obstacle_active.len(),
                assets.environment.static_sight_obstacles.len(),
            ));
        }
        if self.mobile_elements.len() != assets.entities.mobile_element_count {
            return Err(format!(
                "snapshot mobile-element count {} does not match loaded level count {}",
                self.mobile_elements.len(),
                assets.entities.mobile_element_count,
            ));
        }

        self.validate_pathfinder_states_inner(assets)?;
        self.validate_fast_grid_runtime_lengths(assets)?;
        self.validate_fast_grid_indices_against(assets.navigation.level_grid.sectors.len())?;

        for (id, entity) in self.entities.occupied() {
            if let Some(enemy) = entity.enemy_ai() {
                if assets
                    .profile_manager
                    .get_soldier(enemy.behavior_profile)
                    .is_none()
                {
                    return Err(format!(
                        "entity {id} references missing behavior profile {}",
                        enemy.behavior_profile.0
                    ));
                }
            }
            entity
                .sprite()
                .validate_runtime_cache(&assets.sprite_scriptor)
                .map_err(|detail| {
                    format!("entity {} sprite attachment failed: {detail}", id.index())
                })?;
        }
        for (mobile_index, mobile) in self.mobile_elements.iter().enumerate() {
            let mobile_index_u16 = u16::try_from(mobile_index)
                .map_err(|_| format!("mobile index {mobile_index} does not fit in u16"))?;
            let first = *mobile.sprite_ids.first().ok_or_else(|| {
                format!("mobile {mobile_index} has no first masked child owner boundary")
            })?;
            for (offset, &sprite_id) in mobile.sprite_ids.iter().enumerate() {
                let expected_slot = first.index().checked_add(offset as u32).ok_or_else(|| {
                    format!("mobile {mobile_index} child adjacency overflows after {first}")
                })?;
                let actual_id = self.entities.id_at_legacy_slot(expected_slot).ok_or_else(|| {
                    format!(
                        "mobile {mobile_index} child {sprite_id} is missing from required adjacent slot {expected_slot}"
                    )
                })?;
                if actual_id != sprite_id {
                    return Err(format!(
                        "mobile {mobile_index} child {sprite_id} expected at adjacent slot {expected_slot}, found {actual_id}"
                    ));
                }
                let fx = self.entities.get(sprite_id).and_then(Entity::as_fx).ok_or_else(|| {
                    format!(
                        "mobile {mobile_index} references missing or non-FX sprite entity {sprite_id}"
                    )
                })?;
                if fx.fx.mobile_index != Some(mobile_index_u16) {
                    return Err(format!(
                        "mobile {mobile_index} child {sprite_id} has wrong master index {:?}",
                        fx.fx.mobile_index
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_pc_index_inner(&self) -> Result<(), String> {
        let entity_pc_count = self.entities.pcs().count();
        for (name, ids) in [
            ("pc_ids", self.pc_ids.as_slice()),
            (
                "original_pc_registry_ids",
                self.original_pc_registry_ids.as_slice(),
            ),
        ] {
            let mut seen = std::collections::HashSet::with_capacity(ids.len());
            for &id in ids {
                if !seen.insert(id) {
                    return Err(format!("{name} contains duplicate entity {id}"));
                }
                if !matches!(self.entities.get(id), Some(Entity::Pc(_))) {
                    return Err(format!("{name} references missing or non-PC entity {id}"));
                }
            }
            if name == "pc_ids" && ids.len() != entity_pc_count {
                return Err(format!(
                    "{name} contains {} entries but entity storage contains {entity_pc_count} PCs",
                    ids.len(),
                ));
            }
        }
        if let Some(id) = self
            .original_pc_registry_ids
            .iter()
            .find(|id| !self.pc_ids.contains(id))
        {
            return Err(format!(
                "original_pc_registry_ids contains {id}, which is absent from pc_ids"
            ));
        }
        Ok(())
    }

    fn validate_pathfinder_states_inner(&self, assets: &LevelAssets) -> Result<(), String> {
        if self.pathfinder.states.len() != assets.navigation.pathfinder_graph.states.len() {
            return Err(format!(
                "pathfinder state layer count {} does not match level graph layer count {}",
                self.pathfinder.states.len(),
                assets.navigation.pathfinder_graph.states.len(),
            ));
        }
        for (layer_idx, (runtime, level)) in self
            .pathfinder
            .states
            .iter()
            .zip(&assets.navigation.pathfinder_graph.states)
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
                assets.navigation.level_grid.lines.len(),
            ),
            (
                "sector",
                self.fast_grid.sector_active.len(),
                assets.navigation.level_grid.sectors.len(),
            ),
            (
                "mask",
                self.fast_grid.mask_active.len(),
                assets.navigation.level_grid.masks.len(),
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
    fn soldier_registry_preserves_publication_order_deactivation_and_restore() {
        // Full actor decoding needs more stack than the default test thread.
        // Keep the codec matrix on its own stack, like other world roundtrips.
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(check_soldier_registry_lifecycle_and_restore)
            .unwrap()
            .join()
            .unwrap();
    }

    fn check_soldier_registry_lifecycle_and_restore() {
        use crate::element::Camp;
        use crate::engine::test_support::actors::make_test_ai_soldier;

        let mut engine = crate::engine::EngineInner::new();
        let first = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let foreign = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        let second = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        assert_eq!(
            engine.world.soldier_registry.all(),
            &[first.index(), foreign.index(), second.index()]
        );
        assert_eq!(
            engine.world.soldier_registry.camp(Camp::Lacklandists),
            &[first.index(), second.index()]
        );

        engine
            .world
            .entities
            .get_mut(first)
            .unwrap()
            .element_data_mut()
            .active = false;
        engine.remove_entity(foreign);
        let third = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        assert_eq!(
            engine.world.soldier_registry.all(),
            &[first.index(), second.index(), third.index()]
        );
        assert!(
            engine
                .world
                .soldier_registry
                .camp(Camp::Royalists)
                .is_empty()
        );

        // Explicit load adoption can differ from entity-table order. Both
        // persistence paths must retain that order and inactive membership.
        let civilian =
            engine.add_test_entity(crate::engine::test_support::actors::make_test_civilian(
                crate::element::Posture::Upright,
            ));
        let npc_order = vec![first, second, third, civilian];
        assert_eq!(engine.world.npc_registry_ids, npc_order);
        engine
            .world
            .soldier_registry
            .rebuild_from_order(&engine.world.entities, [third, first, second]);
        let wire = serde_json::to_vec(&engine.world.persisted_clone()).unwrap();
        let saved: WorldState = serde_json::from_slice(&wire).unwrap();
        let restored = saved;
        let rollback: WorldState = bitcode::decode(&bitcode::encode(&engine.world)).unwrap();
        for world in [&restored, &rollback] {
            assert_eq!(world.npc_registry_ids, npc_order);
            assert_eq!(
                world.soldier_registry.all(),
                &[third.index(), first.index(), second.index()]
            );
            assert_eq!(
                world.soldier_registry.camp(Camp::Lacklandists),
                world.soldier_registry.all()
            );
            assert!(!world.entities.get(first).unwrap().element_data().active);
            assert_eq!(
                robin_util::state_hash::compute(&world.soldier_registry),
                robin_util::state_hash::compute(&engine.world.soldier_registry)
            );
        }

        engine.remove_entity(first);
        assert_eq!(engine.world.npc_registry_ids, vec![second, third, civilian]);
        assert_eq!(restored.npc_registry_ids, npc_order);
        assert_eq!(
            engine.world.soldier_registry.all(),
            &[third.index(), second.index()]
        );
        assert_eq!(
            restored.soldier_registry.all(),
            &[third.index(), first.index(), second.index()]
        );
    }

    fn registry_test_soldier() -> Entity {
        Entity::Soldier(crate::element::ActorSoldier {
            element: Default::default(),
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: crate::element::SoldierData {
                cached_camp: crate::element::Camp::Lacklandists,
                ..Default::default()
            },
        })
    }

    #[test]
    fn fighter_registry_preserves_creation_and_tie_order_across_edits() {
        let mut engine = crate::engine::EngineInner::new();
        let mut ids = Vec::new();
        for _ in 0..4 {
            ids.push(engine.add_test_entity(registry_test_soldier()));
        }
        engine.world.install_original_creation_orders(
            ids.iter().copied().zip([50, 20, 20, 40]).collect(),
            51,
        );
        engine.add_test_entity(Entity::Fx(crate::element::ElementFx {
            element: Default::default(),
            fx: Default::default(),
        }));
        assert_eq!(
            engine.world.fighter_registry_ids,
            [ids[1], ids[2], ids[3], ids[0]]
        );

        engine.remove_entity(ids[1]);
        let added = engine.add_test_entity(registry_test_soldier());
        assert_eq!(
            engine.world.fighter_registry_ids,
            [ids[2], ids[3], ids[0], added]
        );
    }

    #[test]
    #[should_panic(expected = "published actor lacks creation order")]
    fn fighter_registry_rejects_missing_creation_order() {
        let mut engine = crate::engine::EngineInner::new();
        engine.add_test_entity(registry_test_soldier());
        engine.add_test_entity(registry_test_soldier());
        engine
            .world
            .install_original_creation_orders(BTreeMap::new(), 0);
    }

    #[test]
    fn empty_world_accepts_empty_level_attachments() {
        // Successful validation is the assertion: this must not panic.
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
    fn retired_pc_may_be_absent_from_original_registry() {
        let mut engine = crate::engine::EngineInner::new();
        let id = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
            crate::element::Posture::Upright,
        ));
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.retire_replaced_pc(id);
        assert!(!engine.world.original_pc_registry_ids.contains(&id));
        assert!(engine.world.actor_registry_ids.contains(&id));
        assert!(engine.world.fighter_registry_ids.contains(&id));
        engine.world.validate_level_attachments(&assets, 0);
    }

    #[test]
    #[should_panic(
        expected = "script zone 0 references grid sector 0, but the level has 0 sectors"
    )]
    fn out_of_bounds_script_zone_index_fails_loudly() {
        let world = WorldState::new();
        let mut assets = LevelAssets::new();
        assets.scripts.zone_grid_indices = std::sync::Arc::new(vec![0]);
        world.validate_level_attachments(&assets, 1);
    }
}
