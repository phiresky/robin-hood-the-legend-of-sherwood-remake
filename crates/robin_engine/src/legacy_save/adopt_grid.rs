//! Validated adoption of Original Linux-v48 `RHFastFindGrid` runtime state.
//!
//! The initialized Rust mission remains the owner of immutable geometry and
//! attachment topology. This module first resolves every saved reference and
//! Original array slot into a mutation-only plan. Applying that plan to the
//! same candidate engine is then infallible.

use thiserror::Error;

use crate::{
    ai::{Position, RepulsivePoint},
    coordinates::MapVec,
    element::{Entity, EntityId},
    engine::{EngineInner, LegacyGridGateAsset, LevelAssets},
    fast_find_grid::LiftRuntimeState,
    gate::GateType,
    patch::OccupantId,
};

use super::{
    LegacySaveAbiProfile,
    adopt::{LegacyEntityFixups, LegacyPositionTopology, LegacySaveAdoptError},
    adopt_elements::{LegacyElementAdoptError, LegacyElementBaseAdoption},
    adopt_vm_arena::{LegacyVmArenaError, LegacyVmArenaOwner, LegacyVmArenaPlan},
    gate_topology::{LegacyGateOrderError, derive_legacy_gate_order},
    post_grid::{
        LegacyFastFindGridState, LegacyGateState, LegacyPatchState, LegacySpecialSectorState,
    },
    topology_adapter::{LegacyTopologyAdapterError, derive_grid_topology},
};

#[derive(Debug, Error)]
pub enum LegacyGridAdoptError {
    #[error(transparent)]
    VmArena(#[from] LegacyVmArenaError),
    #[error("FastFindGrid adoption supports Linux i386 v48, not {profile:?}")]
    UnsupportedAbi { profile: LegacySaveAbiProfile },
    #[error(transparent)]
    Topology(#[from] LegacyTopologyAdapterError),
    #[error(transparent)]
    Reference(#[from] LegacySaveAdoptError),
    #[error(transparent)]
    Element(#[from] LegacyElementAdoptError),
    #[error("cannot map Original FastFindGrid gate identity: {0}")]
    GateOrder(#[from] LegacyGateOrderError),
    #[error(
        "saved FastFindGrid {field} count is {saved}, but initialized mission topology has {runtime}"
    )]
    CountMismatch {
        field: &'static str,
        saved: usize,
        runtime: usize,
    },
    #[error("saved FastFindGrid {field} at index {index} does not match initialized topology")]
    KindMismatch { field: &'static str, index: usize },
    #[error(
        "saved patch walk entry {walk_index} maps to missing initialized patch index {patch_index}"
    )]
    MissingPatch {
        walk_index: usize,
        patch_index: usize,
    },
    #[error("saved {field} occupant {creation_order} resolves to non-actor entity {entity_id}")]
    NonActorOccupant {
        field: &'static str,
        creation_order: u32,
        entity_id: EntityId,
    },
    #[error("saved {field} occupant list contains a null element pointer")]
    NullOccupant { field: &'static str },
    #[error("saved patch {patch_index} FX identity resolves to missing/non-FX entity {entity_id}")]
    MissingPatchFx {
        patch_index: usize,
        entity_id: EntityId,
    },
    #[error("saved patch {patch_index} FX points at patch {saved_patch:?}, expected {patch_index}")]
    PatchFxOwnerMismatch {
        patch_index: usize,
        saved_patch: Option<i16>,
    },
    #[error(
        "saved script sector {script_object_index} VM presence is {saved}, but initialized zone {zone_index} VM presence is {runtime}"
    )]
    ZoneVmPresenceMismatch {
        script_object_index: usize,
        zone_index: usize,
        saved: bool,
        runtime: bool,
    },
    #[error("saved static repulsive point {index} field {field} contains non-finite value {value}")]
    NonFiniteStaticRepulsivePoint {
        index: usize,
        field: &'static str,
        value: f32,
    },
    #[error("initialized grid runtime array {field} is shorter than required index {index}")]
    MissingRuntimeIndex { field: &'static str, index: usize },
    #[error("saved patch {patch_index} has invalid changing-obstacle topology: {detail}")]
    InvalidChangingObstacle { patch_index: usize, detail: String },
    #[error(
        "saved building sector {sector_index} maps to missing initialized building index {building_index}"
    )]
    MissingBuilding {
        sector_index: usize,
        building_index: usize,
    },
    #[error(
        "saved script object {script_object_index} maps to missing initialized script zone {zone_index}"
    )]
    MissingScriptZone {
        script_object_index: usize,
        zone_index: usize,
    },
}

#[derive(Clone, Debug)]
pub struct LegacyFastFindGridAdoptionPlan {
    patches: Vec<PlannedPatch>,
    doors: Vec<PlannedDoor>,
    script_zones: Vec<PlannedOccupants>,
    door_sectors: Vec<(usize, bool)>,
    buildings: Vec<PlannedBuilding>,
    lifts: Vec<(u32, LiftRuntimeState)>,
    static_repulsive_points: Vec<RepulsivePoint>,
    line_active: Vec<bool>,
    sector_active: Vec<bool>,
    mask_active: Vec<bool>,
    sight_obstacle_active: Vec<bool>,
    pathfinder: crate::pathfinder::PathFinder,
}

#[derive(Clone, Debug)]
struct PlannedPatch {
    patch_index: usize,
    active: bool,
    applied: bool,
    in_transition: bool,
    locked: bool,
    display_doors: bool,
    occupants: Vec<OccupantId>,
    fx: Option<PlannedPatchFx>,
}

#[derive(Clone, Debug)]
struct PlannedPatchFx {
    entity_id: EntityId,
    element: LegacyElementBaseAdoption,
    force_display: bool,
    restore_background: bool,
}

/// Presentation state emitted by grid adoption and installed only after the
/// candidate simulation has been atomically accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LegacyGridHostState {
    display_doors: Vec<(usize, bool)>,
}

impl LegacyGridHostState {
    pub(crate) fn apply(self, engine: &mut EngineInner) {
        for (patch_index, display_doors) in self.display_doors {
            engine.script_domains.interactables.patches[patch_index].display_doors = display_doors;
        }
    }
}

#[derive(Clone, Debug)]
struct PlannedDoor {
    door_index: usize,
    locked_pc: bool,
    locked_npc_villain: bool,
    locked_npc_civilian: bool,
    unlockable: bool,
    special_authorisation_pc: bool,
    authorised_pc_direct: u16,
    authorised_pc_indirect: u16,
}

#[derive(Clone, Debug)]
struct PlannedOccupants {
    index: usize,
    occupants: Vec<EntityId>,
    vm_heap: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct PlannedBuilding {
    building_index: usize,
    occupants: Vec<EntityId>,
    arrow_reserve: bool,
}

impl LegacyFastFindGridAdoptionPlan {
    /// Validate all topology, references, indices, and unsupported lossless
    /// representation boundaries before the candidate engine is mutated.
    pub fn preflight(
        engine: &EngineInner,
        assets: &LevelAssets,
        state: &LegacyFastFindGridState,
        entities: &LegacyEntityFixups,
        position_topology: &LegacyPositionTopology,
        vm_arena: &LegacyVmArenaPlan,
    ) -> Result<Self, LegacyGridAdoptError> {
        let static_repulsive_points = preflight_static_repulsive_points(state)?;

        let decoded_topology = derive_grid_topology(engine, assets)?;
        let retained = assets
            .legacy_grid_topology
            .as_ref()
            .expect("derive_grid_topology accepted missing retained topology");
        validate_state_shape(state, &decoded_topology)?;

        let mut patches = Vec::with_capacity(state.patches.len());
        for (walk_index, saved) in state.patches.iter().enumerate() {
            let retained_patch = &retained.patches[walk_index];
            let patch_index = retained_patch.patch_index as usize;
            let runtime = engine
                .script_domains
                .interactables
                .patches
                .get(patch_index)
                .ok_or(LegacyGridAdoptError::MissingPatch {
                    walk_index,
                    patch_index,
                })?;
            preflight_patch_runtime_indices(engine, runtime)?;
            let occupants = resolve_patch_occupants(engine, entities, saved)?;
            let fx = preflight_patch_fx(engine, entities, position_topology, patch_index, saved)?;
            patches.push(PlannedPatch {
                patch_index,
                active: saved.active_now,
                applied: saved.applied_now,
                in_transition: saved.in_transition_now,
                locked: saved.locked,
                display_doors: saved.display_doors,
                occupants,
                fx,
            });
        }

        let gate_order =
            derive_legacy_gate_order(&retained.gates, &engine.script_domains.interactables.doors)?;
        let mut doors = Vec::new();
        for (gate_index, (saved, retained_gate)) in
            state.gates.iter().zip(&retained.gates).enumerate()
        {
            match (saved, retained_gate) {
                (LegacyGateState::Stateless, LegacyGridGateAsset::Stateless) => {}
                (LegacyGateState::Door(saved), LegacyGridGateAsset::Door) => {
                    let door_index = usize::from(gate_order[gate_index]);
                    let runtime = engine
                        .script_domains
                        .interactables
                        .doors
                        .get(door_index)
                        .ok_or(LegacyGridAdoptError::MissingRuntimeIndex {
                            field: "doors",
                            index: door_index,
                        })?;
                    if runtime.gate_type != GateType::Door {
                        return Err(LegacyGridAdoptError::KindMismatch {
                            field: "gates",
                            index: door_index,
                        });
                    }
                    doors.push(PlannedDoor {
                        door_index,
                        locked_pc: saved.locked_pc,
                        locked_npc_villain: saved.locked_npc_villain,
                        locked_npc_civilian: saved.locked_npc_civilian,
                        unlockable: saved.unlockable,
                        special_authorisation_pc: saved.special_authorisation_pc,
                        authorised_pc_direct: saved.authorised_pc_direct,
                        authorised_pc_indirect: saved.authorised_pc_indirect,
                    });
                }
                _ => {
                    return Err(LegacyGridAdoptError::KindMismatch {
                        field: "gates",
                        index: gate_index,
                    });
                }
            }
        }

        check_count(
            "script zones",
            state.script_sectors.len(),
            engine.script_domains.zones.scripts.len(),
        )?;
        let mut script_zones = Vec::with_capacity(state.script_sectors.len());
        for (zone_index, saved) in state.script_sectors.iter().enumerate() {
            let runtime_vm = engine
                .scripts
                .mission
                .as_ref()
                .and_then(|mission| mission.zone_vm_class_and_heap(zone_index));
            if saved.script_members.is_some() != runtime_vm.is_some() {
                return Err(LegacyGridAdoptError::ZoneVmPresenceMismatch {
                    script_object_index: saved.script_object_index,
                    zone_index,
                    saved: saved.script_members.is_some(),
                    runtime: runtime_vm.is_some(),
                });
            }
            let vm_heap = saved
                .script_members
                .as_ref()
                .zip(runtime_vm)
                .map(|(members, (class, heap))| {
                    vm_arena.preflight_heap(
                        engine,
                        assets,
                        entities,
                        LegacyVmArenaOwner::ScriptZone(zone_index),
                        members,
                        class,
                        heap,
                    )
                })
                .transpose()?;
            if engine
                .script_domains
                .zones
                .scripts
                .get(zone_index)
                .is_none()
            {
                return Err(LegacyGridAdoptError::MissingScriptZone {
                    script_object_index: saved.script_object_index,
                    zone_index,
                });
            }
            script_zones.push(PlannedOccupants {
                index: zone_index,
                occupants: resolve_actor_occupants(
                    engine,
                    entities,
                    "script sector",
                    &saved.occupants,
                )?,
                vm_heap,
            });
        }

        let runtime_special = runtime_special_sector_indices(engine);
        check_count(
            "door sectors",
            state
                .special_sectors
                .iter()
                .filter(|sector| matches!(sector, LegacySpecialSectorState::Door { .. }))
                .count(),
            runtime_special.doors.len(),
        )?;
        check_count(
            "building sectors",
            state
                .special_sectors
                .iter()
                .filter(|sector| matches!(sector, LegacySpecialSectorState::Building { .. }))
                .count(),
            runtime_special.buildings.len(),
        )?;
        check_count(
            "lift sectors",
            state
                .special_sectors
                .iter()
                .filter(|sector| matches!(sector, LegacySpecialSectorState::Lift { .. }))
                .count(),
            runtime_special.lifts.len(),
        )?;
        let mut door_sectors = Vec::new();
        let mut buildings = Vec::new();
        let mut lifts = Vec::new();
        let mut door_ordinal = 0;
        let mut building_ordinal = 0;
        let mut lift_ordinal = 0;
        for saved in &state.special_sectors {
            match saved {
                LegacySpecialSectorState::Door { active, .. } => {
                    let runtime_index = runtime_special.doors[door_ordinal];
                    door_ordinal += 1;
                    if runtime_index >= engine.world.fast_grid.sector_active.len() {
                        return Err(LegacyGridAdoptError::MissingRuntimeIndex {
                            field: "door sector active",
                            index: runtime_index,
                        });
                    }
                    door_sectors.push((runtime_index, *active));
                }
                LegacySpecialSectorState::Building {
                    sector_index,
                    occupants,
                    arrow_reserve,
                } => {
                    let runtime_index = runtime_special.buildings[building_ordinal];
                    building_ordinal += 1;
                    let building_index = engine.world.fast_grid.level.sectors[runtime_index]
                        .building_index
                        .map(usize::from)
                        .ok_or(LegacyGridAdoptError::KindMismatch {
                            field: "building sectors",
                            index: *sector_index,
                        })?;
                    if engine
                        .script_domains
                        .buildings
                        .occupants
                        .get(building_index)
                        .is_none()
                        || engine
                            .script_domains
                            .buildings
                            .arrow_reserves
                            .get(building_index)
                            .is_none()
                    {
                        return Err(LegacyGridAdoptError::MissingBuilding {
                            sector_index: *sector_index,
                            building_index,
                        });
                    }
                    let occupants =
                        resolve_actor_occupants(engine, entities, "building sector", occupants)?;
                    buildings.push(PlannedBuilding {
                        building_index,
                        occupants,
                        arrow_reserve: *arrow_reserve,
                    });
                }
                LegacySpecialSectorState::Lift {
                    sector_index: _,
                    occupants_pc,
                    occupants,
                    occupied_upwards,
                    occupied_downwards,
                    wait_time,
                } => {
                    let runtime_index = runtime_special.lifts[lift_ordinal];
                    lift_ordinal += 1;
                    let runtime_index = u32::try_from(runtime_index).map_err(|_| {
                        LegacyGridAdoptError::MissingRuntimeIndex {
                            field: "lift sector exceeds u32",
                            index: runtime_index,
                        }
                    })?;
                    lifts.push((
                        runtime_index,
                        LiftRuntimeState {
                            occupants_pc: *occupants_pc,
                            occupants: *occupants,
                            occupied_upwards: *occupied_upwards,
                            occupied_downwards: *occupied_downwards,
                            wait_time: *wait_time,
                        },
                    ));
                }
            }
        }

        let mut runtime_grid = (*engine.world.fast_grid).clone();
        let mut pathfinder = crate::pathfinder::PathFinder::new();
        pathfinder.initialize_from_graph(assets.pathfinder_graph.as_ref(), &mut runtime_grid);
        let mut sight_obstacle_active = engine.world.static_sight_obstacle_active.clone();
        for planned in &patches {
            let patch = &engine.script_domains.interactables.patches[planned.patch_index];
            let applied = planned.applied;
            for &index in &patch.old_sector_indices {
                runtime_grid.sector_active[index as usize] = !applied;
            }
            for &index in &patch.new_sector_indices {
                runtime_grid.sector_active[index as usize] = applied;
            }
            for &index in &patch.old_line_indices {
                runtime_grid.line_active[usize::from(index)] = !applied;
            }
            for &index in &patch.new_line_indices {
                runtime_grid.line_active[usize::from(index)] = applied;
            }
            for &index in &patch.old_mask_indices {
                runtime_grid.mask_active[usize::from(index)] = !applied;
            }
            for &index in &patch.new_mask_indices {
                runtime_grid.mask_active[usize::from(index)] = applied;
            }
            for &index in &patch.old_sight_obstacle_indices {
                sight_obstacle_active[usize::from(index)] = !applied;
            }
            for &index in &patch.new_sight_obstacle_indices {
                sight_obstacle_active[usize::from(index)] = applied;
            }
            if applied && patch.use_changing_obstacles {
                let area = pathfinder
                    .try_convert_sector(assets.pathfinder_graph.as_ref(), patch.pathfinder_sector)
                    .ok_or_else(|| LegacyGridAdoptError::InvalidChangingObstacle {
                        patch_index: planned.patch_index,
                        detail: format!(
                            "pathfinder sector {} has no graph area",
                            patch.pathfinder_sector
                        ),
                    })?;
                let changing_obstacle = u16::try_from(patch.pathfinder_changing_obstacles)
                    .map_err(|_| LegacyGridAdoptError::InvalidChangingObstacle {
                        patch_index: planned.patch_index,
                        detail: format!(
                            "changing obstacle {} exceeds u16",
                            patch.pathfinder_changing_obstacles
                        ),
                    })?;
                // RHPathFinder stores two state bits per changing obstacle in
                // a u32. Validate before the runtime's shift operation.
                if changing_obstacle >= 16 {
                    return Err(LegacyGridAdoptError::InvalidChangingObstacle {
                        patch_index: planned.patch_index,
                        detail: format!(
                            "changing obstacle {changing_obstacle} exceeds the 16 two-bit u32 slots"
                        ),
                    });
                }
                let layer = usize::from(patch.pathfinder_layer);
                if layer >= pathfinder.states.len()
                    || usize::from(area) >= pathfinder.states[layer].len()
                {
                    return Err(LegacyGridAdoptError::InvalidChangingObstacle {
                        patch_index: planned.patch_index,
                        detail: format!(
                            "layer {layer}, area {area} is outside pathfinder state topology"
                        ),
                    });
                }
                let mut appeared = Vec::new();
                let mut line_toggles = Vec::new();
                pathfinder.toggle_obstacle_state(
                    assets.pathfinder_graph.as_ref(),
                    layer,
                    usize::from(area),
                    changing_obstacle,
                    &mut appeared,
                    &mut line_toggles,
                );
                for (line_index, active) in line_toggles {
                    let index = usize::from(line_index);
                    if index >= runtime_grid.line_active.len() {
                        return Err(LegacyGridAdoptError::MissingRuntimeIndex {
                            field: "pathfinder obstacle line",
                            index,
                        });
                    }
                    runtime_grid.line_active[index] = active;
                }
            }
        }

        Ok(Self {
            patches,
            doors,
            script_zones,
            door_sectors,
            buildings,
            lifts,
            static_repulsive_points,
            line_active: runtime_grid.line_active,
            sector_active: runtime_grid.sector_active,
            mask_active: runtime_grid.mask_active,
            sight_obstacle_active,
            pathfinder,
        })
    }

    /// Apply a fully validated plan to the same initialized candidate.
    ///
    /// Patch-dependent grid flags and door swap baselines are reconstructed
    /// before the independently serialized door fields overwrite current
    /// authorization, matching Original load order.
    pub(crate) fn apply(self, engine: &mut EngineInner) -> LegacyGridHostState {
        engine.world.fast_grid_mut().line_active = self.line_active;
        engine.world.fast_grid_mut().sector_active = self.sector_active;
        engine.world.fast_grid_mut().mask_active = self.mask_active;
        engine.world.static_sight_obstacle_active = self.sight_obstacle_active;
        engine.world.pathfinder = self.pathfinder;
        engine.ai.global.repulsive_points = self.static_repulsive_points;

        for planned in &self.patches {
            let patch = &engine.script_domains.interactables.patches[planned.patch_index];
            if planned.applied {
                for &door_index in &patch.door_indices {
                    engine.script_domains.interactables.doors[door_index as usize]
                        .swap_rights_patch();
                }
            }
        }

        let mut display_doors = Vec::with_capacity(self.patches.len());
        for planned in self.patches {
            display_doors.push((planned.patch_index, planned.display_doors));
            {
                let patch = &mut engine.script_domains.interactables.patches[planned.patch_index];
                patch.active = planned.active;
                patch.applied = planned.applied;
                patch.in_transition = planned.in_transition;
                patch.locked = planned.locked;
                patch.occupants = planned.occupants;
                // Host output is installed only after the candidate is
                // accepted; it never enters simulation snapshots or hashes.
                patch.display_doors = false;
            }
            if let Some(fx) = planned.fx {
                fx.element.apply(engine);
                let Entity::Fx(entity) = engine
                    .world
                    .entities
                    .get_mut(fx.entity_id)
                    .expect("preflighted patch FX disappeared")
                else {
                    unreachable!("preflighted patch FX changed class");
                };
                entity.fx.force_display = fx.force_display;
                entity.fx.restore_background = fx.restore_background;
            }
        }

        for planned in self.doors {
            let door = &mut engine.script_domains.interactables.doors[planned.door_index];
            door.locked_pc = planned.locked_pc;
            door.locked_npc_villain = planned.locked_npc_villain;
            door.locked_npc_civilian = planned.locked_npc_civilian;
            door.unlockable = planned.unlockable;
            door.special_authorisation_pc = planned.special_authorisation_pc;
            door.authorised_pc_direct = planned.authorised_pc_direct;
            door.authorised_pc_indirect = planned.authorised_pc_indirect;
        }
        // DoorSeekInfo is built from the proto door table before save
        // adoption. Its authorization bit includes the live active/lock
        // fields, so restoring those fields above must refresh the derived
        // cache as well. Original FindDoorEnemyCouldBeBehind reads the live
        // RHDoor on every query; retaining the proto-time bit could let a
        // soldier seek through a door that the loaded save has locked.
        for door_info in &mut engine.ai.global.door_seek_infos {
            let door = engine
                .script_domains
                .interactables
                .doors
                .get(usize::from(door_info.door_index))
                .unwrap_or_else(|| {
                    panic!(
                        "AI door-seek cache references missing canonical door {}",
                        door_info.door_index
                    )
                });
            door_info.npc_villain_authorized_direct =
                crate::ai::cache_npc_villain_authorized_direct(door);
        }
        for planned in self.script_zones {
            engine.script_domains.zones.scripts[planned.index].occupant_indices = planned.occupants;
            if let Some(heap) = planned.vm_heap {
                engine
                    .scripts
                    .mission
                    .as_mut()
                    .expect("preflighted script-zone VM mission disappeared")
                    .replace_zone_vm_heap(planned.index, heap);
            }
        }
        for (sector_index, active) in self.door_sectors {
            engine.world.fast_grid_mut().sector_active[sector_index] = active;
        }
        for planned in self.buildings {
            engine.script_domains.buildings.occupants[planned.building_index] = planned
                .occupants
                .iter()
                .copied()
                .map(crate::natives::ScriptHandleCodec::actor_handle)
                .collect();
            engine.script_domains.buildings.arrow_reserves[planned.building_index] =
                planned.arrow_reserve;
            // `RHSectorBuilding::Serialize` restores the authoritative
            // occupant list from the save. `AiGlobalState::houses` is the
            // Rust-side view used by EnemyInHouseAlert, so it must observe
            // the same restored list instead of retaining mission-start
            // occupants from `initialize_buildings`.
            if let Some(house) = engine.ai.global.houses.iter_mut().find(|house| {
                house.building_index
                    == crate::sector::BuildingIdx::new(planned.building_index as u16)
            }) {
                house.occupant_ids = planned.occupants;
                house.arrow_reserve = planned.arrow_reserve;
            }
        }
        for (sector_index, lift) in self.lifts {
            if lift.occupants_pc == 0
                && lift.occupants == 0
                && !lift.occupied_upwards
                && !lift.occupied_downwards
                && lift.wait_time == 0
            {
                engine
                    .world
                    .fast_grid_mut()
                    .lift_state
                    .remove(&sector_index);
            } else {
                engine
                    .world
                    .fast_grid_mut()
                    .lift_state
                    .insert(sector_index, lift);
            }
        }
        LegacyGridHostState { display_doors }
    }
}

fn validate_state_shape(
    state: &LegacyFastFindGridState,
    topology: &super::post_grid::LegacyGridTopology,
) -> Result<(), LegacyGridAdoptError> {
    check_count("patches", state.patches.len(), topology.patches.len())?;
    for (index, (saved, expected)) in state.patches.iter().zip(&topology.patches).enumerate() {
        if &saved.topology != expected {
            return Err(LegacyGridAdoptError::KindMismatch {
                field: "patch topology",
                index,
            });
        }
    }
    check_count("gates", state.gates.len(), topology.gates.len())?;
    check_count(
        "script sectors",
        state.script_sectors.len(),
        topology
            .script_objects
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    super::post_grid::LegacyScriptObjectTopology::Sector { .. }
                )
            })
            .count(),
    )?;
    let expected_script_indices =
        topology
            .script_objects
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                matches!(
                    entry,
                    super::post_grid::LegacyScriptObjectTopology::Sector { .. }
                )
                .then_some(index)
            });
    for (ordinal, (saved, expected_index)) in state
        .script_sectors
        .iter()
        .zip(expected_script_indices)
        .enumerate()
    {
        if saved.script_object_index != expected_index {
            return Err(LegacyGridAdoptError::KindMismatch {
                field: "script sector topology",
                index: ordinal,
            });
        }
    }
    let expected_special = topology
        .sectors
        .iter()
        .enumerate()
        .filter_map(|(index, kind)| {
            (!matches!(kind, super::post_grid::LegacySectorTopology::NullOrOrdinary))
                .then_some((index, kind))
        })
        .collect::<Vec<_>>();
    check_count(
        "special sectors",
        state.special_sectors.len(),
        expected_special.len(),
    )?;
    for (ordinal, (saved, (expected_index, expected_kind))) in state
        .special_sectors
        .iter()
        .zip(expected_special)
        .enumerate()
    {
        let matches = match (saved, expected_kind) {
            (
                LegacySpecialSectorState::Door { sector_index, .. },
                super::post_grid::LegacySectorTopology::Door,
            )
            | (
                LegacySpecialSectorState::Building { sector_index, .. },
                super::post_grid::LegacySectorTopology::Building,
            )
            | (
                LegacySpecialSectorState::Lift { sector_index, .. },
                super::post_grid::LegacySectorTopology::Lift,
            ) => *sector_index == expected_index,
            _ => false,
        };
        if !matches {
            return Err(LegacyGridAdoptError::KindMismatch {
                field: "special sector topology",
                index: ordinal,
            });
        }
    }
    Ok(())
}

fn check_count(
    field: &'static str,
    saved: usize,
    runtime: usize,
) -> Result<(), LegacyGridAdoptError> {
    if saved != runtime {
        return Err(LegacyGridAdoptError::CountMismatch {
            field,
            saved,
            runtime,
        });
    }
    Ok(())
}

fn resolve_patch_occupants(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    saved: &LegacyPatchState,
) -> Result<Vec<OccupantId>, LegacyGridAdoptError> {
    Ok(
        resolve_actor_occupants(engine, entities, "patch", &saved.occupants)?
            .into_iter()
            .map(|entity| OccupantId(entity.index()))
            .collect(),
    )
}

fn resolve_actor_occupants(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    field: &'static str,
    references: &[super::payload_base::LegacyElementRef],
) -> Result<Vec<EntityId>, LegacyGridAdoptError> {
    references
        .iter()
        .map(|&reference| {
            let creation_order = reference
                .0
                .ok_or(LegacyGridAdoptError::NullOccupant { field })?;
            let entity_id = entities
                .resolve_element(reference)?
                .expect("non-null reference resolved as null");
            let entity = engine.world.entities.get(entity_id).ok_or(
                LegacyGridAdoptError::NonActorOccupant {
                    field,
                    creation_order,
                    entity_id,
                },
            )?;
            if !entity.is_actor() {
                return Err(LegacyGridAdoptError::NonActorOccupant {
                    field,
                    creation_order,
                    entity_id,
                });
            }
            Ok(entity_id)
        })
        .collect()
}

fn preflight_patch_fx(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    position_topology: &LegacyPositionTopology,
    patch_index: usize,
    saved: &LegacyPatchState,
) -> Result<Option<PlannedPatchFx>, LegacyGridAdoptError> {
    let Some(fx) = &saved.fx else {
        return Ok(None);
    };
    // RHPatch::Serialize writes this common base after the ordinary element
    // stream. It therefore composes with, and overwrites, that earlier copy
    // exactly as it does during Original load.
    let element =
        LegacyElementBaseAdoption::preflight(engine, &fx.element, entities, position_topology)?;
    let entity_id = element.entity_id();
    if !matches!(engine.world.entities.get(entity_id), Some(Entity::Fx(_))) {
        return Err(LegacyGridAdoptError::MissingPatchFx {
            patch_index,
            entity_id,
        });
    }
    let expected_patch =
        i16::try_from(patch_index).map_err(|_| LegacyGridAdoptError::MissingRuntimeIndex {
            field: "patch FX owner exceeds i16",
            index: patch_index,
        })?;
    if fx.patch.0 != Some(expected_patch) {
        return Err(LegacyGridAdoptError::PatchFxOwnerMismatch {
            patch_index,
            saved_patch: fx.patch.0,
        });
    }
    Ok(Some(PlannedPatchFx {
        entity_id,
        element,
        force_display: fx.force_display,
        restore_background: fx.restore_background,
    }))
}

fn preflight_static_repulsive_points(
    state: &LegacyFastFindGridState,
) -> Result<Vec<RepulsivePoint>, LegacyGridAdoptError> {
    state
        .static_repulsive_points
        .iter()
        .enumerate()
        .map(|(index, saved)| {
            for (field, value) in [
                ("position.x", saved.point.position.x),
                ("position.y", saved.point.position.y),
                ("limit_left.x", saved.point.limit_left.x),
                ("limit_left.y", saved.point.limit_left.y),
                ("limit_right.x", saved.point.limit_right.x),
                ("limit_right.y", saved.point.limit_right.y),
                ("action_radius", saved.point.action_radius),
                ("force_a", saved.point.force_a),
                ("force_b", saved.point.force_b),
                ("radius", saved.point.radius),
            ] {
                if !value.is_finite() {
                    return Err(LegacyGridAdoptError::NonFiniteStaticRepulsivePoint {
                        index,
                        field,
                        value,
                    });
                }
            }
            let flags = i32::from(saved.point.affects_pcs)
                | (i32::from(saved.point.affects_soldiers) << 1)
                | (i32::from(saved.point.affects_civilians) << 2)
                | (i32::from(saved.point.affects_animals) << 3);
            Ok(RepulsivePoint {
                // Script handles are signed i32 but Original owns an ULONG;
                // this bit-preserving cast keeps all 2^32 identities.
                id: saved.point.id as i32,
                position: Position {
                    x: saved.point.position.x,
                    y: saved.point.position.y,
                    level: saved.layer,
                    sector: None,
                },
                radius: saved.point.radius,
                action_radius: saved.point.action_radius,
                force_a: saved.point.force_a,
                force_b: saved.point.force_b,
                concave: saved.point.concave,
                limit_left: MapVec::new(saved.point.limit_left.x, saved.point.limit_left.y),
                limit_right: MapVec::new(saved.point.limit_right.x, saved.point.limit_right.y),
                flags,
            })
        })
        .collect()
}

fn preflight_patch_runtime_indices(
    engine: &EngineInner,
    patch: &crate::patch::Patch,
) -> Result<(), LegacyGridAdoptError> {
    for (field, indices, count) in [
        (
            "patch.old_sector_indices",
            patch
                .old_sector_indices
                .iter()
                .map(|&value| value as usize)
                .collect::<Vec<_>>(),
            engine.world.fast_grid.sector_active.len(),
        ),
        (
            "patch.new_sector_indices",
            patch
                .new_sector_indices
                .iter()
                .map(|&value| value as usize)
                .collect(),
            engine.world.fast_grid.sector_active.len(),
        ),
        (
            "patch.old_line_indices",
            patch
                .old_line_indices
                .iter()
                .map(|&value| usize::from(value))
                .collect(),
            engine.world.fast_grid.line_active.len(),
        ),
        (
            "patch.new_line_indices",
            patch
                .new_line_indices
                .iter()
                .map(|&value| usize::from(value))
                .collect(),
            engine.world.fast_grid.line_active.len(),
        ),
        (
            "patch.old_mask_indices",
            patch
                .old_mask_indices
                .iter()
                .map(|&value| usize::from(value))
                .collect(),
            engine.world.fast_grid.mask_active.len(),
        ),
        (
            "patch.new_mask_indices",
            patch
                .new_mask_indices
                .iter()
                .map(|&value| usize::from(value))
                .collect(),
            engine.world.fast_grid.mask_active.len(),
        ),
        (
            "patch.old_sight_obstacle_indices",
            patch
                .old_sight_obstacle_indices
                .iter()
                .map(|&value| usize::from(value))
                .collect(),
            engine.world.static_sight_obstacle_active.len(),
        ),
        (
            "patch.new_sight_obstacle_indices",
            patch
                .new_sight_obstacle_indices
                .iter()
                .map(|&value| usize::from(value))
                .collect(),
            engine.world.static_sight_obstacle_active.len(),
        ),
    ] {
        if let Some(index) = indices.into_iter().find(|&index| index >= count) {
            return Err(LegacyGridAdoptError::MissingRuntimeIndex { field, index });
        }
    }
    for &index in &patch.door_indices {
        if index as usize >= engine.script_domains.interactables.doors.len() {
            return Err(LegacyGridAdoptError::MissingRuntimeIndex {
                field: "patch.door_indices",
                index: index as usize,
            });
        }
    }
    Ok(())
}

struct RuntimeSpecialSectorIndices {
    doors: Vec<usize>,
    buildings: Vec<usize>,
    lifts: Vec<usize>,
}

fn runtime_special_sector_indices(engine: &EngineInner) -> RuntimeSpecialSectorIndices {
    let mut result = RuntimeSpecialSectorIndices {
        doors: Vec::new(),
        buildings: Vec::new(),
        lifts: Vec::new(),
    };
    for (index, sector) in engine.world.fast_grid.level.sectors.iter().enumerate() {
        if sector.sector_type.is_door() {
            result.doors.push(index);
        } else if sector.sector_type.is_building() {
            result.buildings.push(index);
        } else if sector.sector_type.is_lift() {
            result.lifts.push(index);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        engine::LevelAssets,
        legacy_save::{
            payload_base::{LegacyElementRef, LegacyPoint2},
            post_grid::{LegacyGridTopology, LegacyLayeredRepulsivePoint, LegacyRepulsivePoint},
        },
        patch::Patch,
        sector::ScriptSectorData,
    };

    fn empty_state() -> LegacyFastFindGridState {
        LegacyFastFindGridState {
            start_offset: 0,
            abi_profile: LegacySaveAbiProfile::PortLinuxI386V48,
            patches: Vec::new(),
            gates: Vec::new(),
            script_sectors: Vec::new(),
            special_sectors: Vec::new(),
            static_repulsive_points: Vec::new(),
            end_offset: 0,
        }
    }

    fn empty_fixups() -> LegacyEntityFixups {
        LegacyEntityFixups {
            by_creation_order: BTreeMap::new(),
            by_saved_slot: Vec::new(),
            creation_order_by_entity: BTreeMap::new(),
            mobile_by_creation_order: BTreeMap::new(),
            mobile_owner_by_creation_order: BTreeMap::new(),
        }
    }

    fn empty_position_topology() -> LegacyPositionTopology {
        LegacyPositionTopology {
            sectors: Vec::new(),
            sector_doors: Vec::new(),
            doors: Vec::new(),
            projection_areas: Vec::new(),
            sight_obstacles: Vec::new(),
        }
    }

    #[test]
    fn windows_profile_reaches_normal_topology_validation() {
        let mut state = empty_state();
        state.abi_profile = LegacySaveAbiProfile::RetailWindowsX86V48;
        let error = LegacyFastFindGridAdoptionPlan::preflight(
            &EngineInner::new(),
            &LevelAssets::new(),
            &state,
            &empty_fixups(),
            &empty_position_topology(),
            &LegacyVmArenaPlan::empty_for_tests(),
        )
        .unwrap_err();
        assert!(!matches!(
            error,
            LegacyGridAdoptError::UnsupportedAbi { .. }
        ));
    }

    #[test]
    fn preserves_all_static_repulsive_point_state() {
        let mut state = empty_state();
        state
            .static_repulsive_points
            .push(LegacyLayeredRepulsivePoint {
                point: LegacyRepulsivePoint {
                    position: LegacyPoint2 { x: 1.0, y: 2.0 },
                    concave: true,
                    limit_left: LegacyPoint2 { x: -8.0, y: 9.0 },
                    limit_right: LegacyPoint2 { x: 10.0, y: -11.0 },
                    action_radius: 3.0,
                    force_a: 4.0,
                    force_b: 5.0,
                    radius: 6.0,
                    id: u32::MAX,
                    affects_pcs: true,
                    affects_soldiers: true,
                    affects_civilians: true,
                    affects_animals: true,
                },
                layer: 0,
            });
        let points = preflight_static_repulsive_points(&state).unwrap();
        assert_eq!(points.len(), 1);
        let point = &points[0];
        assert_eq!(point.id, -1);
        assert_eq!((point.position.x, point.position.y), (1.0, 2.0));
        assert_eq!(point.position.level, 0);
        assert_eq!(point.action_radius, 3.0);
        assert_eq!(point.force_a, 4.0);
        assert_eq!(point.force_b, 5.0);
        assert_eq!(point.radius, 6.0);
        assert!(point.concave);
        assert_eq!(point.limit_left, MapVec::new(-8.0, 9.0));
        assert_eq!(point.limit_right, MapVec::new(10.0, -11.0));
        assert_eq!(point.flags, 0xf);
    }

    #[test]
    fn rejects_non_finite_static_repulsive_state_before_apply() {
        let mut state = empty_state();
        state
            .static_repulsive_points
            .push(LegacyLayeredRepulsivePoint {
                point: LegacyRepulsivePoint {
                    position: LegacyPoint2 {
                        x: f32::NAN,
                        y: 0.0,
                    },
                    concave: false,
                    limit_left: LegacyPoint2 { x: 0.0, y: 0.0 },
                    limit_right: LegacyPoint2 { x: 0.0, y: 0.0 },
                    action_radius: 1.0,
                    force_a: 1.0,
                    force_b: 0.0,
                    radius: 0.0,
                    id: 1,
                    affects_pcs: false,
                    affects_soldiers: false,
                    affects_civilians: false,
                    affects_animals: false,
                },
                layer: 0,
            });

        assert!(matches!(
            preflight_static_repulsive_points(&state),
            Err(LegacyGridAdoptError::NonFiniteStaticRepulsivePoint {
                index: 0,
                field: "position.x",
                ..
            })
        ));
    }

    #[test]
    fn shape_validation_rejects_sparse_special_sector_identity_mismatch() {
        let mut state = empty_state();
        state.special_sectors.push(LegacySpecialSectorState::Door {
            sector_index: 4,
            active: true,
        });
        let topology = LegacyGridTopology {
            patches: Vec::new(),
            gates: Vec::new(),
            script_objects: Vec::new(),
            sectors: vec![
                super::super::post_grid::LegacySectorTopology::NullOrOrdinary,
                super::super::post_grid::LegacySectorTopology::Door,
            ],
        };
        assert!(matches!(
            validate_state_shape(&state, &topology),
            Err(LegacyGridAdoptError::KindMismatch {
                field: "special sector topology",
                ..
            })
        ));
    }

    #[test]
    fn apply_installs_preflighted_patch_door_zone_building_lift_state() {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().sector_active = vec![true, true, true];
        engine.world.fast_grid_mut().line_active = vec![true, true];
        engine.world.fast_grid_mut().mask_active = vec![true, true];
        engine.world.static_sight_obstacle_active = vec![true, true];

        let mut patch = Patch {
            active: true,
            initially_active: true,
            old_sector_indices: vec![0],
            new_sector_indices: vec![1],
            old_line_indices: vec![crate::fast_find_grid::LineIndex::new(0).unwrap()],
            new_line_indices: vec![crate::fast_find_grid::LineIndex::new(1).unwrap()],
            old_mask_indices: vec![crate::mask::MaskIndex::new(0).unwrap()],
            new_mask_indices: vec![crate::mask::MaskIndex::new(1).unwrap()],
            old_sight_obstacle_indices: vec![
                crate::sight_obstacle::SightObstacleIndex::new(0).unwrap(),
            ],
            new_sight_obstacle_indices: vec![
                crate::sight_obstacle::SightObstacleIndex::new(1).unwrap(),
            ],
            door_indices: vec![0],
            ..Patch::default()
        };
        patch.locked = false;
        engine.script_domains.interactables.patches.push(patch);
        let mut door = crate::gate::Door {
            gate_type: GateType::Door,
            active: true,
            door_type: crate::gate::DoorType::Building,
            locked_pc: false,
            locked_pc_after_patch: true,
            ..crate::gate::Door::default()
        };
        door.locked_npc_villain_after_patch = true;
        engine.script_domains.interactables.doors.push(door);
        engine
            .ai
            .global
            .door_seek_infos
            .push(crate::ai::DoorSeekInfo {
                door_index: crate::gate::DoorIndex(0),
                door_type: crate::gate::DoorType::Building,
                point_out: crate::coordinates::MapPoint::ZERO,
                position_in: Position::default(),
                sector_out: 0,
                sector_in: 0,
                layer_out: 0,
                // Proto-time state was unlocked. The serialized state below
                // locks this door for NPC villains.
                npc_villain_authorized_direct: true,
            });
        engine
            .script_domains
            .zones
            .scripts
            .push(ScriptSectorData::default());
        engine.script_domains.buildings.occupants.push(Vec::new());
        engine.script_domains.buildings.arrow_reserves.push(false);
        engine.ai.global.houses.push(crate::ai::House {
            building_index: crate::sector::BuildingIdx::new(0),
            ..crate::ai::House::default()
        });

        let plan = LegacyFastFindGridAdoptionPlan {
            patches: vec![PlannedPatch {
                patch_index: 0,
                active: false,
                applied: true,
                in_transition: false,
                locked: true,
                display_doors: true,
                occupants: vec![OccupantId(9)],
                fx: None,
            }],
            doors: vec![PlannedDoor {
                door_index: 0,
                locked_pc: false,
                locked_npc_villain: true,
                locked_npc_civilian: true,
                unlockable: true,
                special_authorisation_pc: true,
                authorised_pc_direct: 3,
                authorised_pc_indirect: 4,
            }],
            script_zones: vec![PlannedOccupants {
                index: 0,
                occupants: Vec::new(),
                vm_heap: None,
            }],
            door_sectors: vec![(2, false)],
            buildings: vec![PlannedBuilding {
                building_index: 0,
                occupants: vec![EntityId::Soldier(crate::entity_id::SoldierId(9))],
                arrow_reserve: true,
            }],
            lifts: vec![(
                2,
                LiftRuntimeState {
                    occupants_pc: 0,
                    occupants: 2,
                    occupied_upwards: true,
                    occupied_downwards: false,
                    wait_time: 17,
                },
            )],
            static_repulsive_points: vec![RepulsivePoint::new(
                11,
                Position::default(),
                2.0,
                3.0,
                5,
            )],
            line_active: vec![false, true],
            sector_active: vec![false, true, true],
            mask_active: vec![false, true],
            sight_obstacle_active: vec![false, true],
            pathfinder: crate::pathfinder::PathFinder::new(),
        };
        let host = plan.apply(&mut engine);

        let patch = &engine.script_domains.interactables.patches[0];
        assert!(!patch.active);
        assert!(patch.applied);
        assert!(patch.locked);
        assert_eq!(patch.occupants, [OccupantId(9)]);
        assert!(!patch.display_doors);
        assert_eq!(engine.world.fast_grid.sector_active, [false, true, false]);
        assert_eq!(engine.world.fast_grid.line_active, [false, true]);
        assert_eq!(engine.world.fast_grid.mask_active, [false, true]);
        assert_eq!(engine.world.static_sight_obstacle_active, [false, true]);

        let door = &engine.script_domains.interactables.doors[0];
        assert!(!door.locked_pc);
        assert!(door.locked_npc_villain);
        assert!(door.locked_npc_civilian);
        assert!(door.unlockable);
        assert!(door.special_authorisation_pc);
        assert_eq!(door.authorised_pc_direct, 3);
        assert_eq!(door.authorised_pc_indirect, 4);
        assert!(
            !engine.ai.global.door_seek_infos[0].npc_villain_authorized_direct,
            "save adoption must refresh proto-time door authorization caches"
        );
        // Patch application swapped the retained future half before the
        // independently serialized current half was restored.
        assert!(!door.locked_pc_after_patch);

        assert_eq!(engine.script_domains.buildings.occupants[0], [0x1000_0009]);
        assert!(engine.script_domains.buildings.arrow_reserves[0]);
        assert_eq!(
            engine.ai.global.houses[0].occupant_ids,
            [EntityId::Soldier(crate::entity_id::SoldierId(9))]
        );
        assert!(engine.ai.global.houses[0].arrow_reserve);
        let lift = engine.world.fast_grid.lift_state.get(&2).unwrap();
        assert_eq!(lift.occupants_pc, 0);
        assert_eq!(lift.occupants, 2);
        assert!(lift.occupied_upwards);
        assert_eq!(lift.wait_time, 17);
        assert_eq!(engine.ai.global.repulsive_points.len(), 1);
        assert_eq!(engine.ai.global.repulsive_points[0].id, 11);

        host.apply(&mut engine);
        assert!(engine.script_domains.interactables.patches[0].display_doors);
    }

    #[test]
    fn null_occupant_reference_is_never_accepted_as_fake_data() {
        let engine = EngineInner::new();
        assert!(matches!(
            resolve_actor_occupants(&engine, &empty_fixups(), "patch", &[LegacyElementRef(None)],),
            Err(LegacyGridAdoptError::NullOccupant { field: "patch" })
        ));
    }
}
