//! Validated adoption of Original Linux-v48 `RHFastFindGrid` runtime state.
//!
//! The initialized Rust mission remains the owner of immutable geometry and
//! attachment topology. This module first resolves every saved reference and
//! Original array slot into a mutation-only plan. Applying that plan to the
//! same candidate engine is then infallible.

use thiserror::Error;

use crate::{
    element::{Entity, EntityId},
    engine::{EngineInner, LegacyGridGateAsset, LevelAssets},
    fast_find_grid::LiftRuntimeState,
    gate::GateType,
    patch::OccupantId,
};

use super::{
    LegacySaveAbiProfile,
    adopt::{LegacyEntityFixups, LegacySaveAdoptError},
    gate_topology::{LegacyGateOrderError, derive_legacy_gate_order},
    post_grid::{
        LegacyFastFindGridState, LegacyGateState, LegacyPatchState, LegacySpecialSectorState,
    },
    topology_adapter::{LegacyTopologyAdapterError, derive_grid_topology},
};

#[derive(Debug, Error)]
pub enum LegacyGridAdoptError {
    #[error("FastFindGrid adoption supports Linux i386 v48, not {profile:?}")]
    UnsupportedAbi { profile: LegacySaveAbiProfile },
    #[error(transparent)]
    Topology(#[from] LegacyTopologyAdapterError),
    #[error(transparent)]
    Reference(#[from] LegacySaveAdoptError),
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
        "saved script sector {script_object_index} has authoritative VM members; zone VM heap adoption is not implemented"
    )]
    UnsupportedZoneVm { script_object_index: usize },
    #[error(
        "saved lift sector {sector_index} has {occupants_pc} PC occupants; Rust LiftRuntimeState has no lossless PC-occupancy field"
    )]
    UnsupportedLiftPcOccupancy {
        sector_index: usize,
        occupants_pc: u16,
    },
    #[error(
        "save contains {count} static repulsive points; Rust's script-point model drops serialized wedge/force coefficients"
    )]
    UnsupportedStaticRepulsivePoints { count: usize },
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
    occupants: Vec<OccupantId>,
    fx: Option<PlannedPatchFx>,
}

#[derive(Clone, Debug)]
struct PlannedPatchFx {
    entity_id: EntityId,
    force_display: bool,
    restore_background: bool,
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
}

#[derive(Clone, Debug)]
struct PlannedBuilding {
    building_index: usize,
    occupants: Vec<i32>,
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
    ) -> Result<Self, LegacyGridAdoptError> {
        if state.abi_profile != LegacySaveAbiProfile::PortLinuxI386V48 {
            return Err(LegacyGridAdoptError::UnsupportedAbi {
                profile: state.abi_profile,
            });
        }
        if !state.static_repulsive_points.is_empty() {
            // TODO(save-import): preserve all serialized RHRepulsivePoint
            // coefficients in the Rust runtime before accepting these.
            return Err(LegacyGridAdoptError::UnsupportedStaticRepulsivePoints {
                count: state.static_repulsive_points.len(),
            });
        }

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
            let fx = preflight_patch_fx(engine, entities, patch_index, saved)?;
            patches.push(PlannedPatch {
                patch_index,
                active: saved.active_now,
                applied: saved.applied_now,
                in_transition: saved.in_transition_now,
                locked: saved.locked,
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
            if saved
                .script_members
                .as_ref()
                .is_some_and(|members| !members.members.is_empty())
            {
                // TODO(save-import): adopt the zone-owned VM heap as part of
                // the script-runtime plan, then pass its validated plan here.
                return Err(LegacyGridAdoptError::UnsupportedZoneVm {
                    script_object_index: saved.script_object_index,
                });
            }
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
                        resolve_actor_occupants(engine, entities, "building sector", occupants)?
                            .into_iter()
                            .map(crate::natives::ScriptHandleCodec::actor_handle)
                            .collect();
                    buildings.push(PlannedBuilding {
                        building_index,
                        occupants,
                        arrow_reserve: *arrow_reserve,
                    });
                }
                LegacySpecialSectorState::Lift {
                    sector_index,
                    occupants_pc,
                    occupants,
                    occupied_upwards,
                    occupied_downwards,
                    wait_time,
                } => {
                    if *occupants_pc != 0 {
                        // TODO(save-import): add the Original's separate PC
                        // occupancy counter to LiftRuntimeState.
                        return Err(LegacyGridAdoptError::UnsupportedLiftPcOccupancy {
                            sector_index: *sector_index,
                            occupants_pc: *occupants_pc,
                        });
                    }
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
                            occupants: *occupants,
                            occupied_upwards: *occupied_upwards,
                            occupied_downwards: *occupied_downwards,
                            wait_time: *wait_time,
                        },
                    ));
                }
            }
        }

        let mut runtime_grid = engine.world.fast_grid.clone();
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
    pub fn apply(self, engine: &mut EngineInner) {
        engine.world.fast_grid.line_active = self.line_active;
        engine.world.fast_grid.sector_active = self.sector_active;
        engine.world.fast_grid.mask_active = self.mask_active;
        engine.world.static_sight_obstacle_active = self.sight_obstacle_active;
        engine.world.pathfinder = self.pathfinder;

        for planned in &self.patches {
            let patch = &engine.script_domains.interactables.patches[planned.patch_index];
            if planned.applied {
                for &door_index in &patch.door_indices {
                    engine.script_domains.interactables.doors[door_index as usize]
                        .swap_rights_patch();
                }
            }
        }

        for planned in self.patches {
            let patch = &mut engine.script_domains.interactables.patches[planned.patch_index];
            patch.active = planned.active;
            patch.applied = planned.applied;
            patch.in_transition = planned.in_transition;
            patch.locked = planned.locked;
            patch.occupants = planned.occupants;
            // Presentation-only cache: the Rust host recomputes it locally.
            patch.display_doors = false;
            if let Some(fx) = planned.fx {
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
        for planned in self.script_zones {
            engine.script_domains.zones.scripts[planned.index].occupant_indices = planned.occupants;
        }
        for (sector_index, active) in self.door_sectors {
            engine.world.fast_grid.sector_active[sector_index] = active;
        }
        for planned in self.buildings {
            engine.script_domains.buildings.occupants[planned.building_index] = planned.occupants;
            engine.script_domains.buildings.arrow_reserves[planned.building_index] =
                planned.arrow_reserve;
        }
        for (sector_index, lift) in self.lifts {
            if lift.occupants == 0
                && !lift.occupied_upwards
                && !lift.occupied_downwards
                && lift.wait_time == 0
            {
                engine.world.fast_grid.lift_state.remove(&sector_index);
            } else {
                engine.world.fast_grid.lift_state.insert(sector_index, lift);
            }
        }
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
    patch_index: usize,
    saved: &LegacyPatchState,
) -> Result<Option<PlannedPatchFx>, LegacyGridAdoptError> {
    let Some(fx) = &saved.fx else {
        return Ok(None);
    };
    // The nested LegacyElementPayloadBase belongs to entity adoption. This
    // grid plan owns only RHPatchFX's patch attachment and leaf flags.
    // TODO(save-import): compose the entity-base plan here once patch-owned FX
    // elements are included in the unified element adopter.
    let entity_id = entities
        .resolve_element(super::payload_base::LegacyElementRef(Some(
            fx.element.creation_order,
        )))?
        .expect("patch FX creation order is non-null");
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
        force_display: fx.force_display,
        restore_background: fx.restore_background,
    }))
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
        }
    }

    #[test]
    fn rejects_wrong_abi_before_touching_topology() {
        let mut state = empty_state();
        state.abi_profile = LegacySaveAbiProfile::RetailWindowsX86V48;
        let error = LegacyFastFindGridAdoptionPlan::preflight(
            &EngineInner::new(),
            &LevelAssets::new(),
            &state,
            &empty_fixups(),
        )
        .unwrap_err();
        assert!(matches!(error, LegacyGridAdoptError::UnsupportedAbi { .. }));
    }

    #[test]
    fn rejects_lossy_static_repulsive_points_before_mutation() {
        let mut state = empty_state();
        state
            .static_repulsive_points
            .push(LegacyLayeredRepulsivePoint {
                point: LegacyRepulsivePoint {
                    position: LegacyPoint2 { x: 1.0, y: 2.0 },
                    concave: false,
                    limit_left: LegacyPoint2 { x: 0.0, y: 0.0 },
                    limit_right: LegacyPoint2 { x: 0.0, y: 0.0 },
                    action_radius: 3.0,
                    force_a: 4.0,
                    force_b: 5.0,
                    radius: 6.0,
                    id: 7,
                    affects_pcs: true,
                    affects_soldiers: false,
                    affects_civilians: false,
                    affects_animals: false,
                },
                layer: 0,
            });
        let error = LegacyFastFindGridAdoptionPlan::preflight(
            &EngineInner::new(),
            &LevelAssets::new(),
            &state,
            &empty_fixups(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            LegacyGridAdoptError::UnsupportedStaticRepulsivePoints { count: 1 }
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
        engine.world.fast_grid.sector_active = vec![true, true, true];
        engine.world.fast_grid.line_active = vec![true, true];
        engine.world.fast_grid.mask_active = vec![true, true];
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
            locked_pc: false,
            locked_pc_after_patch: true,
            ..crate::gate::Door::default()
        };
        door.locked_npc_villain_after_patch = true;
        engine.script_domains.interactables.doors.push(door);
        engine
            .script_domains
            .zones
            .scripts
            .push(ScriptSectorData::default());
        engine.script_domains.buildings.occupants.push(Vec::new());
        engine.script_domains.buildings.arrow_reserves.push(false);

        let plan = LegacyFastFindGridAdoptionPlan {
            patches: vec![PlannedPatch {
                patch_index: 0,
                active: false,
                applied: true,
                in_transition: false,
                locked: true,
                occupants: vec![OccupantId(9)],
                fx: None,
            }],
            doors: vec![PlannedDoor {
                door_index: 0,
                locked_pc: false,
                locked_npc_villain: false,
                locked_npc_civilian: true,
                unlockable: true,
                special_authorisation_pc: true,
                authorised_pc_direct: 3,
                authorised_pc_indirect: 4,
            }],
            script_zones: vec![PlannedOccupants {
                index: 0,
                occupants: Vec::new(),
            }],
            door_sectors: vec![(2, false)],
            buildings: vec![PlannedBuilding {
                building_index: 0,
                occupants: vec![0x1000_0009],
                arrow_reserve: true,
            }],
            lifts: vec![(
                2,
                LiftRuntimeState {
                    occupants: 2,
                    occupied_upwards: true,
                    occupied_downwards: false,
                    wait_time: 17,
                },
            )],
            line_active: vec![false, true],
            sector_active: vec![false, true, true],
            mask_active: vec![false, true],
            sight_obstacle_active: vec![false, true],
            pathfinder: crate::pathfinder::PathFinder::new(),
        };
        plan.apply(&mut engine);

        let patch = &engine.script_domains.interactables.patches[0];
        assert!(!patch.active);
        assert!(patch.applied);
        assert!(patch.locked);
        assert_eq!(patch.occupants, [OccupantId(9)]);
        assert_eq!(engine.world.fast_grid.sector_active, [false, true, false]);
        assert_eq!(engine.world.fast_grid.line_active, [false, true]);
        assert_eq!(engine.world.fast_grid.mask_active, [false, true]);
        assert_eq!(engine.world.static_sight_obstacle_active, [false, true]);

        let door = &engine.script_domains.interactables.doors[0];
        assert!(!door.locked_pc);
        assert!(!door.locked_npc_villain);
        assert!(door.locked_npc_civilian);
        assert!(door.unlockable);
        assert!(door.special_authorisation_pc);
        assert_eq!(door.authorised_pc_direct, 3);
        assert_eq!(door.authorised_pc_indirect, 4);
        // Patch application swapped the retained future half before the
        // independently serialized current half was restored.
        assert!(!door.locked_pc_after_patch);

        assert_eq!(engine.script_domains.buildings.occupants[0], [0x1000_0009]);
        assert!(engine.script_domains.buildings.arrow_reserves[0]);
        let lift = engine.world.fast_grid.lift_state.get(&2).unwrap();
        assert_eq!(lift.occupants, 2);
        assert!(lift.occupied_upwards);
        assert_eq!(lift.wait_time, 17);
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
