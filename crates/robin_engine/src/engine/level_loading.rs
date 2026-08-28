//! Level loading, entity spawning, and background initialization.

use super::*;
use crate::coordinates::{MapBBox, MapPoint};
use crate::element::{BonusItemTypeExt, Entity, EntityId};

mod entities;
mod environment;
mod finish;
mod pcs;
#[cfg(test)]
mod stage_tests;

/// Validated, staged construction of the mission-authored runtime domains.
///
/// The builder owns the mission identity and explicit script mode; stage inputs
/// and outputs are explicit values so transient proto data cannot leak into
/// gameplay state.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
struct MissionLevelBuilder {
    mission_name: String,
    script_enabled: bool,
    has_authored_content: bool,
}

fn debug_view_radius_light(
    raw_index: usize,
    raw: &crate::level_data::RawLightSector,
    runtime_ambience_mask: u32,
    decision: &str,
    registered: Option<(u32, crate::sector::SectorNumber)>,
) {
    let mut sum_x = 0.0_f32;
    let mut sum_y = 0.0_f32;
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for &(x, y) in &raw.polygon.points {
        let x = f32::from(x);
        let y = f32::from(y);
        sum_x += x;
        sum_y += y;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    let coefficient = if raw.polygon.points.is_empty() {
        0.0
    } else {
        1.0 / raw.polygon.points.len() as f32
    };
    let barycentre_x = sum_x * coefficient;
    let barycentre_y = sum_y * coefficient;
    let grid_index = registered.map_or(-1, |(index, _)| i64::from(index));
    let sector_number = registered.map_or(-1, |(_, number)| i64::from(i16::from(number)));
    eprintln!(
        "VRLIGHT {{\"engine\":\"rust\",\"stage\":\"sector\",\"raw_index\":{raw_index},\"layer\":{},\"ambience\":{},\"runtime_ambience_mask\":{runtime_ambience_mask},\"point_count\":{},\"barycentre_bits\":[{},{}],\"bbox_bits\":[{},{},{},{}],\"decision\":\"{decision}\",\"grid_index\":{grid_index},\"sector_number\":{sector_number}}}",
        raw.layer,
        raw.ambience,
        raw.polygon.points.len(),
        barycentre_x.to_bits(),
        barycentre_y.to_bits(),
        min_x.to_bits(),
        min_y.to_bits(),
        max_x.to_bits(),
        max_y.to_bits(),
    );
}

#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
struct DoorStageOutput {
    authored_door_count: usize,
    building_gates: Vec<Vec<i32>>,
}

#[derive(
    Clone, Debug, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
struct LiftPreflight {
    authored_door_count: usize,
}

use crate::gate::lift_endpoint_door_indices;

#[derive(
    Clone, Debug, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
struct PatchStageOutput {
    patch_count: usize,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
struct BuildingTenantAttachment {
    building_index: usize,
    first_door_index: Option<crate::gate::DoorIndex>,
    tenant_element_indices: Vec<u16>,
    arrow_reserve: bool,
}

#[derive(
    Clone, Debug, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
struct BuildingStageOutput {
    attachments: Vec<BuildingTenantAttachment>,
}

#[derive(
    Clone, Debug, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
struct MissionLevelBuildPlan {
    buildings: BuildingStageOutput,
    building_gates: Vec<Vec<i32>>,
    patch_count: usize,
}

fn retain_legacy_grid_topology(
    assets: &mut LevelAssets,
    loaded: &crate::level_data::LoadedLevel,
    script_enabled: bool,
) -> Result<(), MissionLevelBuildError> {
    use crate::level_data::{MissionGridChunk, ProtoGridChunk, RawBuildingEntry};

    assets.entities.legacy_proto_element_chunk_order = loaded.proto.element_chunk_order.clone();
    assets.entities.legacy_mission_element_chunk_order = loaded.mission.element_chunk_order.clone();
    assets.entities.legacy_mission_element_group_order = loaded.mission.element_group_order.clone();
    assets.entities.legacy_proto_patch_count = loaded.proto.patches.len();
    assets.entities.legacy_proto_animation_count = loaded.proto.animations.len();

    let has_proto_grid_data = !loaded.proto.patches.is_empty()
        || loaded.proto.motion_data.is_some()
        || !loaded.proto.material_sectors.is_empty()
        || !loaded.proto.sight_obstacles.is_empty()
        || !loaded.proto.lifts.is_empty()
        || !loaded.proto.buildings.is_empty()
        || !loaded.proto.light_sectors.is_empty()
        || !loaded.proto.jump_zones.is_empty()
        || !loaded.proto.jump_line_pairs.is_empty();
    if has_proto_grid_data && loaded.proto.grid_chunk_order.is_empty() {
        return Err(MissionLevelBuildError::MissingGridChunkOrder {
            stream: "proto-level".to_owned(),
        });
    }
    let has_mission_grid_data = !loaded.mission.mission_patches.is_empty()
        || loaded.mission.script_objects.is_some()
        || loaded
            .mission
            .tactic_data
            .as_ref()
            .is_some_and(|tactic| !tactic.reinforcement_points.is_empty());
    if has_mission_grid_data && loaded.mission.grid_chunk_order.is_empty() {
        return Err(MissionLevelBuildError::MissingGridChunkOrder {
            stream: "mission".to_owned(),
        });
    }

    let mut topology = LegacyGridTopologyAssets::default();
    let mut sectors = LegacySectorTopologyBuilder::default();
    let mut material_sector_numbers = Vec::new();
    let runtime_motion_sector_count = loaded.proto.motion_data.as_ref().map_or(0usize, |motion| {
        motion
            .layers
            .iter()
            .flatten()
            .map(|area| 1 + area.obstacles.len())
            .sum()
    });
    let mut next_runtime_motion_sector = 0usize;
    let mut next_runtime_building_sector = runtime_motion_sector_count;
    // `SerializeLinePointer` indexes the combined `arrayLines` of a layer.
    // Reproduce the additions in proto chunk order so jump lines retain the
    // identity they had even though Rust stores them in a separate array.
    let mut line_counts = std::collections::BTreeMap::<u16, usize>::new();
    fn append_lines(
        line_counts: &mut std::collections::BTreeMap<u16, usize>,
        layer: u16,
        count: usize,
    ) {
        let total = line_counts.entry(layer).or_default();
        *total = total
            .checked_add(count)
            .expect("Original per-layer line count overflow");
    }
    let mut seen_proto = std::collections::BTreeSet::new();
    for &chunk in &loaded.proto.grid_chunk_order {
        if !seen_proto.insert(chunk as u8) {
            return Err(MissionLevelBuildError::DuplicateGridConstructionChunk {
                stream: "proto-level".to_owned(),
                chunk: format!("{chunk:?}"),
            });
        }
        match chunk {
            ProtoGridChunk::Patch => {
                append_patch_sector_constructors(&mut sectors, &loaded.proto.patches);
                for patch in &loaded.proto.patches {
                    if !patch.apply_sector.points.is_empty() {
                        append_lines(
                            &mut line_counts,
                            patch.final_layer,
                            patch.apply_sector.points.len(),
                        );
                    }
                }
            }
            ProtoGridChunk::Motion => {
                let Some(motion) = loaded.proto.motion_data.as_ref() else {
                    continue;
                };
                for (layer_index, layer) in motion.layers.iter().enumerate() {
                    let layer_index = u16::try_from(layer_index)
                        .expect("Original motion layer index exceeds u16");
                    for area in layer {
                        append_lines(&mut line_counts, layer_index, area.polygon.points.len());
                        for obstacle in &area.obstacles {
                            append_lines(
                                &mut line_counts,
                                layer_index,
                                obstacle.polygon.points.len(),
                            );
                        }
                        let area_number = sectors.construct();
                        sectors.add_position_sector(
                            area_number,
                            if area.is_lift {
                                LegacyGridSectorAsset::Lift
                            } else {
                                LegacyGridSectorAsset::NullOrOrdinary
                            },
                            next_runtime_motion_sector,
                        );
                        next_runtime_motion_sector += 1;
                        for _ in &area.obstacles {
                            let obstacle_number = sectors.construct();
                            sectors.add_position_sector(
                                obstacle_number,
                                LegacyGridSectorAsset::NullOrOrdinary,
                                next_runtime_motion_sector,
                            );
                            next_runtime_motion_sector += 1;
                        }
                    }
                }
                // `InitializeMotionObstaclesFromProtoStream` constructs and
                // appends the out-of-map motion area after all authored areas.
                sectors.construct();
                sectors.insert_last(LegacyGridSectorAsset::NullOrOrdinary);
            }
            ProtoGridChunk::Sight => {
                for &material_index in &loaded.proto.sight_material_indices {
                    let material = loaded
                        .proto
                        .material_sectors
                        .get(usize::from(material_index))
                        .ok_or_else(|| MissionLevelBuildError::MissingGridChunkOrder {
                            stream: format!(
                                "proto-level SIGHT references missing material {material_index}"
                            ),
                        })?;
                    append_lines(&mut line_counts, 0, material.polygon.points.len());
                }
                for &material_index in &loaded.proto.sight_material_indices {
                    let sector_number = material_sector_numbers
                        .get(usize::from(material_index))
                        .copied()
                        .ok_or_else(|| MissionLevelBuildError::MissingGridChunkOrder {
                            stream: format!(
                                "proto-level SIGHT references material {material_index} before its MATERIAL constructor"
                            ),
                        })?;
                    sectors.add(sector_number, LegacyGridSectorAsset::NullOrOrdinary);
                }
                // Every projection-area obstacle owns a constructed plane
                // sector. The synthetic ground projection obstacle does not.
                for _ in loaded
                    .proto
                    .sight_obstacles
                    .iter()
                    .filter(|obstacle| obstacle.projection_area.is_some())
                {
                    let plane_number = sectors.construct();
                    sectors.add(plane_number, LegacyGridSectorAsset::NullOrOrdinary);
                }
            }
            ProtoGridChunk::Material => {
                material_sector_numbers = loaded
                    .proto
                    .material_sectors
                    .iter()
                    .map(|_| sectors.construct())
                    .collect();
            }
            ProtoGridChunk::Bond => {
                for line in &loaded.proto.elevation_lines {
                    append_lines(&mut line_counts, line.layer, 1);
                }
            }
            ProtoGridChunk::Lift => {
                for lift in &loaded.proto.lifts {
                    // The associated click sector consumes a global sector
                    // number, but both AddSector calls are commented out.
                    // A later added door therefore exposes this as a null hole.
                    sectors.construct();
                    for door in &lift.doors {
                        append_door_constructor(&mut topology, &mut sectors, door);
                    }
                }
            }
            ProtoGridChunk::Building => {
                for entry in &loaded.proto.buildings {
                    let (building_number, doors) = match entry {
                        RawBuildingEntry::Building { doors } => (Some(sectors.construct()), doors),
                        RawBuildingEntry::StandaloneDoors { doors } => (None, doors),
                    };
                    for door in doors {
                        append_door_constructor(&mut topology, &mut sectors, door);
                    }
                    if let Some(building_number) = building_number {
                        // `RHSectorBuilding` is constructed before its doors
                        // and only added to `marraySectors` afterwards.  Its
                        // public `GetSectorNumber()` is therefore this sparse
                        // constructor slot, not Rust's dense arena index.
                        sectors.add_position_sector_with_public_number(
                            building_number,
                            LegacyGridSectorAsset::Building,
                            next_runtime_building_sector,
                            building_number,
                        );
                        next_runtime_building_sector += 1;
                    }
                }
            }
            ProtoGridChunk::Light => {
                for light in &loaded.proto.light_sectors {
                    let sector_number = sectors.construct();
                    if light.ambience & loaded.mission.header.ambiance != 0 {
                        sectors.add(sector_number, LegacyGridSectorAsset::NullOrOrdinary);
                    }
                }
            }
            ProtoGridChunk::Jump => {
                let jump_sector_numbers = loaded
                    .proto
                    .jump_zones
                    .iter()
                    .map(|_| sectors.construct())
                    .collect::<Vec<_>>();
                topology.gates.extend(std::iter::repeat_n(
                    LegacyGridGateAsset::Stateless,
                    loaded.proto.jump_line_pairs.len(),
                ));
                for pair in &loaded.proto.jump_line_pairs {
                    for zone_index in [pair.line2.jump_zone_index, pair.line1.jump_zone_index] {
                        let zone = loaded
                            .proto
                            .jump_zones
                            .get(usize::from(zone_index))
                            .ok_or_else(|| MissionLevelBuildError::MissingGridChunkOrder {
                                stream: format!(
                                    "proto-level JUMP line references missing zone {zone_index}"
                                ),
                            })?;
                        let index = *line_counts.entry(zone.layer).or_default();
                        let index = i16::try_from(index).map_err(|_| {
                            MissionLevelBuildError::MissingGridChunkOrder {
                                stream: format!(
                                    "proto-level layer {} exceeds Original signed line-index domain",
                                    zone.layer
                                ),
                            }
                        })?;
                        topology.jump_line_identities.push((zone.layer, index));
                        append_lines(&mut line_counts, zone.layer, 1);
                    }
                }
                for sector_number in jump_sector_numbers {
                    sectors.add(sector_number, LegacyGridSectorAsset::NullOrOrdinary);
                }
            }
        }
    }

    let mut seen_mission = std::collections::BTreeSet::new();
    for &chunk in &loaded.mission.grid_chunk_order {
        if !seen_mission.insert(chunk as u8) {
            return Err(MissionLevelBuildError::DuplicateGridConstructionChunk {
                stream: "mission".to_owned(),
                chunk: format!("{chunk:?}"),
            });
        }
        match chunk {
            MissionGridChunk::Tactic => {
                let count = loaded
                    .mission
                    .tactic_data
                    .as_ref()
                    .map_or(0, |tactic| tactic.reinforcement_points.len());
                topology
                    .gates
                    .extend(std::iter::repeat_n(LegacyGridGateAsset::Door, count));
            }
            MissionGridChunk::Script => {
                if let Some(objects) = loaded.mission.script_objects.as_ref() {
                    topology.script_objects.extend(std::iter::repeat_n(
                        LegacyGridScriptObjectAsset::NonSector,
                        objects.points.len() + objects.lines.len(),
                    ));
                    for sector in &objects.sectors {
                        topology
                            .script_objects
                            .push(LegacyGridScriptObjectAsset::Sector {
                                associated_class: script_enabled
                                    .then(|| sector.script_class.clone())
                                    .flatten(),
                            });
                        let sector_number = sectors.construct();
                        if sector.polygon.points.len() >= 3 {
                            sectors.add(sector_number, LegacyGridSectorAsset::NullOrOrdinary);
                        }
                    }
                }
            }
            MissionGridChunk::Patch => {
                append_patch_sector_constructors(&mut sectors, &loaded.mission.mission_patches);
            }
        }
    }

    let patches = loaded
        .proto
        .patches
        .iter()
        .chain(&loaded.mission.mission_patches)
        .enumerate()
        .collect::<Vec<_>>();
    let mut next_index_by_layer = std::collections::BTreeMap::<u16, u16>::new();
    for (patch_index, patch) in patches {
        let index_in_layer = next_index_by_layer.entry(patch.final_layer).or_default();
        let fx_entity_handle = assets
            .entities
            .patch_animation_entities
            .get(patch_index)
            .copied()
            .flatten();
        if fx_entity_handle.is_none() {
            return Err(MissionLevelBuildError::MissingPatchFxIdentity { patch_index });
        }
        topology.patches.push(LegacyGridPatchAsset {
            patch_index: patch_index as u32,
            layer: patch.final_layer,
            index_in_layer: *index_in_layer,
            fx_entity_handle,
        });
        *index_in_layer = index_in_layer.checked_add(1).ok_or_else(|| {
            MissionLevelBuildError::PatchAttachmentCountMismatch {
                attachment_count: usize::from(u16::MAX),
                patch_count: patch_index + 1,
            }
        })?;
    }
    topology
        .patches
        .sort_by_key(|patch| (patch.layer, patch.index_in_layer));
    topology.sectors = sectors.slots;
    topology.position_sector_numbers = sectors.position_sector_numbers;
    topology.position_sector_indices = sectors.position_sector_indices;
    assets.legacy_grid_topology = Some(topology);
    Ok(())
}

#[derive(Default)]
struct LegacySectorTopologyBuilder {
    next_constructor_number: usize,
    slots: Vec<LegacyGridSectorAsset>,
    position_sector_numbers: Vec<Option<i16>>,
    position_sector_indices: Vec<Option<crate::fast_find_grid::SectorIndex>>,
}

impl LegacySectorTopologyBuilder {
    fn construct(&mut self) -> usize {
        let number = self.next_constructor_number;
        self.next_constructor_number += 1;
        number
    }

    fn add(&mut self, constructor_number: usize, kind: LegacyGridSectorAsset) {
        if self.slots.len() <= constructor_number {
            self.slots.resize(
                constructor_number + 1,
                LegacyGridSectorAsset::NullOrOrdinary,
            );
            self.position_sector_numbers
                .resize(constructor_number + 1, None);
            self.position_sector_indices
                .resize(constructor_number + 1, None);
        }
        self.slots[constructor_number] = kind;
    }

    fn add_position_sector(
        &mut self,
        constructor_number: usize,
        kind: LegacyGridSectorAsset,
        runtime_sector_index: usize,
    ) {
        self.add_position_sector_with_public_number(
            constructor_number,
            kind,
            runtime_sector_index,
            runtime_sector_index,
        );
    }

    fn add_position_sector_with_public_number(
        &mut self,
        constructor_number: usize,
        kind: LegacyGridSectorAsset,
        runtime_sector_index: usize,
        public_sector_number: usize,
    ) {
        self.add(constructor_number, kind);
        self.position_sector_numbers[constructor_number] = Some(
            i16::try_from(public_sector_number)
                .expect("position-sector number exceeds Original signed-word domain"),
        );
        let runtime_sector_index =
            u32::try_from(runtime_sector_index).expect("runtime position-sector index exceeds u32");
        self.position_sector_indices[constructor_number] = Some(
            crate::fast_find_grid::SectorIndex::new(runtime_sector_index)
                .expect("runtime position-sector index equals the null sentinel"),
        );
    }

    fn insert_last(&mut self, kind: LegacyGridSectorAsset) {
        self.slots.push(kind);
        self.position_sector_numbers.push(None);
        self.position_sector_indices.push(None);
    }
}

/// Finish the pointer identity that Original establishes in
/// `RHSectorBuilding::InitializeFromProtoStream` + `AddSector`.
///
/// Runtime motion/building objects occupy a dense Rust arena, while an
/// Original building's public sector number is its (possibly sparse)
/// `marraySectors` slot.  Every door owned by the building is then rewired to
/// that exact object.  Keeping those two identities separate matters when
/// constructor holes make the sparse number differ from the arena index.
fn canonicalize_building_position_sectors(
    assets: &LevelAssets,
    loaded: &mut crate::level_data::LoadedLevel,
    level: &mut crate::fast_find_grid::LevelGrid,
) {
    let retained = assets
        .legacy_grid_topology
        .as_ref()
        .expect("building identity canonicalization requires retained topology");
    let bindings = retained
        .sectors
        .iter()
        .zip(&retained.position_sector_numbers)
        .zip(&retained.position_sector_indices)
        .filter_map(|((kind, number), index)| {
            matches!(kind, LegacyGridSectorAsset::Building).then(|| {
                (
                    number.expect("building has no public sector number"),
                    index.expect("building has no arena identity"),
                )
            })
        })
        .collect::<Vec<_>>();
    let building_count = loaded
        .proto
        .buildings
        .iter()
        .filter(|entry| matches!(entry, crate::level_data::RawBuildingEntry::Building { .. }))
        .count();
    assert_eq!(
        bindings.len(),
        building_count,
        "retained building identities disagree with authored building entries"
    );

    let mut bindings = bindings.into_iter();
    for entry in &mut loaded.proto.buildings {
        let crate::level_data::RawBuildingEntry::Building { doors } = entry else {
            continue;
        };
        let (public_number, arena_index) = bindings
            .next()
            .expect("authored building has no retained identity");
        let sector = level
            .sectors
            .get_mut(usize::from(arena_index))
            .expect("retained building arena index is absent at runtime");
        assert!(
            sector.sector_type.is_building(),
            "retained building identity points at a non-building arena object"
        );
        sector.sector_number = crate::sector::SectorNumber::new(public_number);
        let public_number =
            u16::try_from(public_number).expect("Original building sector number is negative");
        for door in doors {
            door.sector_in = public_number;
        }
    }

    // `add_sector` populated this map before the sparse identities were
    // retained. Rebuild it in arena insertion order, preserving its normal
    // last-object-wins behavior while removing the obsolete dense aliases.
    level.sector_number_map.clear();
    for (index, sector) in level.sectors.iter().enumerate() {
        level.sector_number_map.insert(sector.sector_number, index);
    }
}

fn validate_legacy_position_sector_bijection(
    topology: &LegacyGridTopologyAssets,
    runtime_sectors: &[crate::fast_find_grid::GridSector],
) -> Result<(), String> {
    if topology.position_sector_numbers.len() != topology.sectors.len()
        || topology.position_sector_indices.len() != topology.sectors.len()
    {
        return Err(format!(
            "retained sparse sector arrays disagree: objects={}, public_numbers={}, runtime_indices={}",
            topology.sectors.len(),
            topology.position_sector_numbers.len(),
            topology.position_sector_indices.len(),
        ));
    }

    let mut claimed = std::collections::BTreeMap::new();
    for (sparse_slot, (&public_number, &runtime_index)) in topology
        .position_sector_numbers
        .iter()
        .zip(&topology.position_sector_indices)
        .enumerate()
    {
        match (public_number, runtime_index) {
            (None, None) => continue,
            (Some(_), None) | (None, Some(_)) => {
                return Err(format!(
                    "Original sparse sector slot {sparse_slot} has only one half of its public/runtime identity"
                ));
            }
            (Some(public_number), Some(runtime_index)) => {
                if let Some(previous_slot) = claimed.insert(runtime_index, sparse_slot) {
                    return Err(format!(
                        "Original sparse sector slots {previous_slot} and {sparse_slot} both map to runtime sector index {runtime_index}"
                    ));
                }
                let runtime = runtime_sectors
                    .get(usize::from(runtime_index))
                    .ok_or_else(|| {
                        format!(
                            "Original sparse sector slot {sparse_slot} maps to absent runtime sector index {runtime_index} (runtime count {})",
                            runtime_sectors.len()
                        )
                    })?;
                if i16::from(runtime.sector_number) != public_number {
                    return Err(format!(
                        "Original sparse sector slot {sparse_slot} maps to runtime index {runtime_index}, whose public number {} differs from retained {public_number}",
                        i16::from(runtime.sector_number)
                    ));
                }
                let expected_kind = topology.sectors[sparse_slot];
                let kind_matches = match expected_kind {
                    LegacyGridSectorAsset::Building => runtime.sector_type.is_building(),
                    LegacyGridSectorAsset::Lift => runtime.sector_type.is_lift(),
                    LegacyGridSectorAsset::NullOrOrdinary => runtime.sector_type.is_motion(),
                    LegacyGridSectorAsset::Door { .. } => false,
                };
                if !kind_matches {
                    return Err(format!(
                        "Original sparse sector slot {sparse_slot} ({expected_kind:?}) maps to incompatible runtime sector index {runtime_index} with type {:?}",
                        runtime.sector_type
                    ));
                }
            }
        }
    }
    // Position sectors are registered as the dense runtime prefix. Later
    // shadow/jump/patch click sectors also live in `level.sectors`, but have
    // no `RHposition` pointer role and therefore no sparse position binding.
    for runtime_index in 0..claimed.len() {
        let runtime_index = crate::fast_find_grid::SectorIndex::new(
            u32::try_from(runtime_index).map_err(|_| {
                "runtime motion/building sector count exceeds u32 during bijection validation"
                    .to_owned()
            })?,
        )
        .ok_or_else(|| {
            "runtime motion/building sector index equals the null sentinel".to_owned()
        })?;
        if !claimed.contains_key(&runtime_index) {
            return Err(format!(
                "runtime motion/building sector index {runtime_index} has no Original sparse sector slot"
            ));
        }
    }
    Ok(())
}

/// Resolve the sector pointer stored on every authored script location after
/// the current mission's sparse `marraySectors` topology has been retained.
/// `RHScript::RecordMove` passes this exact `RHSector*` through to
/// `RHSequence::AppendMoveToSequence`; reducing it to the public number makes
/// duplicate-number sectors disconnected from exact door endpoints.
fn retain_script_location_sector_identities(
    assets: &mut LevelAssets,
    loaded: &crate::level_data::LoadedLevel,
) {
    let Some(script_objects) = loaded.mission.script_objects.as_ref() else {
        assets.scripts.location_sector_handles = std::sync::Arc::new(Vec::new());
        return;
    };
    let topology = assets
        .legacy_grid_topology
        .as_ref()
        .expect("script location resolution requires retained Original sector topology");
    let resolve = |sparse_slot: u16| {
        let slot = usize::from(sparse_slot);
        let public = topology
            .position_sector_numbers
            .get(slot)
            .copied()
            .flatten()
            .unwrap_or_else(|| {
                panic!("script location references unmappable Original sector slot {sparse_slot}")
            });
        let arena = topology
            .position_sector_indices
            .get(slot)
            .copied()
            .flatten()
            .unwrap_or_else(|| {
                panic!("script location sector slot {sparse_slot} has no exact runtime identity")
            });
        let public = u16::try_from(public).unwrap_or_else(|_| {
            panic!("script location sector slot {sparse_slot} has negative public number {public}")
        });
        Some(
            crate::position_interface::SectorHandle::new(public)
                .unwrap_or_else(|| {
                    panic!("script location sector slot {sparse_slot} resolves to null sentinel")
                })
                .with_arena_index(arena),
        )
    };
    let handles = script_objects
        .points
        .iter()
        .map(|point| point.sector)
        .map(resolve)
        // Shipped script lines are absent. Script sectors carry their own
        // numeric `muwSector`, not an `RHSector*` resolved through
        // `marraySectors` (RHsector.cpp/RHSectorScript).
        .chain(script_objects.lines.iter().map(|_| None))
        .chain(script_objects.sectors.iter().map(|_| None))
        .collect::<Vec<_>>();
    assert_eq!(
        handles.len(),
        assets.scripts.location_count,
        "script location exact-sector identities disagree with authored locations"
    );
    assets.scripts.location_sector_handles = std::sync::Arc::new(handles);
}

/// Resolve each serialized waypoint `RHSector*` sparse slot to the exact live
/// sector pointer identity retained by Original's `SerializeSectorPointer`.
fn resolve_hiking_waypoint_sector_identities(
    assets: &mut LevelAssets,
    runtime_sectors: &[crate::fast_find_grid::GridSector],
) {
    let topology = assets
        .legacy_grid_topology
        .as_ref()
        .expect("waypoint sector resolution requires retained Original grid topology");
    let paths = std::sync::Arc::make_mut(&mut assets.hiking_paths);
    let exact_paths = paths
        .iter_mut()
        .enumerate()
        .map(|(path_index, path)| {
            path.waypoints
                .iter_mut()
                .enumerate()
                .map(|(waypoint_index, waypoint)| {
                    let sparse_slot = usize::from(waypoint.sector);
                    let public_number = topology
                        .position_sector_numbers
                        .get(sparse_slot)
                        .copied()
                        .flatten()
                        .unwrap_or_else(|| {
                            panic!(
                                "hiking path {path_index} waypoint {waypoint_index} references unmappable Original sector slot {sparse_slot}"
                            )
                        });
                    let runtime_index = topology
                        .position_sector_indices
                        .get(sparse_slot)
                        .copied()
                        .flatten()
                        .unwrap_or_else(|| {
                            panic!(
                                "hiking path {path_index} waypoint {waypoint_index} sector slot {sparse_slot} has no exact runtime identity"
                            )
                        });
                    let runtime = runtime_sectors.get(usize::from(runtime_index)).unwrap_or_else(|| {
                        panic!(
                            "hiking path {path_index} waypoint {waypoint_index} sector slot {sparse_slot} resolves outside runtime sector arena at {runtime_index}"
                        )
                    });
                    assert_eq!(
                        i16::from(runtime.sector_number),
                        public_number,
                        "hiking path {path_index} waypoint {waypoint_index} sector slot {sparse_slot} public/runtime identity conflict"
                    );
                    let public = u16::try_from(public_number).unwrap_or_else(|_| {
                        panic!(
                            "hiking path {path_index} waypoint {waypoint_index} sector slot {sparse_slot} resolved negative public number {public_number}"
                        )
                    });
                    waypoint.sector = public;
                    crate::position_interface::SectorHandle::new(public)
                        .unwrap_or_else(|| {
                            panic!(
                                "hiking path {path_index} waypoint {waypoint_index} resolved reserved no-sector value {public}"
                            )
                        })
                        .with_arena_index(runtime_index)
                })
                .collect()
        })
        .collect();
    assets.hiking_waypoint_sectors = Some(std::sync::Arc::new(exact_paths));
}

fn append_patch_sector_constructors(
    sectors: &mut LegacySectorTopologyBuilder,
    patches: &[crate::level_data::RawPatch],
) {
    // RHPatch constructs old mouse, old masking, new mouse, new masking,
    // apply, and no-apply sectors unconditionally. Empty polygons are
    // deleted but still leave their constructor-numbered array hole.
    for patch in patches {
        for polygon in [
            &patch.old_mouse_sector,
            &patch.old_masking_sector,
            &patch.new_mouse_sector,
            &patch.new_masking_sector,
            &patch.apply_sector,
        ] {
            let sector_number = sectors.construct();
            if !polygon.points.is_empty() {
                sectors.add(sector_number, LegacyGridSectorAsset::NullOrOrdinary);
            }
        }
        // The no-apply sector is constructed but never passed to AddSector.
        sectors.construct();
    }
}

fn append_door_constructor(
    topology: &mut LegacyGridTopologyAssets,
    sectors: &mut LegacySectorTopologyBuilder,
    door: &crate::level_data::RawDoor,
) {
    let gate_index =
        u32::try_from(topology.gates.len()).expect("Original gate array index exceeds u32");
    topology.gates.push(LegacyGridGateAsset::Door);
    let sector_number = sectors.construct();
    if !door.door_sector.points.is_empty() {
        sectors.add(sector_number, LegacyGridSectorAsset::Door { gate_index });
    }
}

#[cfg(test)]
mod legacy_grid_topology_tests {
    use super::*;
    use crate::level_data::{
        ProtoGridChunk, RawArcheryPoint, RawArcherySector, RawBuildingEntry, RawDoor, RawJumpLine,
        RawJumpLinePair, RawJumpZone, RawLift, RawSeekPoint, RawTacticData, SectorPolygon,
    };

    fn door(has_click_sector: bool) -> RawDoor {
        RawDoor {
            door_type: 0,
            active: true,
            locked_pc: false,
            unlockable: false,
            locked_npc_villain: false,
            locked_npc_civilian: false,
            locked_pc_after_patch: false,
            unlockable_after_patch: false,
            locked_npc_villain_after_patch: false,
            locked_npc_civilian_after_patch: false,
            door_sector: SectorPolygon {
                points: has_click_sector
                    .then_some(vec![(0, 0), (1, 0), (0, 1)])
                    .unwrap_or_default(),
            },
            point_out: (0, 0),
            sector_out: 0,
            layer_out: 0,
            point_mid: (0, 0),
            point_in: (0, 0),
            sector_in: 0,
            layer_in: 0,
        }
    }

    fn jump_line() -> RawJumpLine {
        RawJumpLine {
            point_a: (0, 0, 0),
            point_b: (1, 0, 0),
            jump_zone_index: 0,
        }
    }

    fn runtime_position_sector(
        number: i16,
        sector_type: crate::sector::SectorType,
    ) -> crate::fast_find_grid::GridSector {
        crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type,
            layer: 0,
            sector_number: crate::sector::SectorNumber::new(number),
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
        }
    }

    #[test]
    fn constructor_holes_only_enter_sparse_array_when_later_add_exposes_them() {
        let mut builder = LegacySectorTopologyBuilder::default();
        builder.construct();
        let door = builder.construct();
        builder.add(door, LegacyGridSectorAsset::Door { gate_index: 0 });
        builder.construct();

        assert_eq!(
            builder.slots,
            vec![
                LegacyGridSectorAsset::NullOrOrdinary,
                LegacyGridSectorAsset::Door { gate_index: 0 },
            ]
        );
        assert_eq!(builder.position_sector_numbers, vec![None, None]);
        assert_eq!(builder.position_sector_indices, vec![None, None]);
    }

    #[test]
    fn retains_mixed_door_jump_order_and_sparse_special_sector_slots() {
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.proto.grid_chunk_order = vec![
            ProtoGridChunk::Lift,
            ProtoGridChunk::Building,
            ProtoGridChunk::Jump,
        ];
        loaded.proto.lifts.push(RawLift {
            motion_area_index: 0,
            lift_type: 1,
            doors: vec![door(false), door(true)],
            direction: 0,
        });
        loaded.proto.buildings = vec![
            RawBuildingEntry::Building {
                doors: vec![door(true)],
            },
            RawBuildingEntry::StandaloneDoors {
                doors: vec![door(false)],
            },
        ];
        loaded.proto.jump_zones.push(RawJumpZone {
            polygon: SectorPolygon {
                points: vec![(0, 0), (1, 0), (0, 1)],
            },
            sector: 0,
            layer: 0,
            helper_needed: false,
        });
        loaded.proto.jump_line_pairs.push(RawJumpLinePair {
            line1: jump_line(),
            line2: jump_line(),
            jump_long: false,
        });

        let mut assets = LevelAssets::new();
        retain_legacy_grid_topology(&mut assets, &loaded, false).unwrap();
        let topology = assets.legacy_grid_topology.unwrap();

        assert_eq!(
            topology.gates,
            vec![
                LegacyGridGateAsset::Door,
                LegacyGridGateAsset::Door,
                LegacyGridGateAsset::Door,
                LegacyGridGateAsset::Door,
                LegacyGridGateAsset::Stateless,
            ]
        );
        assert_eq!(
            topology.sectors,
            vec![
                // Lift associated-sector and empty lift-door holes.
                LegacyGridSectorAsset::NullOrOrdinary,
                LegacyGridSectorAsset::NullOrOrdinary,
                LegacyGridSectorAsset::Door { gate_index: 1 },
                // Building is constructed before its door, then added after it.
                LegacyGridSectorAsset::Building,
                LegacyGridSectorAsset::Door { gate_index: 2 },
                // Empty standalone door exposed by the later jump-zone AddSector.
                LegacyGridSectorAsset::NullOrOrdinary,
                LegacyGridSectorAsset::NullOrOrdinary,
            ]
        );
        assert_eq!(
            topology.position_sector_numbers,
            vec![None, None, None, Some(3), None, None, None],
            "the building keeps its sparse Original sector number independently of its compact Rust arena index"
        );
        assert_eq!(
            topology.position_sector_indices,
            vec![
                None,
                None,
                None,
                crate::fast_find_grid::SectorIndex::new(0),
                None,
                None,
                None,
            ],
            "the sparse Original building slot retains its exact runtime arena identity"
        );
    }

    #[test]
    fn building_doors_share_sparse_public_and_exact_arena_identity() {
        let mut assets = LevelAssets::new();
        let mut sectors = vec![LegacyGridSectorAsset::NullOrOrdinary; 250];
        sectors[249] = LegacyGridSectorAsset::Building;
        let mut public_numbers = vec![None; 250];
        public_numbers[0] = Some(95);
        public_numbers[249] = Some(249);
        let mut arena_indices = vec![None; 250];
        arena_indices[0] = crate::fast_find_grid::SectorIndex::new(0);
        arena_indices[249] = crate::fast_find_grid::SectorIndex::new(1);
        assets.legacy_grid_topology = Some(LegacyGridTopologyAssets {
            sectors,
            position_sector_numbers: public_numbers,
            position_sector_indices: arena_indices,
            ..LegacyGridTopologyAssets::default()
        });

        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        let mut first = door(false);
        first.door_type = 1;
        first.sector_out = 0;
        let mut second = first.clone();
        second.point_out = (20, 0);
        second.point_in = (10, 0);
        loaded.proto.buildings = vec![RawBuildingEntry::Building {
            doors: vec![first, second],
        }];

        let mut engine = EngineInner::new();
        let outside = runtime_position_sector(
            95,
            crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        );
        let mut building = runtime_position_sector(
            95,
            crate::sector::SectorType::MOTION
                | crate::sector::SectorType::AREA
                | crate::sector::SectorType::BUILDING,
        );
        building.layer = 13;
        engine.world.fast_grid_mut().level_mut().sectors = vec![outside, building];
        engine
            .world
            .fast_grid_mut()
            .level_mut()
            .sector_number_map
            .insert(crate::sector::SectorNumber::new(95), 1);

        canonicalize_building_position_sectors(
            &assets,
            &mut loaded,
            engine.world.fast_grid_mut().level_mut(),
        );
        validate_legacy_position_sector_bijection(
            assets.legacy_grid_topology.as_ref().unwrap(),
            &engine.world.fast_grid.level.sectors,
        )
        .expect("sparse public numbers and dense arena indices stay a bijection");
        engine.build_door_stage(&assets, &loaded, &MissionLevelBuildPlan::default());

        for built in &engine.script_domains.interactables.doors {
            assert_eq!(built.sector_in, crate::sector::SectorNumber::new(249));
            assert_eq!(
                built.sector_in_index,
                crate::fast_find_grid::SectorIndex::new(1),
                "every adjacent door side must retain the one constructed RHSectorBuilding pointer"
            );
        }
        assert_eq!(
            engine.world.fast_grid.level.sector_number_map[&crate::sector::SectorNumber::new(95)],
            0,
            "removing the obsolete dense building alias must expose the real motion sector"
        );
    }

    #[test]
    fn door_sparse_endpoints_resolve_distinct_arena_objects_with_same_public_number() {
        let mut assets = LevelAssets::new();
        assets.legacy_grid_topology = Some(LegacyGridTopologyAssets {
            sectors: vec![
                LegacyGridSectorAsset::NullOrOrdinary,
                LegacyGridSectorAsset::NullOrOrdinary,
            ],
            position_sector_numbers: vec![Some(18), Some(18)],
            position_sector_indices: vec![
                crate::fast_find_grid::SectorIndex::new(40),
                crate::fast_find_grid::SectorIndex::new(41),
            ],
            ..LegacyGridTopologyAssets::default()
        });

        let outside = EngineInner::resolve_sparse_position_sector(&assets, 0);
        let inside = EngineInner::resolve_sparse_position_sector(&assets, 1);

        assert_eq!(outside.0, crate::sector::SectorNumber::new(18));
        assert_eq!(inside.0, crate::sector::SectorNumber::new(18));
        assert_eq!(
            outside.1,
            crate::fast_find_grid::SectorIndex::new(40).unwrap()
        );
        assert_eq!(
            inside.1,
            crate::fast_find_grid::SectorIndex::new(41).unwrap()
        );
        assert_ne!(outside.1, inside.1);
    }

    #[test]
    fn tactic_seek_position_resolves_sparse_slot_to_exact_sector_object() {
        let mut assets = LevelAssets::new();
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.mission.tactic_data = Some(RawTacticData {
            reinforcement_points: Vec::new(),
            ambush_points: Vec::new(),
            seek_points: vec![RawSeekPoint {
                x: 658,
                y: 2905,
                sector: 1,
                level: 0,
                direction: 7,
            }],
            archery_sectors: Vec::new(),
        });
        let mut engine = EngineInner::new();

        // The environment stage runs before the current mission retains its
        // topology. It must not install seek positions through stale assets.
        assets.legacy_grid_topology = Some(LegacyGridTopologyAssets {
            sectors: vec![
                LegacyGridSectorAsset::NullOrOrdinary,
                LegacyGridSectorAsset::NullOrOrdinary,
            ],
            position_sector_numbers: vec![Some(9), Some(9)],
            position_sector_indices: vec![
                crate::fast_find_grid::SectorIndex::new(38),
                crate::fast_find_grid::SectorIndex::new(39),
            ],
            ..LegacyGridTopologyAssets::default()
        });
        engine.load_environment_stage(&mut assets, &mut loaded, false);
        assert!(engine.ai.global.seek_points.is_empty());

        // The deferred stage observes only the newly retained topology and
        // preserves duplicate-public-sector object identity.
        assets.legacy_grid_topology = Some(LegacyGridTopologyAssets {
            sectors: vec![
                LegacyGridSectorAsset::NullOrOrdinary,
                LegacyGridSectorAsset::NullOrOrdinary,
            ],
            position_sector_numbers: vec![Some(18), Some(18)],
            position_sector_indices: vec![
                crate::fast_find_grid::SectorIndex::new(40),
                crate::fast_find_grid::SectorIndex::new(41),
            ],
            ..LegacyGridTopologyAssets::default()
        });
        let tactic_position = EngineInner::resolve_sparse_position_handle(&assets, 1);
        assert_eq!(tactic_position.get(), 18);
        assert_eq!(
            tactic_position.arena_index(),
            crate::fast_find_grid::SectorIndex::new(41),
            "a tactic RHposition must retain its sparse-slot sector identity rather than interpreting the slot as public sector 1"
        );
        engine.install_tactic_seek_points_stage(&assets, &loaded);
        let installed = &engine.ai.global.seek_points[0].position;
        assert_eq!((installed.x, installed.y), (658.0, 2905.0));
        assert_eq!(installed.sector, Some(tactic_position));
    }

    #[test]
    fn archery_waypoint_resolves_to_exact_motion_sector_after_loading() {
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.mission.tactic_data = Some(RawTacticData {
            reinforcement_points: Vec::new(),
            ambush_points: Vec::new(),
            seek_points: Vec::new(),
            archery_sectors: vec![RawArcherySector {
                sector_ref: 25,
                polygon: SectorPolygon::default(),
                points: vec![RawArcheryPoint {
                    x: 983,
                    y: 1518,
                    sector: 25,
                    is_shooting_point: false,
                    direction: 0,
                }],
            }],
        });
        let mut engine = EngineInner::new();
        engine.ai.global.archery_sectors = vec![crate::ai::SectorArchery {
            points: vec![crate::ai::PointArchery {
                position: crate::ai::Position {
                    x: 983.0,
                    y: 1518.0,
                    sector: crate::position_interface::SectorHandle::new(25),
                    level: 0,
                },
                direction: 0,
                is_shooting_point: false,
                sector_index: crate::sector::SectorNumber::new(25),
                owner: None,
            }],
            polygon: Vec::new(),
            layer: 0,
            index_first_shooting_point: None,
            index_last_shooting_point: None,
            num_shooting_points: 0,
            num_owners: 0,
        }];
        let mut motion_sector = runtime_position_sector(
            25,
            crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        );
        motion_sector.layer = 2;
        let arena = engine.world.fast_grid_mut().add_sector(motion_sector, 2);

        engine
            .resolve_archery_topology_after_motion(&loaded)
            .expect("authored archery topology resolves after motion loading");

        let point = &engine.ai.global.archery_sectors[0].points[0].position;
        assert_eq!(point.level, 2);
        assert_eq!(point.sector.expect("archery point sector").get(), 25);
        assert_eq!(
            point.sector.and_then(|sector| sector.arena_index()),
            crate::fast_find_grid::SectorIndex::new(arena),
            "AI GoTo must not mix an exact actor source with a number-only archery destination"
        );
    }

    #[test]
    fn door_ai_cache_retains_exact_endpoint_when_public_numbers_overlap() {
        let outside = crate::fast_find_grid::SectorIndex::new(40).unwrap();
        let inside = crate::fast_find_grid::SectorIndex::new(41).unwrap();
        let mut engine = EngineInner::new();
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                door_type: crate::gate::DoorType::Building,
                sector_out: crate::sector::SectorNumber::new(18),
                sector_in: crate::sector::SectorNumber::new(18),
                sector_out_index: Some(outside),
                sector_in_index: Some(inside),
                ..crate::gate::Door::default()
            });
        let reinforcement_outside = crate::fast_find_grid::SectorIndex::new(50).unwrap();
        let reinforcement_inside = crate::fast_find_grid::SectorIndex::new(51).unwrap();
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                door_type: crate::gate::DoorType::Reinforcement,
                sector_out: crate::sector::SectorNumber::new(19),
                sector_in: crate::sector::SectorNumber::new(19),
                sector_out_index: Some(reinforcement_outside),
                sector_in_index: Some(reinforcement_inside),
                ..crate::gate::Door::default()
            });

        engine.cache_door_ai_metadata();

        let cached = engine.ai.global.door_seek_infos[0]
            .position_in
            .sector
            .expect("building door cache must retain its interior sector");
        assert_eq!(cached.get(), 18);
        assert_eq!(cached.arena_index(), Some(inside));
        assert_ne!(cached.arena_index(), Some(outside));

        let reinforcement = &engine.ai.global.reinforcement_doors[0];
        let reinforcement_in = reinforcement
            .position_in
            .sector
            .expect("reinforcement cache must retain its interior sector");
        let reinforcement_out = reinforcement
            .sector_out
            .expect("reinforcement cache must retain its exterior sector");
        assert_eq!(reinforcement_in.get(), 19);
        assert_eq!(reinforcement_out.get(), 19);
        assert_eq!(reinforcement_in.arena_index(), Some(reinforcement_inside));
        assert_eq!(reinforcement_out.arena_index(), Some(reinforcement_outside));
        assert_ne!(
            reinforcement_in.arena_index(),
            reinforcement_out.arena_index(),
            "equal public reinforcement endpoints must not collapse their arena identity"
        );
    }

    #[test]
    fn retained_position_bijection_excludes_appended_out_of_map_sector() {
        let topology = LegacyGridTopologyAssets {
            sectors: vec![LegacyGridSectorAsset::NullOrOrdinary],
            position_sector_numbers: vec![Some(7)],
            position_sector_indices: vec![crate::fast_find_grid::SectorIndex::new(0)],
            ..LegacyGridTopologyAssets::default()
        };
        let runtime = vec![
            runtime_position_sector(
                7,
                crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            ),
            // Original appends this real pointer after authored position
            // sectors. It is required for reinforcement gates, but is not
            // part of the retained authored-position prefix.
            runtime_position_sector(
                -1,
                crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            ),
        ];

        validate_legacy_position_sector_bijection(&topology, &runtime)
            .expect("the appended out-of-map sector must not extend the authored prefix");
    }

    #[test]
    fn waypoint_sparse_slot_resolves_exact_overlapping_sector_identity() {
        use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

        let mut assets = LevelAssets::new();
        assets.hiking_paths = std::sync::Arc::new(vec![RawHikingPath {
            waypoints: vec![RawWaypoint {
                x: 1432,
                y: 930,
                // Original's serialized sparse marraySectors slot, not the
                // public sector number shared by both runtime objects.
                sector: 2,
                level: 6,
                command: WaypointCommand::None,
            }],
        }]);
        assets.legacy_grid_topology = Some(LegacyGridTopologyAssets {
            sectors: vec![
                LegacyGridSectorAsset::NullOrOrdinary,
                LegacyGridSectorAsset::NullOrOrdinary,
                LegacyGridSectorAsset::NullOrOrdinary,
            ],
            position_sector_numbers: vec![Some(82), None, Some(82)],
            position_sector_indices: vec![
                crate::fast_find_grid::SectorIndex::new(0),
                None,
                crate::fast_find_grid::SectorIndex::new(1),
            ],
            ..LegacyGridTopologyAssets::default()
        });
        let runtime = vec![
            runtime_position_sector(
                82,
                crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            ),
            runtime_position_sector(
                82,
                crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            ),
        ];

        resolve_hiking_waypoint_sector_identities(&mut assets, &runtime);

        assert_eq!(assets.hiking_paths[0].waypoints[0].sector, 82);
        let exact = assets.hiking_waypoint_sectors.as_ref().unwrap()[0][0];
        assert_eq!(exact.get(), 82);
        assert_eq!(
            exact.arena_index(),
            crate::fast_find_grid::SectorIndex::new(1)
        );
    }
}

impl MissionLevelBuilder {
    fn new(
        mission_name: &str,
        script_enabled: bool,
        loaded: &crate::level_data::LoadedLevel,
    ) -> Self {
        let has_authored_content = !mission_name.is_empty()
            || loaded.proto.motion_data.is_some()
            || !loaded.proto.buildings.is_empty()
            || !loaded.proto.lifts.is_empty()
            || !loaded.proto.patches.is_empty()
            || !loaded.mission.mission_patches.is_empty()
            || !loaded.mission.building_tenants.is_empty()
            || loaded.mission.script_objects.is_some()
            || !loaded.mission.soldiers.is_empty()
            || !loaded.mission.civilians.is_empty()
            || !loaded.mission.targets.is_empty()
            || !loaded.mission.bonuses.is_empty()
            || !loaded.mission.scrolls.is_empty();
        Self {
            mission_name: mission_name.to_owned(),
            script_enabled,
            has_authored_content,
        }
    }

    /// Match `RHengine.cpp`: `Bind("StartUp")` is required only when
    /// scripting is enabled. `MissionScript` construction performs that bind,
    /// so a present VM proves both requirements here.
    fn preflight_script_binding(&self, engine: &EngineInner) -> Result<(), MissionLevelBuildError> {
        if self.script_enabled && self.has_authored_content && engine.scripts.mission.is_none() {
            return Err(MissionLevelBuildError::MissingMissionScript {
                mission: self.mission_name.clone(),
            });
        }
        Ok(())
    }

    fn door_stage(
        &self,
        loaded: &crate::level_data::LoadedLevel,
    ) -> Result<DoorStageOutput, MissionLevelBuildError> {
        let mut authored_door_count = 0usize;
        let mut building_gates = Vec::new();
        for (entry_index, entry) in loaded.proto.buildings.iter().enumerate() {
            let (doors, is_building) = match entry {
                crate::level_data::RawBuildingEntry::Building { doors } => (doors, true),
                crate::level_data::RawBuildingEntry::StandaloneDoors { doors } => (doors, false),
            };
            let first_door_index = authored_door_count;
            for (door_index, door) in doors.iter().enumerate() {
                if !is_building && !matches!(door.door_type, 0 | 3 | 7) {
                    return Err(MissionLevelBuildError::InvalidStandaloneDoorType {
                        entry_index,
                        door_index,
                        door_type: door.door_type,
                        x: door.point_mid.0,
                        y: door.point_mid.1,
                    });
                }
                authored_door_count += 1;
            }
            if is_building {
                building_gates.push(
                    (first_door_index..authored_door_count)
                        .map(crate::natives::ScriptHandleCodec::door_handle_from_index)
                        .collect(),
                );
            }
        }
        Ok(DoorStageOutput {
            authored_door_count,
            building_gates,
        })
    }

    fn preflight_lifts(&self, loaded: &crate::level_data::LoadedLevel) -> LiftPreflight {
        LiftPreflight {
            authored_door_count: loaded.proto.lifts.iter().map(|lift| lift.doors.len()).sum(),
        }
    }

    fn patch_stage(
        &self,
        engine: &EngineInner,
        assets: &LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
        doors: &DoorStageOutput,
    ) -> Result<PatchStageOutput, MissionLevelBuildError> {
        let patches = loaded
            .proto
            .patches
            .iter()
            .chain(&loaded.mission.mission_patches);
        let patch_count = loaded.proto.patches.len() + loaded.mission.mission_patches.len();
        if assets.entities.patch_animation_entities.len() != patch_count {
            return Err(MissionLevelBuildError::PatchAttachmentCountMismatch {
                attachment_count: assets.entities.patch_animation_entities.len(),
                patch_count,
            });
        }
        for (patch_index, patch) in patches.enumerate() {
            for &door_index in &patch.door_indices {
                if usize::from(door_index) >= doors.authored_door_count {
                    return Err(MissionLevelBuildError::PatchDoorOutOfRange {
                        patch_index,
                        door_index,
                        door_count: doors.authored_door_count,
                    });
                }
            }
            for (state, refs) in [("old", &patch.old_masks), ("new", &patch.new_masks)] {
                for mask_ref in refs {
                    let exists = engine
                        .world
                        .fast_grid
                        .level
                        .layers
                        .get(mask_ref.layer as usize)
                        .and_then(|layer| layer.mask_indices.get(mask_ref.index as usize))
                        .is_some();
                    if !exists {
                        return Err(MissionLevelBuildError::MissingPatchMask {
                            patch_index,
                            state: state.to_owned(),
                            layer: mask_ref.layer,
                            mask_index: mask_ref.index,
                        });
                    }
                }
            }
        }
        Ok(PatchStageOutput { patch_count })
    }

    fn building_stage(
        &self,
        engine: &EngineInner,
        loaded: &crate::level_data::LoadedLevel,
    ) -> Result<BuildingStageOutput, MissionLevelBuildError> {
        let building_count = loaded
            .proto
            .buildings
            .iter()
            .filter(|entry| matches!(entry, crate::level_data::RawBuildingEntry::Building { .. }))
            .count();
        if loaded.mission.building_tenants.len() != building_count {
            return Err(MissionLevelBuildError::BuildingTenantCountMismatch {
                tenant_count: loaded.mission.building_tenants.len(),
                building_count,
            });
        }
        let mut attachments = Vec::with_capacity(building_count);
        let mut building_index = 0usize;
        let mut authored_door_index = 0usize;
        for entry in &loaded.proto.buildings {
            let crate::level_data::RawBuildingEntry::Building { doors } = entry else {
                if let crate::level_data::RawBuildingEntry::StandaloneDoors { doors } = entry {
                    authored_door_index += doors.len();
                }
                continue;
            };
            let tenants = &loaded.mission.building_tenants[building_index];
            let first_door_index = if tenants.tenant_element_indices.is_empty() {
                None
            } else {
                if doors.is_empty() {
                    return Err(MissionLevelBuildError::BuildingWithoutDoor { building_index });
                }
                Some(crate::gate::DoorIndex(authored_door_index as u32))
            };
            for &element_index in &tenants.tenant_element_indices {
                let Some(entity_id) = engine
                    .world
                    .entities
                    .id_at_legacy_slot(u32::from(element_index))
                else {
                    return Err(MissionLevelBuildError::MissingBuildingTenant {
                        building_index,
                        element_index,
                    });
                };
                if !engine
                    .world
                    .entities
                    .get(entity_id)
                    .is_some_and(Entity::is_human)
                {
                    return Err(MissionLevelBuildError::NonHumanBuildingTenant {
                        building_index,
                        element_index,
                    });
                }
            }
            attachments.push(BuildingTenantAttachment {
                building_index,
                first_door_index,
                tenant_element_indices: tenants.tenant_element_indices.clone(),
                arrow_reserve: tenants.arrow_reserve,
            });
            authored_door_index += doors.len();
            building_index += 1;
        }
        Ok(BuildingStageOutput { attachments })
    }

    fn preflight(
        &self,
        engine: &EngineInner,
        assets: &LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
    ) -> Result<MissionLevelBuildPlan, MissionLevelBuildError> {
        self.preflight_script_binding(engine)?;
        let doors = self.door_stage(loaded)?;
        let lifts = self.preflight_lifts(loaded);
        let patches = self.patch_stage(engine, assets, loaded, &doors)?;
        let buildings = self.building_stage(engine, loaded)?;
        tracing::debug!(
            mission = %self.mission_name,
            non_lift_doors = doors.authored_door_count,
            lift_doors = lifts.authored_door_count,
            patches = patches.patch_count,
            buildings = buildings.attachments.len(),
            "validated MissionLevelBuilder stages"
        );
        Ok(MissionLevelBuildPlan {
            buildings,
            building_gates: doors.building_gates,
            patch_count: patches.patch_count,
        })
    }
}

/// Convert the serialized position of an animation-kind sprite to the map
/// anchor used by `RHPositionInterface`.
///
/// `RHSprite::LoadPositionInfoFromFile(RHFRAMEKIND_ANIMATION)` stores the
/// serialized pair as the sprite's top-left and adds `GetSpriteCenter()` when
/// it constructs the map position. Mobile children are then translated by the
/// starting waypoint. Treating the serialized pair as a map position shifts
/// `chariot02_cart` by exactly `(-70, -71)`, exposing two horse teams during
/// the authored static-to-mobile handoff.
fn mobile_sprite_map_position(
    raw_x: i16,
    raw_y: i16,
    center: crate::coordinates::SpriteAnchor,
    waypoint: MapPoint,
) -> MapPoint {
    MapPoint::new(
        raw_x as f32 + center.x + waypoint.x,
        raw_y as f32 + center.y + waypoint.y,
    )
}

/// CPU-decoded background map ready for GPU upload.
///
/// Produced by [`EngineInner::pre_decode_background_map`] (slow — bzip2) and
/// consumed by [`EngineInner::apply_background_map`] (fast — GPU upload).
/// Mask compositing is no longer done CPU-side — `mask_overlay.wgsl`
/// samples the live bg texture in the fragment stage.
pub struct PreDecodedBackground {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u16>,
    /// Optional continuous ground-depth field for custom rendered maps.
    /// Values are normalized u16 map-ground Y; zero means no reconstructed
    /// surface at that pixel.
    pub occlusion_depth: Option<Vec<u16>>,
}

/// CPU-decoded minimap ready for GPU upload.  See [`PreDecodedBackground`].
pub struct PreDecodedMinimap {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u16>,
}

/// Apply the position fields embedded in an animation sprite reference.
///
/// `RHSprite::LoadPositionInfoFromFile(RHFRAMEKIND_ANIMATION)` treats the
/// serialized X/Y as the sprite's top-left, then builds its 3D anchor from
/// `(x + center.x, y + center.y + elevation, elevation)`. Keeping that
/// authored elevation is also what decides whether an FX belongs to the
/// background-animation list or the normal sorted display list.
fn apply_animation_sprite_placement(
    sprite: &mut crate::sprite::Sprite,
    raw: &crate::level_data::RawSpriteRef,
) {
    let map_position = MapPoint::new(
        raw.position_x as f32 + sprite.center.x,
        raw.position_y as f32 + sprite.center.y,
    );
    sprite.apply_placement(
        map_position,
        0,
        None,
        0,
        crate::element::GameMaterial::default(),
        None,
        None,
    );

    let elevation = raw.elevation as f32;
    sprite
        .position_iface
        .set_position(crate::coordinates::WorldPoint3D {
            x: map_position.x,
            y: map_position.y + elevation,
            z: elevation,
        });
}

#[cfg(test)]
mod animation_placement_tests {
    use super::apply_animation_sprite_placement;
    use crate::coordinates::{MapPoint, SpriteAnchor};
    use crate::level_data::RawSpriteRef;
    use crate::sprite::Sprite;

    #[test]
    fn animation_position_uses_top_left_center_and_authored_elevation() {
        let mut sprite = Sprite {
            center: SpriteAnchor::new(12.0, 18.0),
            ..Default::default()
        };
        let raw = RawSpriteRef {
            frame_profile_name: String::new(),
            profile_name: String::new(),
            position_x: 100,
            position_y: 200,
            elevation: 30,
        };

        apply_animation_sprite_placement(&mut sprite, &raw);

        assert_eq!(
            sprite.position_iface.map_position(),
            MapPoint::new(112.0, 218.0)
        );
        assert_eq!(
            sprite.position_iface.get_position(),
            crate::coordinates::WorldPoint3D::new(112.0, 248.0, 30.0)
        );
        assert_eq!(
            sprite.position_iface.map_position().x - sprite.center.x,
            raw.position_x as f32
        );
        assert_eq!(
            sprite.position_iface.map_position().y - sprite.center.y,
            raw.position_y as f32
        );
    }
}

#[cfg(test)]
mod mission_level_builder_tests {
    use super::MissionLevelBuilder;
    use crate::coordinates::MapPoint;
    use crate::element::{
        ActorCivilian, ActorData, ActorPc, CivilianData, ElementData, ElementKind, Entity,
        HumanData, NpcData, PcData,
    };
    use crate::engine::{
        EngineInner, JumpGateAttachment, LegacyGridSectorAsset, LegacyGridTopologyAssets,
        LevelAssets, LevelLoadStaging, MissionLevelBuildError,
    };
    use crate::level_data::{
        RawBuildingEntry, RawBuildingTenants, RawDoor, RawLift, RawReinforcementPoint,
        RawTacticData, SectorPolygon,
    };

    fn door(door_type: u8) -> RawDoor {
        RawDoor {
            door_type,
            active: true,
            locked_pc: false,
            unlockable: false,
            locked_npc_villain: false,
            locked_npc_civilian: false,
            locked_pc_after_patch: false,
            unlockable_after_patch: false,
            locked_npc_villain_after_patch: false,
            locked_npc_civilian_after_patch: false,
            door_sector: SectorPolygon { points: Vec::new() },
            point_out: (0, 0),
            sector_out: 0,
            layer_out: 0,
            point_mid: (10, 20),
            point_in: (30, 40),
            sector_in: 1,
            layer_in: 0,
        }
    }

    fn civilian() -> Entity {
        Entity::Civilian(ActorCivilian {
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                active: true,
                ..Default::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            civilian: CivilianData::default(),
        })
    }

    /// Minimal retained sparse-sector table for synthetic door fixtures.
    /// Door endpoints in the shipped format are indices into Original's
    /// `marraySectors`, so tests that exercise the production builder must
    /// retain the same identity table instead of relying on public-number
    /// guessing.
    fn door_assets(slot_count: usize, building_slot: usize) -> LevelAssets {
        assert!(building_slot < slot_count);
        let mut sectors = vec![LegacyGridSectorAsset::NullOrOrdinary; slot_count];
        sectors[building_slot] = LegacyGridSectorAsset::Building;
        let mut assets = LevelAssets::new();
        assets.legacy_grid_topology = Some(LegacyGridTopologyAssets {
            sectors,
            position_sector_numbers: (0..slot_count)
                .map(|slot| Some(i16::try_from(slot).expect("test sector slot fits i16")))
                .collect(),
            position_sector_indices: (0..slot_count)
                .map(|slot| {
                    crate::fast_find_grid::SectorIndex::new(
                        u32::try_from(slot).expect("test sector slot fits u32"),
                    )
                })
                .collect(),
            ..LegacyGridTopologyAssets::default()
        });
        assets
    }

    #[test]
    fn door_stage_keeps_building_and_standalone_authored_order() {
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.proto.buildings = vec![
            RawBuildingEntry::Building {
                doors: vec![door(1), door(2)],
            },
            RawBuildingEntry::StandaloneDoors {
                doors: vec![door(3)],
            },
            RawBuildingEntry::Building {
                doors: vec![door(1)],
            },
        ];
        let builder = MissionLevelBuilder::new("ordering", true, &loaded);

        let stage = builder.door_stage(&loaded).expect("valid authored doors");

        assert_eq!(stage.authored_door_count, 4);
        assert_eq!(
            stage.building_gates,
            vec![
                vec![
                    crate::natives::ScriptHandleCodec::door_handle_from_index(0),
                    crate::natives::ScriptHandleCodec::door_handle_from_index(1),
                ],
                vec![crate::natives::ScriptHandleCodec::door_handle_from_index(3,)],
            ]
        );
    }

    #[test]
    fn door_stage_rejects_illegal_standalone_type_with_context() {
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.proto.buildings = vec![RawBuildingEntry::StandaloneDoors {
            doors: vec![door(4)],
        }];
        let builder = MissionLevelBuilder::new("bad-door", true, &loaded);

        assert_eq!(
            builder.door_stage(&loaded),
            Err(MissionLevelBuildError::InvalidStandaloneDoorType {
                entry_index: 0,
                door_index: 0,
                door_type: 4,
                x: 10,
                y: 20,
            })
        );
    }

    #[test]
    fn script_preflight_rejects_authored_level_without_startup_when_enabled() {
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.proto.buildings = vec![RawBuildingEntry::StandaloneDoors { doors: Vec::new() }];
        let builder = MissionLevelBuilder::new("missing-script", true, &loaded);

        assert_eq!(
            builder.preflight_script_binding(&EngineInner::new()),
            Err(MissionLevelBuildError::MissingMissionScript {
                mission: "missing-script".to_owned(),
            })
        );
    }

    #[test]
    fn no_script_mode_still_constructs_doors_lifts_and_sector_links() {
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.proto.buildings = vec![
            RawBuildingEntry::StandaloneDoors {
                doors: vec![door(3)],
            },
            RawBuildingEntry::Building {
                doors: vec![door(8)],
            },
        ];
        loaded.mission.building_tenants = vec![RawBuildingTenants {
            tenant_element_indices: Vec::new(),
            arrow_reserve: false,
        }];
        let mut lift_door = door(4);
        lift_door.sector_out = 20;
        lift_door.sector_in = 21;
        loaded.proto.lifts = vec![RawLift {
            motion_area_index: 0,
            lift_type: 0,
            doors: vec![lift_door],
            direction: 0,
        }];
        let builder = MissionLevelBuilder::new("no-script", false, &loaded);
        let assets = door_assets(22, 1);
        let mut engine = EngineInner::new();

        let plan = builder
            .preflight(&engine, &assets, &loaded)
            .expect("disabled scripting must not require a mission VM");
        engine
            .build_mission_level_stages(&assets, &loaded, &plan)
            .expect("non-script domains still construct");

        assert!(engine.scripts.mission.is_none());
        assert_eq!(engine.script_domains.interactables.doors.len(), 3);
        assert_eq!(
            engine.script_domains.interactables.doors[0].door_type,
            crate::gate::DoorType::Gate
        );
        assert_eq!(
            engine.script_domains.interactables.doors[1].door_type,
            crate::gate::DoorType::Reinforcement
        );
        assert_eq!(
            engine.script_domains.interactables.doors[2].door_type,
            crate::gate::DoorType::LiftHigh
        );

        engine.cache_door_ai_metadata();
        assert_eq!(engine.ai.global.door_seek_infos.len(), 3);
        assert_eq!(engine.ai.global.reinforcement_doors.len(), 1);
        assert_eq!(
            engine.ai.global.reinforcement_doors[0].door_index,
            crate::gate::DoorIndex(1)
        );

        for sector_number in [0_i16, 1_i16] {
            let level = engine.world.fast_grid_mut().level_mut();
            let grid_index = level.sectors.len();
            level
                .sector_number_map
                .insert(crate::sector::SectorNumber::new(sector_number), grid_index);
            level.sectors.push(crate::fast_find_grid::GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
                layer: 0,
                sector_number: crate::sector::SectorNumber::new(sector_number),
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
            });
        }
        engine.populate_sector_gates_from_doors();
        assert_eq!(
            engine.world.fast_grid.level.sectors[0].gate_indices,
            vec![crate::gate::DoorIndex(0), crate::gate::DoorIndex(1)]
        );
        assert_eq!(
            engine.world.fast_grid.level.sectors[1].gate_indices,
            vec![crate::gate::DoorIndex(0), crate::gate::DoorIndex(1)]
        );
    }

    #[test]
    fn reinforcement_door_resolves_exact_out_of_map_endpoint_identity() {
        let mut engine = EngineInner::new();
        for sector_number in [7_i16, -1_i16] {
            engine.world.fast_grid_mut().level_mut().sectors.push(
                crate::fast_find_grid::GridSector {
                    points: Vec::new(),
                    bounding_box: crate::coordinates::MapBBox::new(),
                    sector_type: crate::sector::SectorType::MOTION
                        | crate::sector::SectorType::AREA,
                    layer: 0,
                    sector_number: crate::sector::SectorNumber::new(sector_number),
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
                },
            );
        }
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                door_type: crate::gate::DoorType::Reinforcement,
                sector_in: crate::sector::SectorNumber::new(7),
                sector_out: crate::sector::SectorNumber::new(-1),
                ..Default::default()
            });

        engine.populate_sector_gates_from_doors();

        let door = &engine.script_domains.interactables.doors[0];
        assert_eq!(
            door.sector_in_index,
            crate::fast_find_grid::SectorIndex::new(0)
        );
        assert_eq!(
            door.sector_out_index,
            crate::fast_find_grid::SectorIndex::new(1)
        );
        assert_eq!(
            engine.world.fast_grid.level.sectors[0].gate_indices,
            vec![crate::gate::DoorIndex(0)]
        );
        assert_eq!(
            engine.world.fast_grid.level.sectors[1].gate_indices,
            vec![crate::gate::DoorIndex(0)]
        );
    }

    #[test]
    fn reinforcement_install_resolves_sparse_slot_across_public_sector_collision() {
        let motion_area = |public| crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            layer: 0,
            sector_number: crate::sector::SectorNumber::new(public),
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
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().level_mut().sectors =
            vec![motion_area(18), motion_area(18), motion_area(-1)];
        engine
            .world
            .fast_grid_mut()
            .level_mut()
            .map_bbox
            .expand_point(MapPoint::new(0.0, 0.0));
        engine
            .world
            .fast_grid_mut()
            .level_mut()
            .map_bbox
            .expand_point(MapPoint::new(100.0, 100.0));
        let mut assets = LevelAssets::new();
        assets.legacy_grid_topology = Some(LegacyGridTopologyAssets {
            sectors: vec![
                LegacyGridSectorAsset::NullOrOrdinary,
                LegacyGridSectorAsset::NullOrOrdinary,
            ],
            position_sector_numbers: vec![Some(18), Some(18)],
            position_sector_indices: vec![
                crate::fast_find_grid::SectorIndex::new(0),
                crate::fast_find_grid::SectorIndex::new(1),
            ],
            ..LegacyGridTopologyAssets::default()
        });
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.mission.tactic_data = Some(RawTacticData {
            reinforcement_points: vec![RawReinforcementPoint {
                x: 10,
                y: 20,
                direction: 0,
                action: 0,
                obstacle_index: 0,
                sector: 1,
                layer: 0,
            }],
            ambush_points: Vec::new(),
            seek_points: Vec::new(),
            archery_sectors: Vec::new(),
        });

        engine.install_reinforcement_doors_stage(&assets, &loaded);

        let door = &engine.script_domains.interactables.doors[0];
        assert_eq!(door.sector_in, crate::sector::SectorNumber::new(18));
        assert_eq!(
            door.sector_in_index,
            crate::fast_find_grid::SectorIndex::new(1),
            "the authored sparse slot must select the second arena object despite equal public numbers"
        );
        assert_eq!(door.sector_out, crate::sector::SectorNumber::new(-1));
        assert_eq!(
            door.sector_out_index,
            crate::fast_find_grid::SectorIndex::new(2)
        );

        engine.cache_door_ai_metadata();
        let cached_inside = engine.ai.global.door_seek_infos[0]
            .position_in
            .sector
            .expect("reinforcement initialization must cache its inside RHposition");
        assert_eq!(cached_inside.get(), 18);
        assert_eq!(
            cached_inside.arena_index(),
            crate::fast_find_grid::SectorIndex::new(1),
            "the init cache must receive the exact sparse-slot identity"
        );
        assert_eq!(
            engine.ai.global.reinforcement_doors[0]
                .position_in
                .sector
                .and_then(|sector| sector.arena_index()),
            crate::fast_find_grid::SectorIndex::new(1)
        );
    }

    #[test]
    fn no_script_mode_still_attaches_jump_gates() {
        let mut engine = EngineInner::new();
        let mut staging = LevelLoadStaging::default();
        staging.attachments.jump_gates.push(JumpGateAttachment {
            point_out: MapPoint::new(1.0, 2.0),
            point_in: MapPoint::new(3.0, 4.0),
            layer_out: 0,
            layer_in: 1,
            sector_out: crate::sector::SectorNumber::new(10),
            sector_in: crate::sector::SectorNumber::new(11),
            sector_out_index: crate::fast_find_grid::SectorIndex::new(10).unwrap(),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(11).unwrap(),
            jump_line_out: 7,
            jump_line_in: 8,
            jump_line_in_helper_needed: false,
            jump_line_out_helper_needed: true,
            penalty: 9.0,
        });

        engine
            .attach_jump_gates(&mut staging)
            .expect("jump gates do not require a mission VM");

        assert!(engine.scripts.mission.is_none());
        assert!(staging.attachments.jump_gates.is_empty());
        assert_eq!(engine.script_domains.interactables.doors.len(), 1);
        assert_eq!(
            engine.script_domains.interactables.doors[0].gate_type,
            crate::gate::GateType::Jump
        );
    }

    #[test]
    fn building_stage_requires_one_tenant_record_per_building() {
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.proto.buildings = vec![RawBuildingEntry::Building {
            doors: vec![door(1)],
        }];
        loaded.mission.building_tenants = vec![
            RawBuildingTenants {
                tenant_element_indices: Vec::new(),
                arrow_reserve: false,
            },
            RawBuildingTenants {
                tenant_element_indices: Vec::new(),
                arrow_reserve: true,
            },
        ];
        let builder = MissionLevelBuilder::new("bad-tenants", true, &loaded);

        let error = builder
            .building_stage(&EngineInner::new(), &loaded)
            .expect_err("mismatched tenant table must fail");
        assert_eq!(
            error,
            MissionLevelBuildError::BuildingTenantCountMismatch {
                tenant_count: 2,
                building_count: 1,
            }
        );
    }

    #[test]
    fn building_without_doors_is_valid_when_it_has_no_tenants() {
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.proto.buildings = vec![RawBuildingEntry::Building { doors: Vec::new() }];
        loaded.mission.building_tenants = vec![RawBuildingTenants {
            tenant_element_indices: Vec::new(),
            arrow_reserve: true,
        }];
        let builder = MissionLevelBuilder::new("empty-building", false, &loaded);

        let stage = builder
            .building_stage(&EngineInner::new(), &loaded)
            .expect("an empty building does not need an attachment door");

        assert_eq!(stage.attachments.len(), 1);
        assert_eq!(stage.attachments[0].first_door_index, None);
        assert!(stage.attachments[0].arrow_reserve);
    }

    #[test]
    fn building_trap_tenant_uses_canonical_adapted_first_door() {
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.proto.buildings = vec![
            RawBuildingEntry::StandaloneDoors {
                doors: vec![door(3)],
            },
            RawBuildingEntry::Building {
                doors: vec![door(2)],
            },
        ];
        loaded.mission.building_tenants = vec![RawBuildingTenants {
            tenant_element_indices: vec![1],
            arrow_reserve: false,
        }];
        let builder = MissionLevelBuilder::new("trap-tenant", false, &loaded);
        let assets = door_assets(2, 1);
        let mut engine = EngineInner::new();
        let carried_id = engine.add_entity(civilian());
        {
            let carried = engine
                .get_entity_mut(carried_id)
                .expect("carried fixture exists")
                .element_data_mut();
            carried.set_layer(2);
            carried.set_sector(crate::position_interface::SectorHandle::new(12));
            carried.set_position_map(MapPoint::new(80.0, 90.0));
        }
        engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                ..Default::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                carried: Some(carried_id),
                ..Default::default()
            },
        }));

        let plan = builder
            .preflight(&engine, &assets, &loaded)
            .expect("valid trap tenant plan");
        assert_eq!(
            plan.buildings.attachments[0].first_door_index,
            Some(crate::gate::DoorIndex(1))
        );
        engine
            .build_mission_level_stages(&assets, &loaded, &plan)
            .expect("construct canonical trap door");
        let adapted_point = engine.script_domains.interactables.doors[1].point_in;
        let adapted_sector = engine.script_domains.interactables.doors[1].sector_in;
        let adapted_sector_index = engine.script_domains.interactables.doors[1]
            .sector_in_index
            .expect("canonical building door retains its interior arena identity");
        assert_ne!(adapted_point, MapPoint::new(30.0, 40.0));

        engine
            .attach_mission_level_stage(&plan)
            .expect("attach tenant through canonical door");

        let (_, tenant) = engine
            .world
            .entities
            .get_legacy_slot(1)
            .expect("tenant remains in authored slot");
        assert_eq!(
            tenant.element_data().sprite.position_iface.map_position(),
            adapted_point
        );
        let tenant_sector = tenant
            .element_data()
            .sector()
            .expect("attached building tenant has a sector");
        assert_eq!(tenant_sector.get(), u16::from(adapted_sector));
        assert_eq!(tenant_sector.arena_index(), Some(adapted_sector_index));
        assert!(!tenant.element_data().active);

        let carried = engine
            .get_entity(carried_id)
            .expect("carried tenant fixture remains live");
        assert!(
            carried.element_data().active,
            "InitOccupant changes the tenant's active state, not its carried actor's"
        );
        let carried_sector = carried
            .element_data()
            .sector()
            .expect("ChangeLayerAndSector propagates the building sector to carried actors");
        assert_eq!(carried_sector.get(), u16::from(adapted_sector));
        assert_eq!(carried_sector.arena_index(), Some(adapted_sector_index));
        assert_eq!(
            carried.element_data().layer(),
            tenant.element_data().layer()
        );
        assert_eq!(
            carried.element_data().position_map(),
            MapPoint::new(80.0, 90.0),
            "InitOccupant changes only the tenant's position after propagated topology"
        );
    }
}

#[cfg(test)]
mod all_sprite_ambiance_variant_tests {
    use super::EngineInner;
    use crate::element::{
        ElementBonus, ElementData, ElementFx, ElementKind, ElementTarget, Entity, FxData,
        ObjectData, ObjectType, TargetData,
    };
    use crate::engine::Ambiance;
    use crate::sprite_variant::SpriteVariant;

    fn bonus() -> Entity {
        Entity::Bonus(ElementBonus {
            element: ElementData {
                kind: ElementKind::ObjectBonus,
                ..Default::default()
            },
            object: ObjectData {
                object_type: ObjectType::BonusApple,
                ..Default::default()
            },
        })
    }

    fn fx(mobile_index: Option<u16>) -> Entity {
        Entity::Fx(ElementFx {
            element: ElementData {
                kind: ElementKind::Fx,
                ..Default::default()
            },
            fx: FxData {
                mobile_index,
                ..Default::default()
            },
        })
    }

    fn target() -> Entity {
        Entity::Target(ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..Default::default()
            },
            fx: FxData::default(),
            target: TargetData::default(),
        })
    }

    #[test]
    fn all_sprite_ambiance_tints_only_day_based_sprites() {
        let mut engine = EngineInner::new();
        engine.world.weather.ambiance = Ambiance::Fog;

        assert_eq!(
            engine.resolve_render_variant(&bonus(), false),
            SpriteVariant::Day
        );
        assert_eq!(
            engine.resolve_render_variant(&bonus(), true),
            SpriteVariant::Fog
        );
        assert_eq!(
            engine.resolve_render_variant(&fx(None), true),
            SpriteVariant::Day
        );
        assert_eq!(
            engine.resolve_render_variant(&target(), true),
            SpriteVariant::Day
        );
        assert_eq!(
            engine.resolve_render_variant(&fx(Some(0)), true),
            SpriteVariant::Fog
        );
        engine.world.weather.ambiance = Ambiance::Night;
        assert_eq!(
            engine.resolve_render_variant(&bonus(), false),
            SpriteVariant::Day
        );
        assert_eq!(
            engine.resolve_render_variant(&bonus(), true),
            SpriteVariant::Night
        );
        assert_eq!(
            engine.resolve_render_variant(&fx(None), true),
            SpriteVariant::Day
        );
        assert_eq!(
            engine.resolve_render_variant(&target(), true),
            SpriteVariant::Day
        );
        assert_eq!(
            engine.resolve_render_variant(&fx(Some(0)), true),
            SpriteVariant::Night
        );
    }
}

/// Minimap bitmap metadata produced by the host after GPU upload and
/// consumed by [`super::Engine::apply_level_bitmaps_loaded`] to finish
/// the minimap-widget wiring (hit mask, map size, initial position).
///
/// `saved_position` is the persisted top-left from the active player
/// profile (`(65536, 65536)` sentinel when never written).  The engine
/// validates it via `MinimapState::set_minimap_position`, snapping to
/// the default corner if the saved point is the sentinel or fully
/// off-screen.
pub struct MinimapBitmapSetup {
    pub hit_mask: crate::minimap::HitMask,
    pub map_size: crate::coordinates::MinimapSize,
    pub saved_position: crate::coordinates::ScreenPoint,
}

/// Look up the sprite file and profile name for a raw bonus type value.
///
/// One entry per bonus type, registering the RHS file and profile name.
/// The raw value comes from the mission file and corresponds to the
/// bonus-type enum (0=Arrow, 1=Stone, …, 18=SwordOfTheState).
pub(crate) fn bonus_type_to_sprite_asset(
    raw_bonus_type: u16,
) -> Option<(&'static str, &'static str, crate::element::ObjectType)> {
    use crate::element::ObjectType;
    match raw_bonus_type {
        0 => Some(("BONUS_Arrows", "BONUS Fleches", ObjectType::BonusArrow)),
        1 => Some(("BONUS_Stones", "BONUS Cailloux", ObjectType::BonusStone)),
        2 => Some(("BONUS_Apples", "BONUS Pommes", ObjectType::BonusApple)),
        3 => Some(("BONUS_Ale", "BONUS Ale", ObjectType::BonusAle)),
        4 => Some(("BONUS_LegOfLamb", "BONUS Gigots", ObjectType::BonusLambLeg)),
        5 => Some(("BONUS_Plants", "BONUS Plantes", ObjectType::BonusPlants)),
        6 => Some(("BONUS_Nets", "BONUS Filets", ObjectType::BonusNet)),
        7 => Some(("BONUS_WaspsNest", "BONUS Guepes", ObjectType::BonusWaspNest)),
        8 => Some((
            "BONUS_MoneyBag",
            "BONUS Bourses d'argent",
            ObjectType::BonusPurse,
        )),
        9 => Some((
            "BONUS_GoldBagsRansom",
            "BONUS Sac d'or rancon",
            ObjectType::BonusRansom,
        )),
        10 => Some((
            "BONUS_FourLeavedClover",
            "BONUS Trefle",
            ObjectType::BonusAmulet,
        )),
        11 => Some(("BONUS_Shield", "Shield", ObjectType::BonusBlazon)),
        12 => Some(("RELIC_Ampulla", "Huile", ObjectType::BonusAmpulla)),
        13 => Some(("RELIC_Spoon", "Cuillere", ObjectType::BonusCoronationSpoon)),
        14 => Some(("RELIC_Crown", "Couronne", ObjectType::BonusRichardsCrown)),
        15 => Some(("RELIC_Stamp", "Sceau", ObjectType::BonusRoyalSeal)),
        16 => Some(("RELIC_Sceptre", "Sceptre", ObjectType::BonusRoyalSceptre)),
        17 => Some(("RELIC_Book", "Registre", ObjectType::BonusDomesdayBook)),
        18 => Some(("RELIC_Sword", "Epee", ObjectType::BonusSwordOfTheState)),
        _ => None,
    }
}

/// Put a freshly loaded actor sprite on the mission-authored animation row.
///
/// `apply_placement` records the actor's direction in `PositionInterface`, but
/// it deliberately does not select an animation row. Leaving row selection to
/// the first `Command::Wait` tick makes a paused frame-zero render show row
/// zero (north) for every actor. The original startup path calls
/// `InitializeAction` / `Wait` while constructing actors, before gameplay can
/// render them.
///
/// Keeping `last_action` initialized is also important for blipped NPCs:
/// `RevealBlip` switches from the silhouette profile to the character profile
/// and derives that profile's row from `last_action + direction`.
fn prime_mission_start_sprite(
    sprite: &mut crate::sprite::Sprite,
    raw_action: u32,
    raw_direction: u32,
    actor_description: &str,
) {
    let action = match crate::order::OrderType::try_from(raw_action) {
        Ok(action) => action,
        Err(_) => {
            tracing::warn!(
                "{actor_description}: unknown mission-start animation ordinal {raw_action}"
            );
            return;
        }
    };
    let direction = (raw_direction & 15) as u16;
    if sprite.force_action_direction(action, direction) {
        return;
    }

    // A blip profile may omit an authored character animation. Preserve the
    // action when it exists in the primary profile so RevealBlip can select
    // the correct primary row. The silhouette remains on its profile's
    // default row until then.
    let resolved = sprite.resolve_animation(action);
    let primary_has_action = sprite
        .conversion
        .get(resolved as usize)
        .is_some_and(|&row| row != crate::sprite_script::UNMAPPED);
    if sprite.use_alternate_profile && primary_has_action {
        sprite.last_action = resolved;
        tracing::warn!(
            "{actor_description}: active blip profile has no {resolved:?} animation; \
             preserving it for reveal"
        );
    } else {
        tracing::warn!(
            "{actor_description}: sprite profile has no mission-start animation {resolved:?}"
        );
    }
}

#[cfg(test)]
mod mission_start_sprite_tests {
    use super::{mobile_sprite_map_position, prime_mission_start_sprite};
    use crate::coordinates::{MapPoint, SpriteAnchor};
    use crate::order::OrderType;
    use crate::sprite::Sprite;
    use crate::sprite_script::UNMAPPED;
    use std::sync::Arc;

    #[test]
    fn primes_authored_action_and_direction_before_first_tick() {
        let action = OrderType::WaitingUprightBored;
        let mut conversion = vec![UNMAPPED; action as usize + 1];
        conversion[action as usize] = 32;
        let mut sprite = Sprite::new(Arc::new(Vec::new()), Arc::new(conversion));

        prime_mission_start_sprite(&mut sprite, action as u32, 11, "test actor");

        assert_eq!(sprite.current_row, 43);
        assert_eq!(sprite.last_action, action);
        assert_eq!(sprite.current_frame, 0);
    }

    #[test]
    fn mobile_animation_position_adds_sprite_center_before_waypoint() {
        let position = mobile_sprite_map_position(
            -70,
            -71,
            SpriteAnchor::new(70.0, 71.0),
            MapPoint::new(772.0, 722.0),
        );

        assert_eq!(position, MapPoint::new(772.0, 722.0));
    }
}

/// Load the raw mission + proto-level binaries for a campaign's current
/// mission.  Standalone helper so the host can parse the mission header
/// (map filename + ambiance) *before* constructing an `Engine`,
/// allowing the background bitmap to be pre-decoded with real
/// dimensions and passed into `Engine::new` as a first-class input —
/// the RAII shape where engine construction fully initializes every
/// field, rather than leaving `fast_grid.map_bbox` zero until a
/// post-construction fixup.
///
/// Handles only the file-I/O half of mission setup; the engine-mutation
/// half (entity spawn, pending motion stash) still lives on
/// `EngineInner::initialize_from_mission`.
pub fn load_mission_for_campaign(
    campaign: &crate::campaign::Campaign,
    profiles: &crate::profiles::ProfileManager,
    level_directory: &str,
    progress: &mut dyn FnMut(f32),
) -> Result<crate::level_data::LoadedLevel, EngineError> {
    let idx = campaign
        .current_mission_idx
        .expect("load_mission_for_campaign: no current mission set");
    let profile = campaign.missions[idx].profile(profiles);
    let mission_filename = &profile.mission_filename;
    let proto_level_filename = &profile.proto_level_filename;

    if crate::level_data::hackable_level_exists(mission_filename) {
        let path = crate::level_data::hackable_level_descriptor_path(mission_filename);
        tracing::info!("Loading hackable level descriptor: {path}");
        let bytes = crate::sbfile::SbFile::read_all(&path).map_err(|error| {
            EngineError::Io(std::io::Error::other(format!(
                "required hackable level descriptor {path} could not be read: error {error}"
            )))
        })?;
        let level =
            crate::level_data::LoadedLevel::hackable_from_json(&bytes).map_err(|error| {
                EngineError::Io(std::io::Error::other(format!(
                    "failed to load {path}: {error}"
                )))
            })?;
        progress(1.0);
        progress(1.0);
        return Ok(level);
    }

    // The is_beggar predicate is needed because beggar civilians have
    // extra scroll-set data in the mission file.  We parse raw data
    // before constructing entities so we pass the check as a closure.
    let mut level = crate::level_data::load_level(
        mission_filename,
        proto_level_filename,
        level_directory,
        &|profile_id| {
            profiles
                .get_civilian(profile_id)
                .is_some_and(|p| p.civilian_type == crate::profiles::CivilianType::Beggar)
        },
        progress,
    )
    .map_err(|e| EngineError::Io(std::io::Error::other(e.to_string())))?;
    apply_character_roster_patch(&mut level, profiles, mission_filename)?;
    Ok(level)
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CharacterRosterPatch {
    initial_character: String,
    rescue_characters: Vec<String>,
    rescue_target_scripts: Vec<String>,
    #[serde(rename = "descriptor_mission")]
    _descriptor_mission: String,
}

fn character_profile_index_by_filename(
    profiles: &crate::profiles::ProfileManager,
    filename: &str,
) -> Result<crate::profiles::CharacterProfileIdx, EngineError> {
    let mut matches = profiles
        .characters
        .iter()
        .enumerate()
        .filter(|(_, profile)| profile.filename == filename);
    let (index, _) = matches
        .next()
        .ok_or_else(|| EngineError::MissionLevelStage {
            stage: "character roster patch",
            reason: format!("unknown character profile filename {filename:?}"),
        })?;
    if matches.next().is_some() {
        return Err(EngineError::MissionLevelStage {
            stage: "character roster patch",
            reason: format!("ambiguous character profile filename {filename:?}"),
        });
    }
    Ok(crate::profiles::CharacterProfileIdx(index as u32))
}

fn apply_character_roster_patch(
    level: &mut crate::level_data::LoadedLevel,
    profiles: &crate::profiles::ProfileManager,
    mission_filename: &str,
) -> Result<(), EngineError> {
    let path = format!("Data/Levels/{mission_filename}.characters.patch.json");
    if !crate::sbfile::SbFile::exists(&path) {
        return Ok(());
    }
    let bytes = crate::sbfile::SbFile::read_all(&path).map_err(|error| {
        EngineError::Io(std::io::Error::other(format!(
            "read required character roster patch {path}: error {error}"
        )))
    })?;
    let patch: CharacterRosterPatch =
        serde_json::from_slice(&bytes).map_err(|error| EngineError::MissionLevelStage {
            stage: "character roster patch",
            reason: format!("parse {path}: {error}"),
        })?;
    if level.mission.beam_mes.len() != 1 {
        return Err(EngineError::MissionLevelStage {
            stage: "character roster patch",
            reason: format!(
                "{path} requires exactly one initial beam-me, found {}",
                level.mission.beam_mes.len()
            ),
        });
    }
    if patch.rescue_characters.len() != level.mission.pcs_to_rescue.len() {
        return Err(EngineError::MissionLevelStage {
            stage: "character roster patch",
            reason: format!(
                "{path} supplies {} rescue profiles for {} rescue slots",
                patch.rescue_characters.len(),
                level.mission.pcs_to_rescue.len()
            ),
        });
    }
    if patch.rescue_target_scripts.len() != patch.rescue_characters.len() {
        return Err(EngineError::MissionLevelStage {
            stage: "character roster patch",
            reason: format!(
                "{path} supplies {} rescue target scripts for {} rescue characters",
                patch.rescue_target_scripts.len(),
                patch.rescue_characters.len()
            ),
        });
    }

    let initial_profile = character_profile_index_by_filename(profiles, &patch.initial_character)?;
    let rescue_profiles = patch
        .rescue_characters
        .iter()
        .map(|filename| character_profile_index_by_filename(profiles, filename))
        .collect::<Result<Vec<_>, _>>()?;
    for (rescue, profile) in level
        .mission
        .pcs_to_rescue
        .iter_mut()
        .zip(rescue_profiles.iter().copied())
    {
        rescue.profile_index = profile.0;
    }
    for (script_class, profile_index) in patch
        .rescue_target_scripts
        .iter()
        .zip(rescue_profiles.iter().copied())
    {
        let matches = level
            .mission
            .targets
            .iter()
            .enumerate()
            .filter_map(|(index, target)| {
                (target.script_class.as_deref() == Some(script_class.as_str())).then_some(index)
            })
            .collect::<Vec<_>>();
        let [target_index] = matches.as_slice() else {
            let reason = if matches.is_empty() {
                format!("{path} target script {script_class:?} does not exist")
            } else {
                format!("{path} target script {script_class:?} is ambiguous")
            };
            return Err(EngineError::MissionLevelStage {
                stage: "character roster patch",
                reason,
            });
        };
        let profile = profiles.get_character(profile_index).ok_or_else(|| {
            EngineError::MissionLevelStage {
                stage: "character roster patch",
                reason: format!("{path} resolved missing character profile {profile_index}"),
            }
        })?;
        let target = &mut level.mission.targets[*target_index];
        target.filename = profile.filename.clone();
        target.profile_name = profile.profile_name.clone();
        target.action = crate::order::OrderType::BeingTied as u32;
        target.character_sprite = true;
    }
    let beam = level
        .mission
        .beam_mes
        .first_mut()
        .expect("exactly one beam-me was validated above");
    beam.profile_override = Some(initial_profile.0);
    beam.robin_role = true;
    // Save Stuteley authors Robin's cape/disguise idle at this slot. Villain
    // NPC sprite sets do not contain that hero-only animation, so begin the
    // replacement in the ordinary upright idle instead.
    beam.action = crate::order::OrderType::WaitingUpright as u32;
    tracing::info!(
        "Applied character roster patch {path}: initial={:?}, rescues={:?}",
        patch.initial_character,
        patch.rescue_characters
    );
    Ok(())
}

/// Map a beam-me `actionInitial` value to a `(posture, action_state)` pair for
/// a PC's initial action.
///
/// The raw value is an animation ordinal; unknown values fall back to
/// `(Upright, Waiting)` with a warning log.
fn map_pc_initial_action(
    raw_action: u32,
) -> (crate::element::Posture, crate::element::ActionState) {
    use crate::element::{ActionState, Posture};
    use crate::order::OrderType;

    let anim = match OrderType::try_from(raw_action) {
        Ok(a) => a,
        Err(_) => {
            tracing::warn!(
                "PC InitializeAction: unknown animation ordinal {raw_action}; \
                 defaulting to (Upright, Waiting)"
            );
            return (Posture::Upright, ActionState::Waiting);
        }
    };

    match anim {
        OrderType::WaitingUpright => (Posture::Upright, ActionState::Waiting),
        OrderType::WaitingUprightBored => (Posture::Upright, ActionState::Bored),
        OrderType::WaitingCrouched => (Posture::Crouched, ActionState::Waiting),
        OrderType::BeingDeadFallenBack => (Posture::DeadBack, ActionState::Waiting),
        OrderType::BeingDead => (Posture::Dead, ActionState::Waiting),
        OrderType::BeingUnconscious => (Posture::Lying, ActionState::Waiting),
        // `WaitingCape` → Spy posture (costume-as-civilian).  The Rust
        // titbit sync pass inserts the hidden titbit automatically on
        // hidden postures.
        OrderType::WaitingCape => (Posture::Spy, ActionState::Waiting),
        // `WaitingHidden` → Tree (hidden-in-bush posture).
        OrderType::WaitingHidden => (Posture::Tree, ActionState::Waiting),
        OrderType::Sitting => (Posture::Sitting, ActionState::Waiting),
        OrderType::SleepingUpright => (Posture::Siesta, ActionState::Waiting),
        OrderType::BeingTied => (Posture::Tied, ActionState::Waiting),
        // `Special` is unimplemented; fall through to (Upright, Waiting)
        // with a warning. The game does not ship any level with `Special`
        // as a PC initial action, so this path should be unreachable in
        // practice.
        OrderType::Special => {
            tracing::warn!(
                "PC InitializeAction: Special animation is unimplemented; \
                 defaulting to (Upright, Waiting)"
            );
            (Posture::Upright, ActionState::Waiting)
        }
        other => {
            tracing::warn!(
                "PC InitializeAction: unsupported initial animation {other:?}; \
                 defaulting to (Upright, Waiting)"
            );
            (Posture::Upright, ActionState::Waiting)
        }
    }
}

/// Spawn the animation elements owned by proto- or mission-level patches.
fn spawn_patch_fx_entities(
    engine: &mut EngineInner,
    assets: &mut LevelAssets,
    patches: &[crate::level_data::RawPatch],
    patch_index_offset: usize,
) -> Vec<Option<i32>> {
    let anim_base_dir = "Data/Animations";
    let sprite_ambiance = Some(engine.world.weather.ambiance.to_sprite_ambiance());
    let bank_signature = assets.bank_signature;
    let mut handles = Vec::with_capacity(patches.len());

    for (local_patch_idx, raw) in patches.iter().enumerate() {
        let patch_idx = patch_index_offset + local_patch_idx;
        let fname = &raw.element_fx.sprite.frame_profile_name;
        let profile = &raw.element_fx.sprite.profile_name;

        if fname.is_empty() {
            handles.push(None);
            continue;
        }

        let mut sprite = crate::sprite::Sprite::default();
        match crate::sprite_script::SpriteScriptor::resolve_rhs_path(
            crate::sprite_script::FrameKind::Animation,
            anim_base_dir,
            fname,
            sprite_ambiance,
        ) {
            Ok(path) => {
                let cache_key = format!("{fname}/{profile}");
                match assets.sprite_scriptor_mut().load(
                    &path,
                    profile,
                    &cache_key,
                    crate::sprite_script::FrameKind::Animation,
                    |file| {
                        let mut sig = 0u32;
                        file.serialize_u32(&mut sig)
                            .map_err(|e| format!("read signature: {e}"))?;
                        if sig != bank_signature {
                            return Err(format!(
                                "bank signature mismatch: file {sig:#x} != bank {bank_signature:#x}"
                            ));
                        }
                        Ok(())
                    },
                ) {
                    Ok(info) => {
                        sprite.scripts = info.scripts.clone();
                        sprite.conversion = info.conversion.clone();
                        sprite.center = info.center;
                        sprite.current_width = info.size.x as u16;
                        sprite.current_height = info.size.y as u16;
                        sprite.frame_profile_name = fname.clone();
                        sprite.profile_cache_key = cache_key;
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to load sprite for patch {patch_idx} animation \
                             '{fname}' profile '{profile}': {e}"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::error!(
                    "Failed to resolve RHS path for patch {patch_idx} animation \
                     '{fname}': {e}"
                );
            }
        }

        let initially_active = raw.start_animation_valid;
        apply_animation_sprite_placement(&mut sprite, &raw.element_fx.sprite);
        if initially_active {
            if let Some(row) = sprite.row_for_action(crate::order::OrderType::PATCH_INITIAL) {
                sprite.current_row = row;
            }
            sprite.current_frame = 0;
            sprite.frame_count = 0;
        }

        let entity = crate::element::Entity::Fx(crate::element::ElementFx {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::Fx,
                active: initially_active,
                sprite,
                ..Default::default()
            },
            fx: crate::element::FxData {
                restore_background: raw.integrate_in_background,
                force_display: raw.element_fx.force_display,
                animation: crate::order::OrderType::NonanimationEnd,
                display_polyline: raw
                    .element_fx
                    .display_polyline
                    .iter()
                    .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                    .collect(),
                patch_index: crate::patch::PatchIndex::new(patch_idx as u32),
                mobile_index: None,
                animation_speed: 1.0,
                rendering_properties: if raw.element_fx.blit_type != 0 {
                    crate::element::RenderingProperties::NeedShadow
                } else {
                    crate::element::RenderingProperties::Blocky
                },
            },
        });
        let id = engine.add_entity(entity);
        handles.push(Some(crate::natives::ScriptHandleCodec::actor_handle(id)));
    }

    handles
}

/// Spawn the ordinary FX elements from a proto `ANIMATION` chunk.
fn spawn_proto_animation_fx_entities(
    engine: &mut EngineInner,
    assets: &mut LevelAssets,
    animations: &[crate::level_data::RawElementFx],
) {
    let anim_base_dir = "Data/Animations";
    let sprite_ambiance = Some(engine.world.weather.ambiance.to_sprite_ambiance());
    let bank_signature = assets.bank_signature;

    for raw in animations {
        let fname = &raw.sprite.frame_profile_name;
        let profile = &raw.sprite.profile_name;
        let mut sprite = crate::sprite::Sprite::default();
        match crate::sprite_script::SpriteScriptor::resolve_rhs_path(
            crate::sprite_script::FrameKind::Animation,
            anim_base_dir,
            fname,
            sprite_ambiance,
        ) {
            Ok(path) => {
                let cache_key = format!("{fname}/{profile}");
                match assets.sprite_scriptor_mut().load(
                    &path,
                    profile,
                    &cache_key,
                    crate::sprite_script::FrameKind::Animation,
                    |file| {
                        let mut sig = 0u32;
                        file.serialize_u32(&mut sig)
                            .map_err(|e| format!("read signature: {e}"))?;
                        if sig != bank_signature {
                            return Err(format!(
                                "bank signature mismatch: file {sig:#x} != bank {bank_signature:#x}"
                            ));
                        }
                        Ok(())
                    },
                ) {
                    Ok(info) => {
                        sprite.scripts = info.scripts.clone();
                        sprite.conversion = info.conversion.clone();
                        sprite.center = info.center;
                        sprite.current_width = info.size.x as u16;
                        sprite.current_height = info.size.y as u16;
                        sprite.frame_profile_name = fname.clone();
                        sprite.profile_cache_key = cache_key;
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to load sprite scripts for animation '{fname}' \
                             profile '{profile}': {e}"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to resolve animation RHS path for '{fname}': {e}");
            }
        }
        apply_animation_sprite_placement(&mut sprite, &raw.sprite);
        let entity = Entity::Fx(crate::element::ElementFx {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::Fx,
                active: raw.active,
                sprite,
                ..Default::default()
            },
            fx: crate::element::FxData {
                restore_background: false,
                force_display: raw.force_display,
                animation: crate::order::OrderType::NonanimationEnd,
                display_polyline: raw
                    .display_polyline
                    .iter()
                    .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                    .collect(),
                patch_index: None,
                mobile_index: None,
                animation_speed: 1.0,
                rendering_properties: if raw.blit_type != 0 {
                    crate::element::RenderingProperties::NeedShadow
                } else {
                    crate::element::RenderingProperties::Blocky
                },
            },
        });
        let _ = engine.add_entity(entity);
    }
}

impl EngineInner {
    // ─── Accessory sprite hydration ──────────────────────────────

    /// Attach an accessory sprite (arrow/stone/apple/net/wasp/purse/
    /// coin/ale/cape) to a freshly-spawned projectile or object entity
    /// by cloning from the preloaded master prototype.
    ///
    /// Every projectile spawn pulls its sprite from a global master
    /// registry. The Rust port preloads every accessory sprite into
    /// `LevelAssets::accessory_sprite_prototypes` at level load and
    /// clones here per-spawn — tick paths don't need mutable asset
    /// access.
    ///
    /// No-op if the object type has no preloaded accessory prototype
    /// (bonus-type projectiles spawned as throws reuse the bonus-side
    /// sprite, also preloaded at level load).
    pub(crate) fn attach_accessory_sprite(
        &mut self,
        assets: &crate::engine::LevelAssets,
        id: crate::element::EntityId,
    ) {
        let Some(entity) = self.world.entities.get(id) else {
            return;
        };
        let object_type = match entity {
            crate::element::Entity::Projectile(p) => p.object.object_type,
            crate::element::Entity::Net(n) => n.object.object_type,
            // Ground-dropped bonuses that carry an *accessory* object
            // type (e.g. ObjectType::Ale dropped by DropAle — see
            // `spawn_dropped_ale`) need the preloaded ACCESSORIES
            // sprite cloned in just like a projectile.  Pre-placed
            // BONUS_* bonuses already have their sprite loaded inline
            // at mission spawn time and will not be found in the
            // accessory table, so the `get(&object_type) = None` below
            // makes this a no-op for them — no need to gate explicitly.
            crate::element::Entity::Bonus(b) => b.object.object_type,
            _ => return,
        };
        let Some(prototype) = assets.accessory_sprite_prototypes.get(&object_type) else {
            return;
        };
        let position_iface = entity.element_data().sprite.position_iface.clone();
        let mut sprite = prototype.clone();
        sprite.position_iface = position_iface;
        if let Some(entity) = self.world.entities.get_mut(id) {
            entity.element_data_mut().sprite = sprite;
        }
    }

    /// Preload accessory sprite prototypes at level-load time.
    ///
    /// Called from `initialize_from_mission` after the sprite bank is
    /// available.  Loads one sprite per accessory `ObjectType` (or
    /// `BonusNet`/`BonusWaspNest` for throw-a-pickup projectiles) into
    /// `LevelAssets::accessory_sprite_prototypes`; runtime
    /// [`attach_accessory_sprite`] calls then clone from that table.
    pub(crate) fn preload_accessory_sprite_prototypes(assets: &mut crate::engine::LevelAssets) {
        use crate::element::ObjectType;
        assets.accessory_sprite_prototypes.clear();
        let char_base_dir = "Data/Characters";
        let bank_signature = assets.bank_signature;
        // Every variant accessory registered as a master.
        let mut entries: Vec<(ObjectType, &str, &str)> = vec![
            (ObjectType::Arrow, "ACCESSORIES_Arrow", "ACCESSOIRES Fleche"),
            (
                ObjectType::Stone,
                "ACCESSORIES_Stone",
                "ACCESSOIRES Cailloux",
            ),
            (ObjectType::Ale, "ACCESSORIES_Ale", "ACCESSOIRES Ale"),
            (ObjectType::Apple, "ACCESSORIES_Apple", "ACCESSOIRES Pomme"),
            (
                ObjectType::Purse,
                "ACCESSORIES_MoneyBag",
                "ACCESSOIRES Bourse d'argent",
            ),
            (
                ObjectType::WaspNest,
                "ACCESSORIES_Wasp",
                "ACCESSOIRES Guepes",
            ),
            (ObjectType::Cape, "ACCESSORIES_Coat", "Manteau"),
            (ObjectType::Net, "ACCESSORIES_Net", "ACCESSOIRES Filet"),
            (
                ObjectType::Coin,
                "ACCESSORIES_Coin",
                "ACCESSOIRES Piece d'or",
            ),
            (ObjectType::Wasp, "ACCESSORIES_WaspSting", "Guepe"),
        ];
        // Original creates masters for every bonus and for scrolls even when
        // the current mission contains no authored instance. Save loading can
        // therefore reconstruct any on-the-fly object without depending on
        // which pickups happen to be present in this level.
        for raw_bonus_type in 0..=18 {
            if let Some((file, profile, object_type)) = bonus_type_to_sprite_asset(raw_bonus_type) {
                entries.push((object_type, file, profile));
            }
        }
        entries.push((ObjectType::Scroll, "BONUS_Parchment", "BONUS Parchemin"));
        for (object_type, file, profile) in entries {
            let mut sprite = crate::sprite::Sprite::default();
            if let Err(e) = sprite.load_frame_info(
                assets.sprite_scriptor_mut(),
                crate::sprite_script::FrameKind::Object,
                char_base_dir,
                file,
                profile,
                bank_signature,
                None,
            ) {
                tracing::error!(
                    "Failed to preload accessory sprite '{file}' profile '{profile}': {e}",
                );
                continue;
            }
            assets
                .accessory_sprite_prototypes
                .insert(object_type, sprite);
        }
    }

    /// Preload the scroll-amulet bonus sprite
    /// (`BONUS_FourLeavedClover` / `"BONUS Trefle"`).
    ///
    /// Called at level load so the mid-tick scroll-reveal path
    /// ([`Self::drain_pending_scroll_amulets`]) can hit the scriptor
    /// cache through `&LevelAssets` instead of needing `&mut` to
    /// load on demand (which would break the
    /// "mutation-only-in-perform_hourglass" invariant).
    pub(crate) fn preload_scroll_amulet_sprite(&mut self, assets: &mut crate::engine::LevelAssets) {
        let bank_signature = assets.bank_signature;
        let mut sprite = crate::sprite::Sprite::default();
        if let Err(e) = sprite.load_frame_info(
            assets.sprite_scriptor_mut(),
            crate::sprite_script::FrameKind::Object,
            "Data/Characters",
            "BONUS_FourLeavedClover",
            "BONUS Trefle",
            bank_signature,
            Some(self.world.weather.ambiance.to_sprite_ambiance()),
        ) {
            tracing::error!("Failed to preload scroll-amulet sprite: {e}");
        }
        // We only care that the scriptor cache is populated; discard
        // the Sprite itself — the runtime spawn builds its own.
        drop(sprite);
    }

    /// Preload character sprites for every campaign description.
    ///
    /// The reinforcement spawn ([`Self::drain_pending_reinforcements`])
    /// picks a random non-instanced, non-VIP peasant from the current
    /// gang. That pool is known at level load, so we can eagerly load
    /// each candidate's `.rhs` into the scriptor cache and the mid-tick
    /// spawn path can then use the cache-only `&SpriteScriptor`
    /// accessor.
    ///
    /// Safe to call repeatedly — `SpriteScriptor::load` short-circuits
    /// on cache hit, so re-preloading is a no-op beyond a few hashmap
    /// probes.
    pub(crate) fn preload_campaign_peasant_sprites(
        &mut self,
        assets: &mut crate::engine::LevelAssets,
    ) {
        let Some(campaign) = Some(&self.mission_domain.campaign) else {
            return;
        };
        let bank_signature = assets.bank_signature;
        // Snapshot the profiles we need so we can drop the campaign
        // borrow before mutating assets.
        let profiles: Vec<(
            crate::profiles::CharacterProfileIdx,
            String,
            String,
            Option<String>,
        )> = campaign
            .characters
            .iter()
            .filter_map(|desc| {
                let cpi = desc.character_profile_idx?;
                let profile = assets.profile_manager.get_character(cpi)?;
                let alternate_profile = (profile.valid_alternative_profile
                    && !profile.alternative_profile_name.is_empty())
                .then(|| profile.alternative_profile_name.clone());
                Some((
                    cpi,
                    profile.filename.clone(),
                    profile.profile_name.clone(),
                    alternate_profile,
                ))
            })
            .collect();
        assets.character_sprite_prototypes.clear();
        for (profile_index, filename, profile_name, alternate_profile) in profiles {
            let mut sprite = crate::sprite::Sprite::default();
            if let Err(e) = sprite.load_frame_info(
                assets.sprite_scriptor_mut(),
                crate::sprite_script::FrameKind::Character,
                "Data/Characters",
                &filename,
                &profile_name,
                bank_signature,
                Some(self.world.weather.ambiance.to_sprite_ambiance()),
            ) {
                tracing::warn!(
                    "Failed to preload campaign character sprite '{filename}' / '{profile_name}': {e}",
                );
                continue;
            }
            // RHElementActorPC's constructor loads the alternative track
            // whenever the character profile declares one. Dynamic PCs and
            // save-only PCs are cloned from this cache rather than passing
            // through the ordinary beam-me constructor, so their prototype
            // must carry the same second track as well. Otherwise adopting a
            // serialized `mbAlternateProfile=true` selects an empty Rust
            // slot and silently falls back to the primary animations.
            if let Some(alternate_profile) = alternate_profile
                && let Err(e) = sprite.load_alternate_profile(
                    assets.sprite_scriptor_mut(),
                    crate::sprite_script::FrameKind::Character,
                    "Data/Characters",
                    &filename,
                    &alternate_profile,
                    bank_signature,
                    None,
                )
            {
                tracing::warn!(
                    "Failed to preload alternate campaign character sprite \
                     '{filename}' / '{alternate_profile}': {e}",
                );
                continue;
            }
            assets
                .character_sprite_prototypes
                .insert(profile_index, sprite);
        }
    }

    // ─── Level loading ───────────────────────────────────────────

    /// Load a level from proto-level + mission files.
    ///
    /// This reads chunk-based binary files: the proto-level contains
    /// geometry (motion, sight, patches, etc.) and the mission file
    /// contains actors, scripts, and gameplay data.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn initialize_from_mission(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &mut LevelAssets,
        staging: &mut LevelLoadStaging,
        mission_name: &str,
        proto_level_name: &str,
        mut loaded: crate::level_data::LoadedLevel,
        level_directory: &str,
        bg_pixel_dims: (f32, f32),
        progress: &mut dyn FnMut(f32),
    ) -> Result<(), EngineError> {
        let config = sim.config();
        let level_builder = MissionLevelBuilder::new(mission_name, config.script_enabled, &loaded);

        self.begin_mission_level_stage();
        self.load_environment_stage(assets, &mut loaded, config.script_enabled);
        progress(1.0);

        self.load_sound_sources_stage(assets, &loaded)?;
        progress(1.0);

        self.load_motion_stage(assets, staging, &mut loaded, bg_pixel_dims)?;
        self.spawn_proto_entities_stage(assets, &loaded);
        if loaded.mission.reserve_null_ai_handle {
            self.reserve_null_ai_handle_slot_if_empty();
        }
        progress(1.0);

        // The original engine creates object masters before loading mission
        // entities. Its SpriteScriptor cache is keyed by filename/profile,
        // so targets authored with an accessory filename reuse that global
        // master even though target loading requests FrameKind::Animation.
        Self::preload_accessory_sprite_prototypes(assets);

        self.spawn_civilians_and_rescue_pcs_stage(sim, assets, &loaded, config)?;
        progress(1.0);

        self.spawn_soldiers_stage(assets, &loaded, config)?;
        progress(1.0);

        self.spawn_targets_stage(assets, &loaded);
        progress(1.0);

        self.spawn_bonuses_stage(sim, assets, &loaded);
        progress(1.0);

        let force_visible_scroll_ids = self.spawn_scrolls_stage(assets, &loaded, config);
        progress(1.0);

        self.spawn_beam_me_pcs_stage(sim, assets, &mut loaded)?;
        self.spawn_mobile_elements_stage(sim, assets, &loaded)?;
        self.finish_mission_identity_stage(&loaded, mission_name, proto_level_name);
        self.load_mission_script_stage(
            assets,
            mission_name,
            level_directory,
            config.script_enabled,
            force_visible_scroll_ids,
        );

        retain_legacy_grid_topology(assets, &loaded, config.script_enabled)?;
        canonicalize_building_position_sectors(
            assets,
            &mut loaded,
            self.world.fast_grid_mut().level_mut(),
        );
        // Runtime motion/building objects are registered before proto/mission
        // entities finish constructing the retained Original sparse arrays.
        // Validate the exact sparse-slot ↔ live-index bijection at the first
        // point where both sides exist, before doors or save adoption consume
        // it.
        let retained_topology = assets.legacy_grid_topology.as_ref().unwrap_or_else(|| {
            panic!("retaining Original grid topology completed without a topology value")
        });
        validate_legacy_position_sector_bijection(
            retained_topology,
            &self.world.fast_grid.level.sectors,
        )
        .unwrap_or_else(|detail| panic!("legacy position-sector registration drifted: {detail}"));
        retain_script_location_sector_identities(assets, &loaded);
        self.install_tactic_seek_points_stage(assets, &loaded);
        resolve_hiking_waypoint_sector_identities(assets, &self.world.fast_grid.level.sectors);
        let level_plan = level_builder.preflight(self, assets, &loaded)?;
        self.build_mission_level_stages(assets, &loaded, &level_plan)?;
        self.install_reinforcement_doors_stage(assets, &loaded);
        self.attach_jump_gates(staging)?;
        self.attach_mission_level_stage(&level_plan)?;
        self.cache_door_ai_metadata();
        self.sort_pc_ids_by_priority(assets);
        self.select_highest_priority_pc(assets, 0);

        Ok(())
    }

    /// AI stores entity-table indices in legacy nullable handles, where zero
    /// means no actor. Original levels normally publish a proto entity before
    /// mission humans, but geometry-only hackable maps may publish none. Keep
    /// slot zero vacant in that case so the first NPC cannot alias null.
    pub(crate) fn reserve_null_ai_handle_slot_if_empty(&mut self) {
        if self.world.entities.is_empty() {
            self.world.entities.push(None);
        }
    }

    // ─── Ambiance / sprite variants ────────────────────────────────

    /// Return the sprite variant matching the current ambiance.
    pub fn default_variant(&self) -> crate::sprite_variant::SpriteVariant {
        use crate::sprite_variant::SpriteVariant;
        // Force Day regardless of ambiance when the fog-sprites-crash
        // workaround is enabled.
        if self.control.sim_config.bypass_fog_sprites_crash {
            return SpriteVariant::Day;
        }
        match self.world.weather.ambiance {
            Ambiance::Fog => SpriteVariant::Fog,
            Ambiance::Night => SpriteVariant::Night,
            _ => SpriteVariant::Day,
        }
    }

    /// Resolve the sprite variant to use when rendering `entity` this frame.
    ///
    /// PCs/NPCs always pick up the default variant; objects/projectiles only
    /// do so when their object type has an ambiance variant. Other Day-based
    /// sprites normally retain that palette. With `apply_fog_to_all_sprites`,
    /// those sprites also receive the generated Fog or Night variant.
    pub fn resolve_render_variant(
        &self,
        entity: &crate::element::Entity,
        apply_fog_to_all_sprites: bool,
    ) -> crate::sprite_variant::SpriteVariant {
        use crate::element::Entity;
        use crate::sprite_variant::SpriteVariant;

        // Ordinary FX and targets are loaded from Data/Animations/<Ambiance>,
        // so their pixels already contain the mission ambiance. Applying the
        // generated variant again would double-tint patches and decorations.
        // Mobile child FX are deliberately loaded from the Day directory and
        // therefore still need the generated variant.
        let has_ambiance_baked_pixels = match entity {
            Entity::Fx(fx) => fx.fx.mobile_index.is_none(),
            Entity::Target(_) => true,
            _ => false,
        };
        let force_ambiance_variant = apply_fog_to_all_sprites
            && matches!(self.world.weather.ambiance, Ambiance::Fog | Ambiance::Night)
            && !has_ambiance_baked_pixels;
        let apply_ambiance = force_ambiance_variant
            || match entity {
                // PCs/NPCs always pick up the default variant.
                Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_) => true,
                // Objects/projectiles only pick up the variant when their type
                // has an ambiance variant.
                Entity::Projectile(p) => p.object.object_type.has_ambiance_variant(),
                Entity::Net(n) => n.object.object_type.has_ambiance_variant(),
                Entity::Bonus(b) => b.object.object_type.has_ambiance_variant(),
                // Scroll, Animal, Fx, FxMasked, Target, Mobile: variant stays
                // at its default (Day).
                _ => false,
            };
        if !apply_ambiance {
            return SpriteVariant::Day;
        }

        let default = self.default_variant();
        if !matches!(default, SpriteVariant::Fog | SpriteVariant::Night) {
            return default;
        }

        // Shadow-sector fallback: revert Fog/Night → Day when the entity
        // stands in a `SECTOR_SHADOW` (light-at-night) polygon.
        let elem = entity.element_data();
        if elem.layer() != 0xFFFF
            && self
                .world
                .fast_grid
                .is_in_shadow_sector(elem.position_map(), elem.layer())
        {
            SpriteVariant::Day
        } else {
            default
        }
    }

    // NOTE: `initialize_sprite_variants` was moved to robin_rs
    // (`level_loading_host::initialize_sprite_variants`) as part of
    // the engine carve-out (Decision 1): it only manipulates the host-
    // side `FrameHolder` (variant dictionaries, global shadow values)
    // which the engine must no longer reference. After level load: Day
    // drops night+fog dictionaries (shadow=40, blip=60); Fog drops
    // night and generates fog dictionaries (shadow=10, blip=40); Night
    // drops fog and generates night dictionaries (shadow=40, blip=60).

    /// Pre-pass that rewrites every `Building`-entry door's `sector_in`
    /// / `layer_in` to point at the empty `BUILDING` grid sector the
    /// motion-init pass will allocate for that building, and records
    /// the allocated sector numbers on constructor-local pending data.
    ///
    /// Two-step building load: create one empty-polygon sector per
    /// `Building` entry on the lift layer (type
    /// `MOTION | AREA | BUILDING`), then route every door inside that
    /// building's `sector_in` to point at the building's sector — the
    /// door's stream-read sector is discarded in favour of the
    /// building's.
    ///
    /// We can't defer the rewrite until after motion construction because the
    /// MissionLevelBuilder must consume the rewritten authored doors. This
    /// pre-pass runs during the initial load, right after the level file is
    /// parsed, computes the same sector number each building would get in
    /// the motion-init pass, and patches the raw doors in place.  The motion
    /// pass later creates the matching grid sectors using the stashed numbers
    /// (with a `debug_assert_eq!` that catches any drift between the two
    /// allocators).
    fn rewire_building_doors(
        &mut self,
        staging: &mut LevelLoadStaging,
        buildings: &mut [crate::level_data::RawBuildingEntry],
        motion_data: Option<&crate::level_data::RawMotionData>,
    ) -> Result<(), MissionLevelBuildError> {
        staging.motion.building_sector_numbers.clear();

        // Count motion areas + obstacles in proto order — must match the
        // allocation loop in `initialize_motion_from_level_data`.
        let Some(md) = motion_data else {
            let building_count = buildings
                .iter()
                .filter(|entry| {
                    matches!(entry, crate::level_data::RawBuildingEntry::Building { .. })
                })
                .count();
            if building_count == 0 {
                return Ok(());
            }
            return Err(MissionLevelBuildError::MissingBuildingMotionData { building_count });
        };
        let mut next_sector: i16 = 0;
        for layer_areas in &md.layers {
            for area in layer_areas {
                next_sector += 1; // area polygon
                next_sector += area.obstacles.len() as i16; // obstacles
            }
        }

        // The proto count includes the lift layer:
        // `lift_layer = special_layer - 1 = motion_data.layers.len() - 1`.
        // Computing it from the raw motion data keeps this pass
        // independent of `fast_grid` init order.
        let building_lift_layer = u16::try_from(md.layers.len())
            .expect("motion layer count exceeds u16")
            .checked_sub(1)
            .expect("building mission has no lift layer");

        // Allocate one sector per Building entry, in proto order, and rewrite
        // each of its doors in place.  StandaloneDoors entries are left alone:
        // their `sector_in` already points at a real motion area.
        for entry in buildings.iter_mut() {
            let crate::level_data::RawBuildingEntry::Building { doors } = entry else {
                continue;
            };
            let sn = next_sector;
            next_sector += 1;
            staging.motion.building_sector_numbers.push(sn);
            for door in doors.iter_mut() {
                door.sector_in = sn as u16;
                door.layer_in = building_lift_layer;
            }
        }
        Ok(())
    }

    /// Initialize the fast find grid and pathfinder from motion data loaded from the proto level.
    ///
    /// Must be called after `load_background_map` sets `cutscene_camera.level_size`.
    pub(crate) fn initialize_motion_from_level_data(
        &mut self,
        assets: &mut LevelAssets,
        staging: &mut LevelLoadStaging,
        motion_data: &crate::level_data::RawMotionData,
        lifts: &[crate::level_data::RawLift],
    ) {
        let level_w = self.feedback.cutscene_camera.level_size.x as u16;
        let level_h = self.feedback.cutscene_camera.level_size.y as u16;

        // Size the grid from map dimensions.
        let grid_w = level_w / 64;
        let grid_h = level_h / 64;
        self.world.fast_grid_mut().size_map(grid_w, grid_h);
        let conventional_layers = u16::try_from(motion_data.layers.len())
            .expect("motion layer count exceeds u16")
            .checked_sub(1)
            .expect("motion data has no lift layer");
        self.world
            .fast_grid_mut()
            .allocate_layers(conventional_layers);

        // Register the already-loaded sight obstacles with the grid so
        // per-cell queries (`get_obstacle_indices`) can restrict the
        // 3D raycast scan to overlapping obstacles.
        // Snapshot (idx, layer, box_ground) before mutating fast_grid —
        // `self.sight_obstacles(assets)` borrows engine immutably while
        // `add_obstacle_index` needs `&mut self.world.fast_grid`.
        let obstacle_metadata: Vec<(u32, u16, crate::coordinates::GroundBBox)> = self
            .sight_obstacles(assets)
            .iter_indexed()
            .map(|(idx, obs)| (idx, obs.layer, obs.box_ground))
            .collect();
        for (obs_idx, layer, box_ground) in obstacle_metadata {
            if let Some(idx) = crate::sight_obstacle::SightObstacleIndex::new(obs_idx) {
                self.world
                    .fast_grid_mut()
                    .add_obstacle_index(idx, layer, &box_ground);
            }
        }

        // Drain raw masks stashed by `initialize_from_mission` and push the
        // decoded `RuntimeMask`s into the grid.  Masks are pushed just
        // after the grid is sized.
        let raw_masks = std::mem::take(&mut staging.motion.masks);
        let raw_count = raw_masks.len();
        let mut added = 0usize;
        for raw in raw_masks {
            if let Some(mask) = crate::mask::RuntimeMask::from_raw(&raw) {
                self.world.fast_grid_mut().add_mask(mask);
                added += 1;
            }
        }
        if raw_count > 0 {
            tracing::debug!(
                "Loaded {} masks into fast grid ({} skipped)",
                added,
                raw_count - added,
            );
        }

        // ── Elevation (bond) lines → grid lines ──
        //
        // Each bond line separates two adjacent sight obstacles on the
        // same layer. Register them as non-motion `GridLine`s tagged
        // `is_elevation = true` so the per-tick line-cross query in
        // `tick_entity_movement` can detect when an actor walks over
        // one and dispatch `cross_elevation_line`.
        //
        // The proto stores `right_obstacle_index` before `left`.
        let elev_raw = std::mem::take(&mut staging.motion.elevation_lines);
        let num_obstacles = self.sight_obstacles(assets).len();
        let mut elev_added = 0usize;
        let mut elev_skipped_layer = 0usize;
        for raw in &elev_raw {
            let to_idx = |i: u16| -> Option<u16> {
                // 0xFFFF is the sentinel for "no obstacle" in the proto.
                if i == 0xFFFF {
                    None
                } else if (i as usize) < num_obstacles {
                    Some(i)
                } else {
                    None
                }
            };
            let left = to_idx(raw.left_obstacle_index);
            let right = to_idx(raw.right_obstacle_index);
            let line = crate::fast_find_grid::GridLine::new_elevation(
                MapPoint::new(raw.point_a.0 as f32, raw.point_a.1 as f32),
                MapPoint::new(raw.point_b.0 as f32, raw.point_b.1 as f32),
                left,
                right,
            );
            if (raw.layer as usize) >= self.world.fast_grid.level.layers.len() {
                elev_skipped_layer += 1;
                continue;
            }
            tracing::trace!(
                layer = raw.layer,
                ax = raw.point_a.0,
                ay = raw.point_a.1,
                bx = raw.point_b.0,
                by = raw.point_b.1,
                left_obstacle = ?left,
                right_obstacle = ?right,
                "loading elevation line"
            );
            self.world.fast_grid_mut().add_line(line, raw.layer);
            elev_added += 1;
        }
        if !elev_raw.is_empty() {
            tracing::debug!(
                "Loaded {} elevation lines into fast grid ({} skipped for bad layer)",
                elev_added,
                elev_skipped_layer,
            );
        }

        // ── Part 1: Motion obstacles → grid lines + pathfinder move_layers ──
        // Part 5 registers these same areas/obstacles in exactly this order.
        // Reserve their flat sector slots now so pathfinder state changes can
        // toggle each obstacle sector together with its perimeter lines.
        let mut next_motion_sector_index = u32::try_from(self.world.fast_grid.level.sectors.len())
            .expect("fast-grid sector count exceeds u32");
        for (layer_idx, layer_areas) in motion_data.layers.iter().enumerate() {
            let mut move_areas = Vec::new();
            let mut alt_move_areas = Vec::new();

            for area in layer_areas {
                // The area's own motion sector precedes its obstacles.
                next_motion_sector_index = next_motion_sector_index
                    .checked_add(1)
                    .expect("motion sector index overflow");
                // Add motion area polygon lines to the grid: the
                // perimeter is both motion-blocking and repulsive, so
                // anti-collision pushes actors off walls instead of
                // letting them scrape along.
                let poly = &area.polygon;
                for current in 0..poly.points.len() {
                    // Original AddSectorLines seeds pointLast from the final
                    // polygon vertex, so the closing edge is inserted first,
                    // followed by 0→1, 1→2, ... . FindAutorizedPosition
                    // applies intersecting line pushes in this insertion
                    // order, making the cyclic edge order observable.
                    let previous = if current == 0 {
                        poly.points.len() - 1
                    } else {
                        current - 1
                    };
                    let (x1, y1) = poly.points[previous];
                    let (x2, y2) = poly.points[current];
                    let mut line = crate::fast_find_grid::GridLine::new(
                        MapPoint::new(x1 as f32, y1 as f32),
                        MapPoint::new(x2 as f32, y2 as f32),
                        true, // is_motion
                    );
                    line.initialize_motion_normal(true);
                    line.set_repulsive(true);
                    self.world.fast_grid_mut().add_line(line, layer_idx as u16);
                }
                // Emit cone-limited repulsive points at inward corners
                // of the motion area.  `det(v1, v2) < 0` marks an
                // inward corner (the wedge faces *into* the walkable
                // area, pushing actors away from the pinch point).
                let n = poly.points.len();
                if n > 2 {
                    for i in 0..n {
                        let (ax, ay) = poly.points[i];
                        let (bx, by) = poly.points[(i + 1) % n];
                        let (cx, cy) = poly.points[(i + 2) % n];
                        let v1 = (bx as f32 - ax as f32, by as f32 - ay as f32);
                        let v2 = (cx as f32 - bx as f32, cy as f32 - by as f32);
                        let det = v1.0 * v2.1 - v1.1 * v2.0;
                        if det < 0.0 {
                            // `GetNormal(false)` → (y, -x).
                            let limit_left = crate::coordinates::MapVec::new(v1.1, -v1.0);
                            let limit_right = crate::coordinates::MapVec::new(v2.1, -v2.0);
                            let is_concave =
                                crate::geo2d::cross(limit_left.to_geo(), limit_right.to_geo())
                                    < 0.0;
                            self.world
                                .fast_grid_mut()
                                .level_mut()
                                .level_repulsive_points
                                .push(crate::fast_find_grid::LevelRepulsivePoint {
                                    position: MapPoint::new(bx as f32, by as f32),
                                    layer: layer_idx as u16,
                                    limit_left,
                                    limit_right,
                                    is_concave,
                                });
                        }
                    }
                }

                // Build skeleton segments
                let skeleton: Vec<geo::Line<f32>> = area
                    .skeleton_segments
                    .iter()
                    .map(|&((x1, y1), (x2, y2))| {
                        crate::geo2d::segment(
                            crate::geo2d::pt(x1 as f32, y1 as f32),
                            crate::geo2d::pt(x2 as f32, y2 as f32),
                        )
                    })
                    .collect();

                // Add motion obstacle lines to the grid and build per-obstacle
                // metadata (state_id + bbox + polygon + grid-line indices) for
                // runtime state swaps.  The grid-line indices feed the reverse
                // mapping used by `SetLineForMotionSectorActive` so a
                // state transition can flip each perimeter line's
                // `line_active` flag without rescanning the layer.
                let mut obstacles = Vec::new();
                for obstacle in &area.obstacles {
                    let grid_sector_index = crate::fast_find_grid::SectorIndex::new(
                        next_motion_sector_index,
                    )
                    .expect("motion obstacle sector index uses the invalid u32::MAX sentinel");
                    next_motion_sector_index = next_motion_sector_index
                        .checked_add(1)
                        .expect("motion sector index overflow");
                    let obs_poly = &obstacle.polygon;
                    let mut bbox = crate::coordinates::MapBBox::new();
                    let mut poly_pts: Vec<MapPoint> = Vec::with_capacity(obs_poly.points.len());
                    let mut line_indices: Vec<crate::fast_find_grid::LineIndex> =
                        Vec::with_capacity(obs_poly.points.len());
                    for &(x, y) in &obs_poly.points {
                        let p = MapPoint::new(x as f32, y as f32);
                        bbox.expand_point(p);
                        poly_pts.push(p);
                    }
                    for current in 0..obs_poly.points.len() {
                        let previous = if current == 0 {
                            obs_poly.points.len() - 1
                        } else {
                            current - 1
                        };
                        let (x1, y1) = obs_poly.points[previous];
                        let (x2, y2) = obs_poly.points[current];
                        let mut line = crate::fast_find_grid::GridLine::new(
                            MapPoint::new(x1 as f32, y1 as f32),
                            MapPoint::new(x2 as f32, y2 as f32),
                            true,
                        );
                        line.initialize_motion_normal(false);
                        line.set_repulsive(true);
                        let line_idx = self.world.fast_grid_mut().add_line(line, layer_idx as u16);
                        line_indices.push(line_idx);
                    }
                    // Obstacle-corner repulsive points: `det(v1, v2) > 0`
                    // marks the convex outward corners of the obstacle
                    // polygon.
                    let on = obs_poly.points.len();
                    if on > 2 {
                        for i in 0..on {
                            let (ax, ay) = obs_poly.points[i];
                            let (ox, oy) = obs_poly.points[(i + 1) % on];
                            let (cx, cy) = obs_poly.points[(i + 2) % on];
                            let v1 = (ox as f32 - ax as f32, oy as f32 - ay as f32);
                            let v2 = (cx as f32 - ox as f32, cy as f32 - oy as f32);
                            let det = v1.0 * v2.1 - v1.1 * v2.0;
                            if det > 0.0 {
                                let limit_left = crate::coordinates::MapVec::new(v1.1, -v1.0);
                                let limit_right = crate::coordinates::MapVec::new(v2.1, -v2.0);
                                let is_concave =
                                    crate::geo2d::cross(limit_left.to_geo(), limit_right.to_geo())
                                        < 0.0;
                                self.world
                                    .fast_grid_mut()
                                    .level_mut()
                                    .level_repulsive_points
                                    .push(crate::fast_find_grid::LevelRepulsivePoint {
                                        position: MapPoint::new(ox as f32, oy as f32),
                                        layer: layer_idx as u16,
                                        limit_left,
                                        limit_right,
                                        is_concave,
                                    });
                            }
                        }
                    }
                    obstacles.push(crate::pathfinder::MotionObstacle {
                        state_id: obstacle.state_id,
                        // Active by default; pathfinder::initialize() will
                        // flip this in line with the default state mask.
                        active: true,
                        bounding_box: bbox,
                        polygon: poly_pts,
                        grid_sector_index: Some(grid_sector_index),
                        grid_line_indices: line_indices,
                    });
                }

                // Store polygon vertices for point-in-area hit-testing.
                let polygon_pts: Vec<MapPoint> = area
                    .polygon
                    .points
                    .iter()
                    .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                    .collect();

                move_areas.push(crate::pathfinder::MotionArea {
                    skeleton,
                    polygon: polygon_pts,
                    motion_obstacles: obstacles,
                });
                alt_move_areas.push(crate::pathfinder::MotionArea {
                    skeleton: Vec::new(),
                    polygon: Vec::new(),
                    motion_obstacles: Vec::new(),
                });
            }

            let graph = std::sync::Arc::make_mut(&mut assets.pathfinder_graph);
            let static_data = graph.static_mut();
            static_data.move_layers.push(move_areas);
            static_data.alternative_move_layers.push(alt_move_areas);
        }

        // ── Part 2: Pathfinder graph ──
        if !motion_data.graph_bytes.is_empty()
            && let Err(e) = std::sync::Arc::make_mut(&mut assets.pathfinder_graph)
                .load_from_proto_stream(self.world.fast_grid_mut(), &motion_data.graph_bytes)
        {
            tracing::error!("Failed to load pathfinder graph: {e}");
        }
        if motion_data.graph_bytes.is_empty() {
            // Hackable descriptors omit the legacy precomputed graph. Keep
            // its hierarchy/state topology faithful to the authored motion
            // areas so PathFinder can apply obstacle state and use its
            // deterministic visibility-graph fallback for routing.
            let graph = std::sync::Arc::make_mut(&mut assets.pathfinder_graph);
            let shape: Vec<Vec<usize>> = graph
                .static_data
                .move_layers
                .iter()
                .map(|layer| {
                    layer
                        .iter()
                        .map(|area| area.motion_obstacles.len())
                        .collect()
                })
                .collect();
            graph.layers = shape
                .iter()
                .map(|layer| {
                    layer
                        .iter()
                        .map(|&obstacles| vec![Vec::new(); obstacles])
                        .collect()
                })
                .collect();
            graph.alternative_layers = graph.layers.clone();
            graph.states = shape.iter().map(|layer| vec![0; layer.len()]).collect();
        }

        // ── Part 3: Build sector conversion table ──
        std::sync::Arc::make_mut(&mut assets.pathfinder_graph).build_sector_conversion();

        // ── Part 4: Initialize pathfinder obstacle states ──
        // Must happen after graph is loaded, not during engine.initialize() which
        // runs before load_background_map processes the motion data.
        {
            let world = &mut self.world;
            let grid = std::sync::Arc::make_mut(&mut world.fast_grid);
            world
                .pathfinder
                .initialize_from_graph(assets.pathfinder_graph.as_ref(), grid);
        }

        // ── Part 5: Register sectors in grid blocks ──
        //
        // Each motion area polygon becomes a MOTION | AREA | MOUSE
        // sector in the grid, and each obstacle becomes MOTION (without
        // AREA).  This enables GetSector/GetSectorScreen spatial queries.
        {
            use crate::sector::SectorType;

            let mut sector_number = crate::sector::SectorNumber::new(0);
            let mut area_flat_idx: u16 = 0;
            let mut expected_motion_sector_idx =
                u32::try_from(self.world.fast_grid.level.sectors.len())
                    .expect("fast-grid sector count exceeds u32");

            for (layer_idx, layer_areas) in motion_data.layers.iter().enumerate() {
                for (area_idx, area) in layer_areas.iter().enumerate() {
                    // Register the walkable area polygon.
                    // ForceCrouched when flags != 0.
                    let force_crouched = area.flags != 0;
                    let mut area_type = SectorType::MOTION | SectorType::AREA | SectorType::MOUSE;
                    if area.is_lift {
                        area_type |= SectorType::LIFT;
                    }

                    let pts: Vec<_> = area
                        .polygon
                        .points
                        .iter()
                        .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                        .collect();
                    let mut bbox = MapBBox::new();
                    for &p in &pts {
                        bbox.expand_point(p);
                    }

                    let area_grid_idx = self.world.fast_grid_mut().add_sector(
                        crate::fast_find_grid::GridSector {
                            points: pts,
                            bounding_box: bbox,
                            sector_type: area_type,
                            layer: layer_idx as u16,
                            sector_number,
                            door_index: None,
                            lift_type: None,
                            lift_direction: 0,
                            force_crouched,
                            building_index: None,
                            low_exit_point: None,
                            high_exit_point: None,
                            lowest_door_index: None,
                            jump_line_indices: Vec::new(),
                            gate_indices: Vec::new(),
                            underlying_sector: None,
                        },
                        layer_idx as u16,
                    );
                    assert_eq!(
                        area_grid_idx, expected_motion_sector_idx,
                        "motion area sector registration drifted from pathfinder topology"
                    );
                    expected_motion_sector_idx = expected_motion_sector_idx
                        .checked_add(1)
                        .expect("motion sector index overflow");
                    sector_number += 1;
                    area_flat_idx += 1;

                    // Register each obstacle polygon (MOTION without AREA)
                    for (obstacle_idx, obstacle) in area.obstacles.iter().enumerate() {
                        let obs_pts: Vec<_> = obstacle
                            .polygon
                            .points
                            .iter()
                            .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                            .collect();
                        let mut obs_bbox = MapBBox::new();
                        for &p in &obs_pts {
                            obs_bbox.expand_point(p);
                        }

                        let obstacle_grid_idx = self.world.fast_grid_mut().add_sector(
                            crate::fast_find_grid::GridSector {
                                points: obs_pts,
                                bounding_box: obs_bbox,
                                sector_type: SectorType::MOTION,
                                layer: layer_idx as u16,
                                sector_number,
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
                            },
                            layer_idx as u16,
                        );
                        let expected = assets.pathfinder_graph.static_data.move_layers[layer_idx]
                            [area_idx]
                            .motion_obstacles[obstacle_idx]
                            .grid_sector_index
                            .expect("loaded motion obstacle has no fast-grid sector binding");
                        assert_eq!(
                            obstacle_grid_idx,
                            u32::from(expected),
                            "motion obstacle sector registration drifted from pathfinder topology"
                        );
                        assert_eq!(
                            obstacle_grid_idx, expected_motion_sector_idx,
                            "motion obstacle sector counter drifted from pathfinder topology"
                        );
                        expected_motion_sector_idx = expected_motion_sector_idx
                            .checked_add(1)
                            .expect("motion sector index overflow");
                        sector_number += 1;
                    }
                }
            }

            self.world.pathfinder.synchronize_motion_obstacle_sectors(
                assets.pathfinder_graph.as_ref(),
                std::sync::Arc::make_mut(&mut self.world.fast_grid),
            );

            // ── Apply lift_type from RawLift data to grid sectors ──
            //
            // The `uwSector` word in CHUNK_LIFT is a sector_number, not
            // a motion-area-flat index — obstacles interleave with areas
            // in the sector array, so the two indices diverge once any
            // motion area has obstacles.  The
            // `RawLift::motion_area_index` field name is a misnomer kept
            // for compatibility.
            //
            // Look up by sector_number directly and set lift_type on the
            // GridSector. legacy implementation reads the lift's associated click sector
            // (`RHSectorLift::InitializeFromProtoStream`) but the two
            // `AddSector(pSectorAssociated, ...)` calls are commented out,
            // so the proto click_sector is not a live mouse-pick sector.
            // The real lift motion polygon remains mouse-enabled from
            // CHUNK_MOTION and is the only clickable lift geometry.
            for lift in lifts {
                let sn = crate::sector::SectorNumber::new(lift.motion_area_index as i16);
                let Some(&lift_grid_idx) = self.world.fast_grid.level.sector_number_map.get(&sn)
                else {
                    continue;
                };
                {
                    let level = self.world.fast_grid_mut().level_mut();
                    if let Some(gs) = level.sectors.get_mut(lift_grid_idx) {
                        // The motion loader must already have marked the
                        // area as a lift.  Panic so malformed level data
                        // surfaces at load time (per project "No fake
                        // data" rule).
                        if !gs.sector_type.is_lift() {
                            panic!(
                                "Illegal lift sector: sector_number {} is not flagged as a lift — \
                                 CHUNK_LIFT references a sector that CHUNK_MOTION didn't mark is_lift=true",
                                i16::from(sn)
                            );
                        }
                        // Intercept `LiftType::Normal` (value 0), warn,
                        // and force `LiftType::Stairs`: default lifts
                        // are no longer supported, levels must use a
                        // simple motion area with "square"-doors. No
                        // live lift sector should end up as
                        // `LiftType::Normal`.
                        let mut raw_lift_type = crate::sector::LiftType::from_u8(lift.lift_type);
                        if raw_lift_type == crate::sector::LiftType::Normal {
                            tracing::warn!(
                                sector_number = i16::from(sn),
                                "default lifts are no longer supported, please use a simple \
                                 motion area with \"square\"-doors ! (forcing LIFT_STAIRS)"
                            );
                            raw_lift_type = crate::sector::LiftType::Stairs;
                        }
                        gs.lift_type = Some(raw_lift_type);
                        gs.lift_direction = lift.direction;
                        // `LIFT` bit is already set by the motion loader;
                        // OR again is a no-op but kept for symmetry.
                        gs.sector_type |= SectorType::LIFT;
                    }
                }
            }

            // ── Building sectors ──
            //
            // Every `Building` entry in the proto stream gets its own
            // sector with no polygon geometry, flagged
            // `MOTION | AREA | BUILDING`, living on the lift layer.
            // Doors inside the building have their `sector_in` rewritten
            // to point at the building sector, so a PC walking through
            // the door ends up with its sector pointer aimed at the
            // building sector.
            //
            // Sector number allocation and door `sector_in` rewrites both
            // already happened in `rewire_building_doors` during the initial
            // level load, so that the later MissionLevelBuilder
            // call sees the correct values.  Here we just walk the stashed
            // list and register the matching empty grid sectors — the
            // `debug_assert_eq!` catches any drift between the two passes.
            let building_lift_layer = self.world.fast_grid.lift_layer();
            let allocated = std::mem::take(&mut staging.motion.building_sector_numbers);
            for (bld_idx, sn) in allocated.iter().copied().enumerate() {
                let sn_wrapped = crate::sector::SectorNumber::new(sn);
                debug_assert_eq!(
                    sn_wrapped, sector_number,
                    "building sector allocation drifted from motion layout \
                     — `rewire_building_doors` and \
                     `initialize_motion_from_level_data` must agree on the \
                     area/obstacle count"
                );
                self.world.fast_grid_mut().add_sector(
                    crate::fast_find_grid::GridSector {
                        points: Vec::new(),
                        bounding_box: crate::coordinates::MapBBox::new(),
                        sector_type: SectorType::MOTION | SectorType::AREA | SectorType::BUILDING,
                        layer: building_lift_layer,
                        sector_number: sn_wrapped,
                        door_index: None,
                        lift_type: None,
                        lift_direction: 0,
                        force_crouched: false,
                        building_index: crate::sector::BuildingIdx::new(bld_idx as u16),
                        low_exit_point: None,
                        high_exit_point: None,
                        lowest_door_index: None,
                        jump_line_indices: Vec::new(),
                        gate_indices: Vec::new(),
                        underlying_sector: None,
                    },
                    building_lift_layer,
                );
                sector_number += 1;
            }

            // Original constructs one real `RHSectorMotionArea` for the
            // out-of-map side of reinforcement gates. It has no polygon and
            // exposes sector number -1, but it is still a distinct pointer
            // used by gate linking and PassDoor. Keep the same live arena
            // object instead of treating -1 as an absent/guessed endpoint.
            //
            // TODO(parity): retain its sparse `marraySectors` slot through
            // v48 position adoption too. Rust's public `SectorHandle` still
            // reserves 0xffff for None, so saved actors outside the map need
            // a dedicated public representation before that can be exposed.
            let special_layer = self.world.fast_grid.level.special_layer;
            self.world.fast_grid_mut().add_sector(
                crate::fast_find_grid::GridSector {
                    points: Vec::new(),
                    bounding_box: crate::coordinates::MapBBox::new(),
                    sector_type: SectorType::MOTION | SectorType::AREA,
                    layer: special_layer,
                    sector_number: crate::sector::SectorNumber::new(-1),
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
                },
                special_layer,
            );

            // ── Light / shadow sectors ──
            //
            // Each raw light sector becomes a `SectorType::SHADOW` grid
            // sector on its own layer iff its ambience bitmask overlaps
            // the mission's ambience.  Sectors whose ambience bit is
            // clear are dropped.  `is_in_shadow_sector` queries these to
            // suppress the fog/night sprite variant when an actor stands
            // inside a torch-lit polygon.
            let ambiance_mask = self.world.weather.ambiance.to_bitmask();
            let raw_light_sectors = std::mem::take(&mut staging.motion.light_sectors);
            let debug_view_radius_lights = crate::ai_vision::view_radius_cache_debug_enabled();
            if debug_view_radius_lights {
                eprintln!(
                    "VRLIGHT {{\"engine\":\"rust\",\"stage\":\"begin\",\"raw_count\":{},\"layer_count\":{},\"runtime_ambience_mask\":{ambiance_mask}}}",
                    raw_light_sectors.len(),
                    self.world.fast_grid.level.layers.len(),
                );
            }
            let mut light_added = 0usize;
            let mut light_skipped_ambience = 0usize;
            let mut light_skipped_layer = 0usize;
            let mut light_skipped_polygon = 0usize;
            for (raw_index, raw) in raw_light_sectors.into_iter().enumerate() {
                if (raw.ambience & ambiance_mask) == 0 {
                    if debug_view_radius_lights {
                        debug_view_radius_light(
                            raw_index,
                            &raw,
                            ambiance_mask,
                            "filtered_ambience",
                            None,
                        );
                    }
                    light_skipped_ambience += 1;
                    continue;
                }
                if (raw.layer as usize) >= self.world.fast_grid.level.layers.len() {
                    if debug_view_radius_lights {
                        debug_view_radius_light(
                            raw_index,
                            &raw,
                            ambiance_mask,
                            "filtered_layer",
                            None,
                        );
                    }
                    light_skipped_layer += 1;
                    continue;
                }
                if raw.polygon.points.len() < 3 {
                    if debug_view_radius_lights {
                        debug_view_radius_light(
                            raw_index,
                            &raw,
                            ambiance_mask,
                            "filtered_polygon",
                            None,
                        );
                    }
                    light_skipped_polygon += 1;
                    continue;
                }
                let pts: Vec<_> = raw
                    .polygon
                    .points
                    .iter()
                    .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                    .collect();
                let mut bbox = MapBBox::new();
                for &p in &pts {
                    bbox.expand_point(p);
                }
                let grid_index = self.world.fast_grid_mut().add_sector(
                    crate::fast_find_grid::GridSector {
                        points: pts,
                        bounding_box: bbox,
                        sector_type: SectorType::SHADOW,
                        layer: raw.layer,
                        sector_number,
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
                    },
                    raw.layer,
                );
                if debug_view_radius_lights {
                    debug_view_radius_light(
                        raw_index,
                        &raw,
                        ambiance_mask,
                        "registered",
                        Some((grid_index, sector_number)),
                    );
                }
                sector_number += 1;
                light_added += 1;
            }
            if light_added + light_skipped_ambience + light_skipped_layer + light_skipped_polygon
                > 0
            {
                tracing::debug!(
                    "Loaded {} shadow sectors ({} filtered by ambience, {} bad layer, {} degenerate polygon)",
                    light_added,
                    light_skipped_ambience,
                    light_skipped_layer,
                    light_skipped_polygon,
                );
            }

            // ── Shadow centroid + radius post-load (NIGHT/FOG only) ──
            //
            // For each SHADOW sector, when ambience is NIGHT or FOG,
            // compute the polygon centroid in 2D, look up the enclosing
            // `SECTOR_PLANE` to read its top-plane coefficients, and
            // derive the 3D centroid + average radius.
            //
            // Stored as a parallel `HashMap` keyed by `GridSector` index
            // so we don't bloat every (mostly non-shadow) GridSector with
            // an `Option<ShadowData>`.  Consumed by the night/fog branch
            // of `ai_vision::compute_view_radius`.
            let is_night_or_fog = matches!(
                self.world.weather.ambiance,
                crate::engine::types::Ambiance::Night | crate::engine::types::Ambiance::Fog
            );
            if is_night_or_fog && light_added > 0 {
                // Snapshot (idx, points, layer) without holding the
                // immutable borrow during the obstacle lookup pass.
                let shadow_inputs: Vec<(u32, Vec<MapPoint>, u16)> = self
                    .world
                    .fast_grid
                    .level
                    .sectors
                    .iter()
                    .enumerate()
                    .filter(|(_, gs)| gs.sector_type.is_shadow())
                    .map(|(i, gs)| (i as u32, gs.points.clone(), gs.layer))
                    .collect();
                for (sector_idx, points, layer) in shadow_inputs {
                    let mut shadow = crate::sector::ShadowData::default();
                    shadow.initialize_2d(&points);

                    // Inline projection-area lookup.  The Original creates
                    // every projection obstacle's SECTOR_PLANE on fast-grid
                    // layer 0 (`AddSector(pSectorPlane, 0, true)`), then asks
                    // for planes in the shadow sector's own layer.  Thus a
                    // nonzero-layer shadow cannot find a plane and retains
                    // the flat `(x, y, 0)` fallback.  Do not match obstacle
                    // and shadow layers here: on layer 0 the Original can
                    // see plane sectors belonging to projection obstacles
                    // from any obstacle layer.
                    let bary = shadow.barycentre_2d;
                    let mut found_top_plane: Option<[[f32; 3]; 3]> = None;
                    if layer == 0 {
                        for (oi, obs) in self.sight_obstacles(assets).iter_indexed() {
                            if !obs.is_projection_area() {
                                continue;
                            }
                            if !obs.box_projection.contains_point(bary) {
                                continue;
                            }
                            if !obs.contains_point_projection(bary) {
                                continue;
                            }
                            found_top_plane = Some(obs.top_plane_points);
                            let _ = oi; // index unused beyond verification
                            break;
                        }
                    }
                    shadow.initialize_3d(found_top_plane.as_ref());

                    self.world
                        .fast_grid_mut()
                        .level_mut()
                        .shadow_data
                        .insert(sector_idx, shadow);
                }
                tracing::debug!(
                    "Initialized shadow centroid data for {} sectors (NIGHT/FOG ambience)",
                    self.world.fast_grid.level.shadow_data.len(),
                );
            }

            tracing::info!(
                "Registered {} grid sectors ({} motion areas + obstacles, {} area-only)",
                self.world.fast_grid.level.sectors.len(),
                i16::from(sector_number),
                area_flat_idx,
            );
        }

        tracing::info!(
            "Motion initialized: {} layers, {} grid lines, {} path nodes, {} path links, {} pf sectors",
            motion_data.layers.len(),
            self.world.fast_grid.level.lines.len(),
            assets.pathfinder_graph.nodes.len(),
            assets.pathfinder_graph.static_data.links.len(),
            assets.pathfinder_graph.static_data.sector_conversion.len(),
        );

        // ── Jump zones + jump line pairs ──
        //
        // Must run after all motion-area sectors are registered so
        // `sector_number_map` lookups succeed for each jump zone's
        // sector number.
        self.load_jump_lines_from_proto(staging);
    }

    /// Minimum fall (negative jump height) before a jump line requires a
    /// helper.  A line's `jump_height = associated.z_a - this.z_a`, so a
    /// value below this threshold means the landing spot is at least 40
    /// units below the take-off.
    const JUMP_HEIGHT_HELPER_THRESHOLD: f32 = -40.0;

    /// Build runtime `JumpLine` entries from the stashed JZ/PPPP
    /// proto data and link them to their home sectors.
    ///
    /// Flow:
    /// 1. Read all jump zones (polygon + associated motion-area
    ///    sector number + layer).  Each line pair stores a
    ///    `jump_zone_index` pointing into this list.
    /// 2. Read each line pair.  For each pair `(line1, line2)`:
    ///    * Link them to each other.
    ///    * Give `line1` a home sector equal to `line2`'s jump
    ///      zone's sector (and symmetrically for `line2`).  This
    ///      places each line on the sector that its paired line
    ///      jumps *into*.
    ///    * Add `line1` to that sector's `jump_line_indices`
    ///      (and `line2` to the other).
    ///
    /// We skip jump-sector registration (polygon sectors for
    /// landing-spot lookup) because the table-swordfight path only
    /// needs the line endpoints and the sector linkage.
    pub(crate) fn load_jump_lines_from_proto(&mut self, staging: &mut LevelLoadStaging) {
        let jump_zones = std::mem::take(&mut staging.motion.jump_zones);
        let line_pairs = std::mem::take(&mut staging.motion.jump_line_pairs);
        if line_pairs.is_empty() {
            return;
        }

        // Resolve each jump zone's sector number to a grid-sector
        // index.  `RawJumpZone.sector` stores a sector number which
        // maps via `sector_number_map` to the flat sector index.
        let zone_sector: Vec<Option<u32>> = jump_zones
            .iter()
            .map(|z| {
                self.world
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&crate::sector::SectorNumber::new(z.sector as i16))
                    .map(|&idx| idx as u32)
            })
            .collect();
        let zone_layer: Vec<u16> = jump_zones.iter().map(|z| z.layer).collect();

        let num_zones = jump_zones.len();
        let mut loaded_pairs = 0usize;
        // Lines stored on each proto jump zone, matching
        // `RHLineJump::InitializeFromProtoStream` adding the line to
        // `RHSectorJump::mapJumpLines`. The line's motion-sector
        // association is assigned separately below from the paired
        // zone, like `RHFastFindGrid::InitializeMotion`.
        let mut zone_jump_lines: Vec<Vec<crate::jump_line::JumpLineIndex>> =
            vec![Vec::new(); num_zones];

        for pair in line_pairs {
            let z1 = pair.line1.jump_zone_index as usize;
            let z2 = pair.line2.jump_zone_index as usize;
            if z1 >= num_zones || z2 >= num_zones {
                tracing::warn!(
                    "Jump line pair references invalid zone index ({z1}, {z2}) / {num_zones}"
                );
                continue;
            }
            // Each line's home is its *paired* line's jump zone.
            let Some(sec1) = zone_sector[z2] else {
                tracing::warn!(
                    "Jump line pair zone {z2} has unresolved sector ({})",
                    jump_zones[z2].sector
                );
                continue;
            };
            let Some(sec2) = zone_sector[z1] else {
                tracing::warn!(
                    "Jump line pair zone {z1} has unresolved sector ({})",
                    jump_zones[z1].sector
                );
                continue;
            };
            let layer1 = zone_layer[z2];
            let layer2 = zone_layer[z1];

            let mut jl1 = crate::jump_line::JumpLine::new(
                crate::coordinates::map_pt(
                    pair.line1.point_a.0 as f32,
                    pair.line1.point_a.1 as f32,
                ),
                crate::coordinates::map_pt(
                    pair.line1.point_b.0 as f32,
                    pair.line1.point_b.1 as f32,
                ),
                pair.line1.point_a.2 as f32,
                pair.line1.point_b.2 as f32,
            );
            jl1.layer = layer1;
            jl1.sector_index = crate::fast_find_grid::SectorIndex::new(sec1);
            jl1.long_jump_forced = pair.jump_long;

            let mut jl2 = crate::jump_line::JumpLine::new(
                crate::coordinates::map_pt(
                    pair.line2.point_a.0 as f32,
                    pair.line2.point_a.1 as f32,
                ),
                crate::coordinates::map_pt(
                    pair.line2.point_b.0 as f32,
                    pair.line2.point_b.1 as f32,
                ),
                pair.line2.point_a.2 as f32,
                pair.line2.point_b.2 as f32,
            );
            jl2.layer = layer2;
            jl2.sector_index = crate::fast_find_grid::SectorIndex::new(sec2);
            jl2.long_jump_forced = pair.jump_long;

            // A line only requires a helper when *both* the drop from
            // its paired line exceeds `JUMP_HEIGHT_HELPER_THRESHOLD` (the
            // paired line is at least 40 units *below* the line's own
            // elevation) *and* either the line's own jump zone or the
            // paired zone has `helper_needed` set.  Using the zone's
            // flag directly forces a helper for shallow drops and misses
            // deep drops where only the paired zone is flagged.
            let either_zone_helper = jump_zones[z1].helper_needed || jump_zones[z2].helper_needed;
            // For each line, `jump_height = associated.z_a - this.z_a`.
            let jh1 = jl2.z_a - jl1.z_a;
            let jh2 = jl1.z_a - jl2.z_a;
            jl1.helper_needed = jh1 < Self::JUMP_HEIGHT_HELPER_THRESHOLD && either_zone_helper;
            jl2.helper_needed = jh2 < Self::JUMP_HEIGHT_HELPER_THRESHOLD && either_zone_helper;

            // Push both lines and cross-link their associated indices.
            let idx1 = self.world.fast_grid.level.jump_lines.len() as u32;
            let idx2 = idx1 + 1;
            jl1.associated_line_index = Some(idx2);
            jl2.associated_line_index = Some(idx1);
            if let Some(idx) = crate::jump_line::JumpLineIndex::new(idx1) {
                zone_jump_lines[z1].push(idx);
            }
            if let Some(idx) = crate::jump_line::JumpLineIndex::new(idx2) {
                zone_jump_lines[z2].push(idx);
            }
            // Remember the line geometry we need for the jump gate
            // below; we have to clone before moving the lines into
            // `fast_grid.level.jump_lines`.
            let jl1_mid = jl1.get_middle_point();
            let jl2_mid = jl2.get_middle_point();
            let jl1_layer = jl1.layer;
            let jl2_layer = jl2.layer;
            let jl1_helper_needed = jl1.helper_needed;
            let jl2_helper_needed = jl2.helper_needed;
            {
                let level = self.world.fast_grid_mut().level_mut();
                level.jump_lines.push(jl1);
                level.jump_lines.push(jl2);

                // Register line indices on their home sectors so
                // `GetNearestJumpLine` can iterate without a global scan.
                if let Some(gs) = level.sectors.get_mut(sec1 as usize)
                    && let Some(idx) = crate::jump_line::JumpLineIndex::new(idx1)
                {
                    gs.jump_line_indices.push(idx);
                }
                if let Some(gs) = level.sectors.get_mut(sec2 as usize)
                    && let Some(idx) = crate::jump_line::JumpLineIndex::new(idx2)
                {
                    gs.jump_line_indices.push(idx);
                }
            }

            // Resolve the sectors' `sector_number` so the jump-gate Door
            // can reference them by the same IDs the rest of the door
            // table uses (sector_out / sector_in are sector numbers, not
            // grid-flat indices).
            let sector_num_out = self
                .world
                .fast_grid
                .level
                .sectors
                .get(sec2 as usize)
                .map(|s| s.sector_number);
            let sector_num_in = self
                .world
                .fast_grid
                .level
                .sectors
                .get(sec1 as usize)
                .map(|s| s.sector_number);

            // Stash the jump-gate Door spec for later push into
            // `self.script_domains.interactables.doors`: compute the midpoint of each line as
            // the in/out point and use each line's home sector as the
            // in/out sector.
            //
            // We can't push directly here: the proto-stream phase
            // motion stage now runs before script loading and the
            // MissionLevelBuilder
            // so that beam-me / soldier sector-motion-area validations
            // see a populated grid (PROTO → MISSION load order).
            // `attach_jump_gates` drains this stash + rebuilds
            // gate links once the canonical mission domains exist.
            if let (Some(num_out), Some(num_in)) = (sector_num_out, sector_num_in)
                && num_out.is_valid()
                && num_in.is_valid()
            {
                // Penalty: `||pt_in - pt_out|| + PENALTY_JUMP`.
                let pdx = jl1_mid.x - jl2_mid.x;
                let pdy = jl1_mid.y - jl2_mid.y;
                let penalty = (pdx * pdx + pdy * pdy).sqrt() + crate::gate::PENALTY_JUMP;

                staging.attachments.jump_gates.push(JumpGateAttachment {
                    point_out: jl2_mid,
                    point_in: jl1_mid,
                    layer_out: jl2_layer,
                    layer_in: jl1_layer,
                    sector_out: num_out,
                    sector_in: num_in,
                    sector_out_index: crate::fast_find_grid::SectorIndex::new(sec2)
                        .expect("jump gate outside sector index equals null sentinel"),
                    sector_in_index: crate::fast_find_grid::SectorIndex::new(sec1)
                        .expect("jump gate inside sector index equals null sentinel"),
                    jump_line_out: idx2,
                    jump_line_in: idx1,
                    // Cache each destination line's `helper_needed`
                    // flag so `Door::is_actor_authorized` can answer
                    // its destination-line branch without reading
                    // back into `fast_grid`.
                    jump_line_in_helper_needed: jl1_helper_needed,
                    jump_line_out_helper_needed: jl2_helper_needed,
                    penalty,
                });
            } else {
                tracing::warn!(
                    "Jump line pair ({z1}/{z2}) failed to resolve sector numbers; \
                     skipping jump-gate registration"
                );
            }

            loaded_pairs += 1;
        }

        if loaded_pairs > 0 {
            tracing::debug!(
                "Loaded {} jump line pair(s) into fast grid ({} jump lines total)",
                loaded_pairs,
                self.world.fast_grid.level.jump_lines.len(),
            );
        }

        // ── Jump sector registration ──
        // Each jump zone becomes a `MOUSE | JUMP` grid sector so cursor
        // hit-tests can land on them; the `underlying_sector` link lets
        // `update_mouse` recurse into the motion area beneath when no
        // jump line resolves.
        let mut registered = 0usize;
        for (zi, zone) in jump_zones.iter().enumerate() {
            // Log instead of aborting when a zone has no jump line —
            // the zone still gets registered below so cursor hit-tests
            // are consistent, but the mismatch is loud enough to catch
            // authoring errors.
            if zone_jump_lines[zi].is_empty() {
                tracing::error!(
                    "Jump zone {} has no jump line referencing it (uwSector={}, layer={})",
                    zi,
                    zone.sector,
                    zone.layer,
                );
            }
            if zone.polygon.points.is_empty() {
                continue;
            }
            let points: Vec<MapPoint> = zone
                .polygon
                .points
                .iter()
                .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                .collect();
            let mut bbox = MapBBox::new();
            for &p in &points {
                bbox.expand_point(p);
            }
            let gs = crate::fast_find_grid::GridSector {
                points,
                bounding_box: bbox,
                sector_type: crate::sector::SectorType::MOUSE | crate::sector::SectorType::JUMP,
                layer: zone.layer,
                sector_number: crate::sector::SectorNumber::new(-1),
                door_index: None,
                lift_type: None,
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: zone_jump_lines[zi].clone(),
                gate_indices: Vec::new(),
                underlying_sector: zone_sector[zi]
                    .and_then(crate::fast_find_grid::SectorIndex::new),
            };
            self.world.fast_grid_mut().add_sector(gs, zone.layer);
            registered += 1;
        }
        if registered > 0 {
            tracing::debug!("Registered {} jump sectors in fast grid", registered);
        }
    }

    /// Attach staged jump-gate specs as
    /// `Door` (gate_type=Jump) into `self.script_domains.interactables.doors`, then rebuild
    /// gate-link connectivity.  Must run after
    /// the authored door/lift stages so the canonical door table exists, and after
    /// every other proto/mission door has been registered so the
    /// gate-link rebuild sees the complete door table in one pass.
    pub(crate) fn attach_jump_gates(
        &mut self,
        staging: &mut LevelLoadStaging,
    ) -> Result<(), MissionLevelBuildError> {
        let specs = std::mem::take(&mut staging.attachments.jump_gates);
        if specs.is_empty() {
            return Ok(());
        }
        let count = specs.len();
        for spec in specs {
            self.script_domains
                .interactables
                .doors
                .push(crate::gate::Door {
                    gate_type: crate::gate::GateType::Jump,
                    door_type: crate::gate::DoorType::Default,
                    point_out: spec.point_out,
                    point_in: spec.point_in,
                    point_mid: MapPoint::new(
                        (spec.point_in.x + spec.point_out.x) * 0.5,
                        (spec.point_in.y + spec.point_out.y) * 0.5,
                    ),
                    layer_out: spec.layer_out,
                    layer_in: spec.layer_in,
                    sector_out: spec.sector_out,
                    sector_in: spec.sector_in,
                    sector_out_index: Some(spec.sector_out_index),
                    sector_in_index: Some(spec.sector_in_index),
                    jump_line_out: Some(spec.jump_line_out),
                    jump_line_in: Some(spec.jump_line_in),
                    jump_line_in_helper_needed: spec.jump_line_in_helper_needed,
                    jump_line_out_helper_needed: spec.jump_line_out_helper_needed,
                    penalty: spec.penalty,
                    ..Default::default()
                });
        }
        crate::gate::build_gate_links(&mut self.script_domains.interactables.doors);
        tracing::debug!(
            "Registered {count} jump-gate Door(s) into self.script_domains.interactables.doors"
        );
        Ok(())
    }

    /// Resolve each door's two endpoint sector numbers to their grid
    /// sectors and record the door index in each sector's
    /// `gate_indices`.
    ///
    /// Must run after `initialize_motion_from_level_data`, which is what
    /// populates `sector_number_map`.  Doors themselves are loaded earlier
    /// by the MissionLevelBuilder door and lift stages.
    pub(crate) fn populate_sector_gates_from_doors(&mut self) {
        let door_count = self.script_domains.interactables.doors.len();
        if door_count == 0 {
            return;
        }

        // Runtime-authored gates (notably reinforcement gates) do not pass
        // through the proto sparse-slot resolver. They may recover an exact
        // endpoint only when the public number names exactly one live motion
        // area; ambiguity is an error, never a guessed pointer.
        let level = self.world.fast_grid.level.clone();
        let unique_runtime_sector = |number: crate::sector::SectorNumber| {
            let mut matches = level
                .sectors
                .iter()
                .enumerate()
                .filter(|(_, sector)| {
                    sector.sector_number == number
                        && sector.sector_type.is_motion()
                        && sector.sector_type.is_area()
                })
                .map(|(index, _)| {
                    crate::fast_find_grid::SectorIndex::new(index as u32)
                        .expect("runtime sector index equals null sentinel")
                });
            let found = matches.next().unwrap_or_else(|| {
                panic!("door endpoint references unknown runtime sector {number}")
            });
            assert!(
                matches.next().is_none(),
                "door endpoint sector {number} is ambiguous without retained arena identity"
            );
            found
        };
        for door in &mut self.script_domains.interactables.doors {
            if door.sector_out_index.is_none() {
                door.sector_out_index = Some(unique_runtime_sector(door.sector_out));
            }
            if door.sector_in_index.is_none() {
                door.sector_in_index = Some(unique_runtime_sector(door.sector_in));
            }
        }
        crate::gate::build_gate_links(&mut self.script_domains.interactables.doors);

        self.world.fast_grid_mut().level_mut().door_projection_infos = self
            .script_domains
            .interactables
            .doors
            .iter()
            .map(|door| crate::fast_find_grid::DoorProjectionInfo {
                point_in: door.point_in,
                point_out: door.point_out,
                sector_out: door.sector_out,
                sector_out_index: door.sector_out_index,
                layer_out: door.layer_out,
            })
            .collect();

        // Snapshot door endpoints so the grid can be borrowed mutably below.
        let endpoints: Vec<(u32, crate::fast_find_grid::SectorIndex)> = self
            .script_domains
            .interactables
            .doors
            .iter()
            .enumerate()
            .flat_map(|(idx, door)| {
                [
                    (
                        idx as u32,
                        door.sector_out_index
                            .expect("resolved door has no outside arena identity"),
                    ),
                    (
                        idx as u32,
                        door.sector_in_index
                            .expect("resolved door has no inside arena identity"),
                    ),
                ]
            })
            .collect();

        for (door_idx, sector_index) in &endpoints {
            let grid_idx = usize::from(*sector_index);
            let level = self.world.fast_grid_mut().level_mut();
            if let Some(gs) = level.sectors.get_mut(grid_idx)
                && gs.sector_type.is_motion()
                && gs.sector_type.is_area()
            {
                gs.gate_indices
                    .push(crate::gate::DoorIndex::from(*door_idx));
            }
        }
    }

    /// Cache the world-derived door metadata consumed by AI queries.
    ///
    /// These caches belong to the constructed level, not to the mission VM,
    /// so they must also be populated when scripting is disabled.
    fn cache_door_ai_metadata(&mut self) {
        self.ai.global.door_seek_infos = self
            .script_domains
            .interactables
            .doors
            .iter()
            .enumerate()
            .map(|(idx, door)| {
                // Cache the actor-independent portion of the exact
                // authorization used by FindDoorEnemyCouldBeBehind.
                // Live building capacity and rider state are applied
                // when the seek helper consumes this snapshot.
                let npc_villain_authorized_direct =
                    crate::ai::cache_npc_villain_authorized_direct(door);
                let sector_in_index = door.sector_in_index.unwrap_or_else(|| {
                    panic!("canonical door {idx} interior sector has no exact arena identity")
                });
                let sector_out_index = door.sector_out_index.unwrap_or_else(|| {
                    panic!("canonical door {idx} exterior sector has no exact arena identity")
                });
                crate::ai::DoorSeekInfo {
                    door_index: crate::gate::DoorIndex(idx as u32),
                    door_type: door.door_type,
                    point_out: door.point_out,
                    position_in: crate::ai::Position {
                        x: door.point_in.x,
                        y: door.point_in.y,
                        sector: crate::position_interface::SectorHandle::new(u16::from(
                            door.sector_in,
                        ))
                        .map(|handle| handle.with_arena_index(sector_in_index)),
                        level: door.layer_in,
                    },
                    sector_out: u16::from(door.sector_out),
                    sector_out_index: Some(sector_out_index),
                    sector_in: u16::from(door.sector_in),
                    layer_out: door.layer_out,
                    npc_villain_authorized_direct,
                }
            })
            .collect();
        tracing::debug!(
            "Cached {} door seek infos for FindDoorEnemyCouldBeBehind",
            self.ai.global.door_seek_infos.len(),
        );

        // Populate reinforcement door info for MerryManForestCassos.
        self.ai.global.reinforcement_doors = self
            .script_domains
            .interactables
            .doors
            .iter()
            .enumerate()
            .filter(|(_, door)| door.door_type == crate::gate::DoorType::Reinforcement)
            .map(|(idx, door)| {
                let sector_in_index = door.sector_in_index.unwrap_or_else(|| {
                    panic!("reinforcement door {idx} interior sector has no exact arena identity")
                });
                let sector_out_index = door.sector_out_index.unwrap_or_else(|| {
                    panic!("reinforcement door {idx} exterior sector has no exact arena identity")
                });
                crate::ai::ReinforcementDoorInfo {
                    position_in: crate::ai::Position {
                        x: door.point_in.x,
                        y: door.point_in.y,
                        sector: crate::position_interface::SectorHandle::new(u16::from(
                            door.sector_in,
                        ))
                        .map(|handle| handle.with_arena_index(sector_in_index)),
                        level: door.layer_in,
                    },
                    door_index: crate::gate::DoorIndex(idx as u32),
                    point_out: door.point_out,
                    point_in: door.point_in,
                    point_mid: door.point_mid,
                    layer_out: door.layer_out,
                    sector_out: crate::position_interface::SectorHandle::new(u16::from(
                        door.sector_out,
                    ))
                    .map(|handle| handle.with_arena_index(sector_out_index)),
                }
            })
            .collect();
        tracing::debug!(
            "Cached {} reinforcement doors for MerryManForestCassos",
            self.ai.global.reinforcement_doors.len(),
        );
    }

    // ─── Loaded level → canonical script domains ────────────────────────

    /// Resolve an Original `marraySectors` slot to the paired public sector
    /// number and exact live arena object. Door stream endpoints use this
    /// sparse pointer space (`RHGate.cpp::InitializeFromProtoStream`).
    fn resolve_sparse_position_sector(
        assets: &LevelAssets,
        sparse_slot: u16,
    ) -> (
        crate::sector::SectorNumber,
        crate::fast_find_grid::SectorIndex,
    ) {
        let retained = assets
            .legacy_grid_topology
            .as_ref()
            .expect("door endpoint resolution requires retained Original sector topology");
        let slot = usize::from(sparse_slot);
        let number = retained
            .position_sector_numbers
            .get(slot)
            .copied()
            .flatten()
            .unwrap_or_else(|| {
                panic!("door endpoint references unmappable Original sector slot {sparse_slot}")
            });
        let index = retained
            .position_sector_indices
            .get(slot)
            .copied()
            .flatten()
            .unwrap_or_else(|| {
                panic!("door endpoint slot {sparse_slot} has no exact runtime sector identity")
            });
        (crate::sector::SectorNumber::new(number), index)
    }

    /// Resolve an Original sparse sector slot into the exact `RHposition`
    /// sector handle carried by tactic points and other serialized positions.
    fn resolve_sparse_position_handle(
        assets: &LevelAssets,
        sparse_slot: u16,
    ) -> crate::position_interface::SectorHandle {
        let (number, index) = Self::resolve_sparse_position_sector(assets, sparse_slot);
        let public = u16::try_from(number.get()).unwrap_or_else(|_| {
            panic!(
                "sparse position sector slot {sparse_slot} resolves to invalid public sector {}",
                number.get()
            )
        });
        crate::position_interface::SectorHandle::new(public)
            .unwrap_or_else(|| {
                panic!(
                    "sparse position sector slot {sparse_slot} resolves to the null sector sentinel"
                )
            })
            .with_arena_index(index)
    }

    /// Building loading replaces a door's stream-read inside pointer with
    /// the newly constructed building sector before the Rust door stage.
    /// Resolve that compact number only among retained building objects and
    /// require one exact arena identity.
    fn resolve_building_position_sector(
        assets: &LevelAssets,
        runtime_number: u16,
    ) -> (
        crate::sector::SectorNumber,
        crate::fast_find_grid::SectorIndex,
    ) {
        let retained = assets
            .legacy_grid_topology
            .as_ref()
            .expect("building door resolution requires retained Original sector topology");
        let wanted = i16::try_from(runtime_number)
            .expect("building runtime sector number exceeds Original signed-word domain");
        let mut matches = retained
            .sectors
            .iter()
            .zip(&retained.position_sector_numbers)
            .zip(&retained.position_sector_indices)
            .filter_map(|((kind, number), index)| {
                (matches!(kind, crate::engine::LegacyGridSectorAsset::Building)
                    && *number == Some(wanted))
                .then_some(*index)
                .flatten()
            });
        let index = matches.next().unwrap_or_else(|| {
            panic!("building door references unmappable runtime sector {runtime_number}")
        });
        assert!(
            matches.next().is_none(),
            "building runtime sector {runtime_number} resolves to multiple arena objects"
        );
        (crate::sector::SectorNumber::new(wanted), index)
    }

    /// Apply the validated door, lift, patch, and building stage outputs in
    /// original authored order.
    fn build_mission_level_stages(
        &mut self,
        assets: &LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
        stages: &MissionLevelBuildPlan,
    ) -> Result<(), MissionLevelBuildError> {
        self.build_door_stage(assets, loaded, stages);
        self.build_lift_stage(assets, loaded);
        self.build_door_lift_attachment_stage()?;
        self.build_patch_stage(assets, loaded, stages.patch_count);
        self.build_building_stage(stages);
        Ok(())
    }

    fn build_door_stage(
        &mut self,
        assets: &LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
        stages: &MissionLevelBuildPlan,
    ) {
        // ── Doors ──
        // Collect every RawDoor from buildings / standalone-door entries.
        //
        // For `Building` entries we also record the resulting door handles
        // in `building_gates[bld_idx]` so script natives (PutActorInBuilding,
        // SetBuildingActive) can find the first gate of a given building.
        // The building's gates are exactly the doors declared inside its
        // proto entry.
        for entry in &loaded.proto.buildings {
            let building_entry =
                matches!(entry, crate::level_data::RawBuildingEntry::Building { .. });
            let raw_doors = match entry {
                crate::level_data::RawBuildingEntry::Building { doors }
                | crate::level_data::RawBuildingEntry::StandaloneDoors { doors } => doors,
            };
            for raw in raw_doors {
                let (sector_out, sector_out_index) =
                    Self::resolve_sparse_position_sector(assets, raw.sector_out);
                let (sector_in, sector_in_index) = if building_entry {
                    Self::resolve_building_position_sector(assets, raw.sector_in)
                } else {
                    Self::resolve_sparse_position_sector(assets, raw.sector_in)
                };
                let door_type = match raw.door_type {
                    1 => crate::gate::DoorType::Building,
                    2 => crate::gate::DoorType::BuildingTrap,
                    3 => crate::gate::DoorType::Gate,
                    4 => crate::gate::DoorType::LiftHigh,
                    5 => crate::gate::DoorType::LiftLow,
                    6 => crate::gate::DoorType::LiftHighCrenel,
                    7 => crate::gate::DoorType::Trap,
                    8 => crate::gate::DoorType::Reinforcement,
                    _ => crate::gate::DoorType::Default,
                };
                let (act_d1, act_d2, act_i1, act_i2) =
                    crate::gate::Door::default_actions_for_type(door_type);
                self.script_domains
                    .interactables
                    .doors
                    .push(crate::gate::Door {
                        gate_type: crate::gate::GateType::Door,
                        active: raw.active,
                        door_type,
                        locked_pc: raw.locked_pc,
                        locked_npc_villain: raw.locked_npc_villain,
                        locked_npc_civilian: raw.locked_npc_civilian,
                        unlockable: raw.unlockable,
                        locked_pc_after_patch: raw.locked_pc_after_patch,
                        locked_npc_villain_after_patch: raw.locked_npc_villain_after_patch,
                        locked_npc_civilian_after_patch: raw.locked_npc_civilian_after_patch,
                        unlockable_after_patch: raw.unlockable_after_patch,
                        special_authorisation_pc: false,
                        authorised_pc_direct: 0,
                        authorised_pc_indirect: 0,
                        point_out: MapPoint::new(raw.point_out.0 as f32, raw.point_out.1 as f32),
                        point_in: MapPoint::new(raw.point_in.0 as f32, raw.point_in.1 as f32),
                        point_mid: MapPoint::new(raw.point_mid.0 as f32, raw.point_mid.1 as f32),
                        layer_out: raw.layer_out,
                        layer_in: raw.layer_in,
                        sector_out,
                        sector_in,
                        sector_out_index: Some(sector_out_index),
                        sector_in_index: Some(sector_in_index),
                        owning_lift_sector: None,
                        gate_links: Vec::new(),
                        click_polygon: raw
                            .door_sector
                            .points
                            .iter()
                            .map(|&(x, y)| (x as f32, y as f32))
                            .collect(),
                        click_bbox: crate::coordinates::MapBBox::new(),
                        penalty: 0.0,
                        patch_index: None,
                        gate_state: crate::gate::GateState::default(),
                        jump_line_out: None,
                        jump_line_in: None,
                        jump_line_in_helper_needed: false,
                        jump_line_out_helper_needed: false,
                        action_direct_1: act_d1,
                        action_direct_2: act_d2,
                        action_indirect_1: act_i1,
                        action_indirect_2: act_i2,
                    });
                if let Some(door) = self.script_domains.interactables.doors.last_mut() {
                    // Apply the `adapt_points` shift before computing the
                    // penalty: building-trap / wall-lift entries have their
                    // `point_in` offset from `point_mid`.  Non-lift building
                    // / standalone doors never hit a wall-lift branch, so
                    // `lift_wall = false` is correct.
                    door.adapt_points(false);
                    // `penalty = |point_in - point_out| + PENALTY_{BUILDING|DEFAULT}`.
                    // Must run after `adapt_points`.
                    door.compute_door_penalty();
                    door.rebuild_click_bbox();
                }
            }
        }
        self.script_domains.buildings.gates = stages.building_gates.clone();
    }

    fn build_lift_stage(&mut self, assets: &LevelAssets, loaded: &crate::level_data::LoadedLevel) {
        // Also collect doors from lifts.
        for lift in &loaded.proto.lifts {
            // `adapt_points` guards its LiftHigh / LiftHighCrenel arms
            // on whether the host lift is a wall lift. We already know
            // the hosting lift's type from the proto stream, so we
            // forward that bit to the Door method rather than looking it
            // up from the grid later.
            let lift_wall =
                crate::sector::LiftType::from_u8(lift.lift_type) == crate::sector::LiftType::Wall;
            for raw in &lift.doors {
                let (sector_out, sector_out_index) =
                    Self::resolve_sparse_position_sector(assets, raw.sector_out);
                let (sector_in, sector_in_index) =
                    Self::resolve_sparse_position_sector(assets, raw.sector_in);
                let door_type = match raw.door_type {
                    1 => crate::gate::DoorType::Building,
                    4 => crate::gate::DoorType::LiftHigh,
                    5 => crate::gate::DoorType::LiftLow,
                    6 => crate::gate::DoorType::LiftHighCrenel,
                    _ => crate::gate::DoorType::Default,
                };
                let (act_d1, act_d2, act_i1, act_i2) =
                    crate::gate::Door::default_actions_for_type(door_type);
                self.script_domains
                    .interactables
                    .doors
                    .push(crate::gate::Door {
                        gate_type: crate::gate::GateType::Door,
                        active: raw.active,
                        door_type,
                        locked_pc: raw.locked_pc,
                        locked_npc_villain: raw.locked_npc_villain,
                        locked_npc_civilian: raw.locked_npc_civilian,
                        unlockable: raw.unlockable,
                        locked_pc_after_patch: raw.locked_pc_after_patch,
                        locked_npc_villain_after_patch: raw.locked_npc_villain_after_patch,
                        locked_npc_civilian_after_patch: raw.locked_npc_civilian_after_patch,
                        unlockable_after_patch: raw.unlockable_after_patch,
                        point_out: MapPoint::new(raw.point_out.0 as f32, raw.point_out.1 as f32),
                        point_in: MapPoint::new(raw.point_in.0 as f32, raw.point_in.1 as f32),
                        point_mid: MapPoint::new(raw.point_mid.0 as f32, raw.point_mid.1 as f32),
                        layer_out: raw.layer_out,
                        layer_in: raw.layer_in,
                        sector_out,
                        sector_in,
                        sector_out_index: Some(sector_out_index),
                        sector_in_index: Some(sector_in_index),
                        // `RHSectorLift` owns every door embedded in this
                        // CHUNK_LIFT record. The owning sector is the chunk's
                        // `uwSector` (`motion_area_index` here), not
                        // necessarily the door's `sector_in`: wall-lift data
                        // can route an embedded door through a distinct
                        // associated sector.
                        owning_lift_sector: Some(crate::sector::SectorNumber::new(
                            lift.motion_area_index as i16,
                        )),
                        click_polygon: raw
                            .door_sector
                            .points
                            .iter()
                            .map(|&(x, y)| (x as f32, y as f32))
                            .collect(),
                        click_bbox: crate::coordinates::MapBBox::new(),
                        action_direct_1: act_d1,
                        action_direct_2: act_d2,
                        action_indirect_1: act_i1,
                        action_indirect_2: act_i2,
                        ..Default::default()
                    });
                if let Some(door) = self.script_domains.interactables.doors.last_mut() {
                    // Order: `adapt_points` then penalty.  LiftHigh /
                    // LiftHighCrenel doors on wall lifts get their
                    // `point_in` nudged toward `point_mid`; other lift
                    // types leave `point_in` alone.
                    door.adapt_points(lift_wall);
                    door.compute_door_penalty();
                    door.rebuild_click_bbox();
                }
            }
        }
    }

    fn build_door_lift_attachment_stage(&mut self) -> Result<(), MissionLevelBuildError> {
        // Build gate links: connect doors that share a sector.
        // Jump gates are appended later by `load_jump_lines_from_proto`,
        // which re-invokes `build_gate_links` to cover them too.
        crate::gate::build_gate_links(&mut self.script_domains.interactables.doors);
        let total_links: usize = self
            .script_domains
            .interactables
            .doors
            .iter()
            .map(|d| d.gate_links.len())
            .sum();
        tracing::info!(
            "Built gate connectivity graph: {} doors, {} links",
            self.script_domains.interactables.doors.len(),
            total_links,
        );

        // Note: motion-area sector `gate_indices` are populated later, in
        // `populate_sector_gates_from_doors`, which runs after
        // `initialize_motion_from_level_data` has registered the sectors
        // in `sector_number_map`.

        // Register door click polygons as grid sectors (DOOR | MOUSE)
        // so GetSector/GetSectorScreen can find them during click hit-testing.
        {
            use crate::sector::SectorType;
            // Register the door's clickable polygon on
            // `max(layer_out, layer_in)`, but exclude the grid's
            // `special_layer` from the bump — keeping the door reachable
            // from the higher of the two real layers without accidentally
            // landing it on the out-of-map layer.
            let special_layer = self.world.fast_grid.level.special_layer;
            let mut door_sectors_registered = 0u32;
            for (door_idx, door) in self.script_domains.interactables.doors.iter().enumerate() {
                if door.click_polygon.is_empty() {
                    continue;
                }
                let pts: Vec<_> = door
                    .click_polygon
                    .iter()
                    .map(|&(x, y)| MapPoint::new(x, y))
                    .collect();
                let bbox = door.click_bbox;
                // Start with layer_out; bump to layer_in iff it's
                // strictly higher AND not the special (out-of-map) layer.
                let mut layer = door.layer_out;
                if door.layer_in > layer && door.layer_in != special_layer {
                    layer = door.layer_in;
                }

                let door_active = door.active;
                let idx = self.world.fast_grid_mut().add_sector(crate::fast_find_grid::GridSector { points: pts,
                bounding_box: bbox,
                sector_type: SectorType::DOOR | SectorType::MOUSE,
                layer,
                sector_number: crate::sector::SectorNumber::new(-1), /* Doors don't have motion sector numbers */
                door_index: Some(door_idx as u32),
                lift_type: None,
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None, jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
                                },
                layer,);
                self.world
                    .fast_grid_mut()
                    .set_sector_active(idx, door_active);
                door_sectors_registered += 1;
            }
            tracing::info!(
                "Registered {} door click sectors in grid",
                door_sectors_registered,
            );
        }

        tracing::info!(
            "Populated {} canonical doors from level data",
            self.script_domains.interactables.doors.len(),
        );

        // The legacy implementation lift sectors expose high/low exit points for
        // DetermineMovementAnimation. Populate the Rust cache after the
        // The canonical door table is fully loaded; an earlier motion-sector pass
        // may run before these doors exist.
        for gs in &mut self.world.fast_grid_mut().level_mut().sectors {
            if gs.sector_type.is_lift() || gs.lift_type.is_some() {
                gs.low_exit_point = None;
                gs.high_exit_point = None;
                gs.lowest_door_index = None;
            }
        }
        let mut lift_endpoints_cached = 0usize;
        let mut lift_endpoints_partial = 0usize;
        let lift_endpoint_selections: Vec<_> = self
            .world
            .fast_grid
            .level
            .sectors
            .iter()
            .enumerate()
            .filter(|(_, sector)| sector.sector_type.is_lift() || sector.lift_type.is_some())
            .filter_map(|(grid_idx, sector)| {
                let (low_idx, high_idx) = lift_endpoint_door_indices(
                    &self.script_domains.interactables.doors,
                    sector.sector_number,
                )?;
                let low_point = self.script_domains.interactables.doors[low_idx as usize].point_in;
                let high_point =
                    self.script_domains.interactables.doors[high_idx as usize].point_in;
                Some((grid_idx, low_idx, low_point, high_point))
            })
            .collect();
        for (grid_idx, low_idx, low_point, high_point) in lift_endpoint_selections {
            let gs = self
                .world
                .fast_grid_mut()
                .level_mut()
                .sectors
                .get_mut(grid_idx)
                .expect("selected lift sector disappeared while caching endpoints");
            gs.low_exit_point = Some(low_point);
            gs.high_exit_point = Some(high_point);
            gs.lowest_door_index = Some(low_idx);
        }
        for gs in &self.world.fast_grid.level.sectors {
            if !(gs.sector_type.is_lift() || gs.lift_type.is_some()) {
                continue;
            }
            match (gs.low_exit_point, gs.high_exit_point) {
                (Some(_), Some(_)) => lift_endpoints_cached += 1,
                (low, high) if low.is_some() ^ high.is_some() => {
                    lift_endpoints_partial += 1;
                    if matches!(
                        gs.lift_type,
                        Some(crate::sector::LiftType::Wall | crate::sector::LiftType::Ladder)
                    ) {
                        return Err(MissionLevelBuildError::MissingLiftEndpoint {
                            lift_type: format!("{:?}", gs.lift_type.expect("matched Some lift")),
                            sector_number: i16::from(gs.sector_number),
                            endpoint: if low.is_none() { "low" } else { "high" }.to_owned(),
                        });
                    }
                }
                (None, None) => {
                    if matches!(
                        gs.lift_type,
                        Some(crate::sector::LiftType::Wall | crate::sector::LiftType::Ladder)
                    ) {
                        return Err(MissionLevelBuildError::MissingLiftEndpoint {
                            lift_type: format!("{:?}", gs.lift_type.expect("matched Some lift")),
                            sector_number: i16::from(gs.sector_number),
                            endpoint: "both high and low".to_owned(),
                        });
                    }
                }
                _ => unreachable!("lift endpoint match is exhaustive"),
            }
        }
        tracing::debug!(
            "Loaded lift exit points after door load: {} sectors fully resolved, {} partial",
            lift_endpoints_cached,
            lift_endpoints_partial,
        );
        Ok(())
    }

    fn build_patch_stage(
        &mut self,
        assets: &LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
        patch_count: usize,
    ) {
        self.script_domains
            .interactables
            .patches
            .reserve(patch_count);
        // ── Patches ──
        for (patch_idx, raw) in loaded
            .proto
            .patches
            .iter()
            .chain(&loaded.mission.mission_patches)
            .enumerate()
        {
            // Copy sight obstacle indices directly — they index into
            // EngineInner::sight_obstacles which is loaded from the same proto.
            let old_sight: Vec<crate::sight_obstacle::SightObstacleIndex> = raw
                .old_sight_obstacles
                .iter()
                .filter_map(|&i| crate::sight_obstacle::SightObstacleIndex::new(u32::from(i)))
                .collect();
            let new_sight: Vec<crate::sight_obstacle::SightObstacleIndex> = raw
                .new_sight_obstacles
                .iter()
                .filter_map(|&i| crate::sight_obstacle::SightObstacleIndex::new(u32::from(i)))
                .collect();

            // Register old/new sector polygons in the FastFindGrid.
            let mut old_sector_indices: Vec<u32> = Vec::new();
            let mut new_sector_indices: Vec<u32> = Vec::new();

            // Helper: register a SectorPolygon as a GridSector if non-empty.
            let register_sector = |grid: &mut crate::fast_find_grid::FastFindGrid,
                                   poly: &crate::level_data::SectorPolygon,
                                   sector_type: crate::sector::SectorType,
                                   active: bool,
                                   layer: u16|
             -> Option<u32> {
                if poly.points.is_empty() {
                    return None;
                }
                let points: Vec<MapPoint> = poly
                    .points
                    .iter()
                    .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                    .collect();
                let mut bbox = MapBBox::new();
                for &p in &points {
                    bbox.expand_point(p);
                }
                let gs = crate::fast_find_grid::GridSector {
                    points,
                    bounding_box: bbox,
                    sector_type,
                    layer,
                    sector_number: crate::sector::SectorNumber::new(-1), /* Patch sectors don't have motion sector numbers */
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
                let idx = grid.add_sector(gs, layer);
                grid.set_sector_active(idx, active);
                Some(idx)
            };

            // Mirrors C++ `mFastGrid.AddPatch(pPatch, pPatch->GetLayer(), …)`
            // — the late `READ(muwLayer)` at the end of `RHPatch::Initialize`
            // clobbers the mid-stream read.
            let patch_layer = raw.final_layer;
            let mouse_patch = crate::sector::SectorType::MOUSE | crate::sector::SectorType::PATCH;
            let mouse_motion = crate::sector::SectorType::MOUSE | crate::sector::SectorType::MOTION;

            // Old sectors: active = true (visible before patch fires)
            if let Some(idx) = register_sector(
                self.world.fast_grid_mut(),
                &raw.old_mouse_sector,
                mouse_patch,
                true,
                patch_layer,
            ) {
                old_sector_indices.push(idx);
            }
            if let Some(idx) = register_sector(
                self.world.fast_grid_mut(),
                &raw.old_masking_sector,
                mouse_motion,
                true,
                patch_layer,
            ) {
                old_sector_indices.push(idx);
            }
            // New sectors: active = false (hidden until patch fires)
            if let Some(idx) = register_sector(
                self.world.fast_grid_mut(),
                &raw.new_mouse_sector,
                mouse_patch,
                false,
                patch_layer,
            ) {
                new_sector_indices.push(idx);
            }
            if let Some(idx) = register_sector(
                self.world.fast_grid_mut(),
                &raw.new_masking_sector,
                mouse_motion,
                false,
                patch_layer,
            ) {
                new_sector_indices.push(idx);
            }

            // ── Apply / NoApply sectors ──
            //
            //   - `apply_sector`    = CROSS | PATCH | APPLY, always
            //     active when non-empty
            //   - `no_apply_sector` = CROSS | PATCH, always active when
            //     non-empty
            // Both get registered with `AddSector` + `AddSectorLines`
            // because they are cross-sectors.  The resulting
            // LINE_PATCH | LINE_CROSS boundary segments feed the per-PC
            // patch auto-trigger.
            let cross_patch_apply = crate::sector::SectorType::CROSS
                | crate::sector::SectorType::PATCH
                | crate::sector::SectorType::APPLY;
            let cross_patch = crate::sector::SectorType::CROSS | crate::sector::SectorType::PATCH;

            // Register the apply polygon as a cross-sector + build its
            // LINE_PATCH boundary lines carrying this patch's index.
            let apply_sector_idx = register_sector(
                self.world.fast_grid_mut(),
                &raw.apply_sector,
                cross_patch_apply,
                true,
                patch_layer,
            );
            if let Some(idx) = apply_sector_idx
                && let Some(patch_index) = crate::patch::PatchIndex::new(patch_idx as u32)
            {
                self.world.fast_grid_mut().add_sector_lines_for_patch(
                    idx,
                    patch_layer,
                    patch_index,
                    true, // apply sector is always active
                );
            }

            // Register the no-apply polygon identically, minus the APPLY bit.
            if let Some(idx) = register_sector(
                self.world.fast_grid_mut(),
                &raw.no_apply_sector,
                cross_patch,
                true,
                patch_layer,
            ) && let Some(patch_index) = crate::patch::PatchIndex::new(patch_idx as u32)
            {
                self.world.fast_grid_mut().add_sector_lines_for_patch(
                    idx,
                    patch_layer,
                    patch_index,
                    true, // no-apply sector is always active
                );
            }

            // Old/new line indices are populated at runtime when a patch
            // is triggered — they don't appear in the proto stream, so we
            // start empty.
            let old_line_indices: Vec<crate::fast_find_grid::LineIndex> = Vec::new();
            let new_line_indices: Vec<crate::fast_find_grid::LineIndex> = Vec::new();

            // Motion construction precedes the mission-domain builder, so mask
            // references can be attached now instead of leaking through a
            // loose pending-data bag.
            let resolve_mask = |mask_ref: &crate::level_data::MaskRef| {
                self.world.fast_grid.level.layers[mask_ref.layer as usize].mask_indices
                    [mask_ref.index as usize]
            };
            let old_mask_indices: Vec<crate::mask::MaskIndex> =
                raw.old_masks.iter().map(resolve_mask).collect();
            let new_mask_indices: Vec<crate::mask::MaskIndex> =
                raw.new_masks.iter().map(resolve_mask).collect();
            for &mask_index in &new_mask_indices {
                self.world
                    .fast_grid_mut()
                    .set_mask_active(mask_index, false);
            }

            self.script_domains
                .interactables
                .patches
                .push(crate::patch::Patch {
                    active: raw.active,
                    // `initially_active` is unconditionally overridden to
                    // `true` (a debug-leftover line, but it is what the
                    // shipped binary does), so script-driven `ForceReset`
                    // re-activates the patch as the game expects.
                    initially_active: true,
                    definitive: raw.definitive,
                    animated: true, // default
                    door_triggered: raw.door_triggered,
                    triggers_door: raw.triggers_door,
                    integrate_in_background: raw.integrate_in_background,
                    animation_flags: crate::patch::AnimationFlags {
                        start_valid: raw.start_animation_valid,
                        transition_valid: raw.transition_animation_valid,
                        end_valid: raw.end_animation_valid,
                    },
                    // Presence of the sector/layer tuple records whether the
                    // encoded stream value was non-zero.  The decoded obstacle
                    // index may itself be zero (`(1 - 1) >> 1`), while the C++
                    // loader still sets mbUseChangingObstacles in that case.
                    use_changing_obstacles: raw.pathfinder_sector.is_some(),
                    pathfinder_layer: raw.pathfinder_layer.unwrap_or(0),
                    pathfinder_sector: raw.pathfinder_sector.unwrap_or(0),
                    pathfinder_changing_obstacles: raw.pathfinder_changing_obstacles,
                    // C++ reads muwLayer twice (mid-stream and again at end of
                    // patch); the late `final_layer` clobbers the early read,
                    // so that's the authoritative value used by `AddPatch`
                    // and `mpSelectedPatch->GetLayer()`.
                    layer: raw.final_layer,
                    sector: raw.sector,
                    waypoint: MapPoint::new(raw.waypoint.0 as f32, raw.waypoint.1 as f32),
                    old_sight_obstacle_indices: old_sight,
                    new_sight_obstacle_indices: new_sight,
                    old_sector_indices,
                    new_sector_indices,
                    old_line_indices,
                    new_line_indices,
                    old_mask_indices,
                    new_mask_indices,
                    apply_sector_index: apply_sector_idx,
                    ..Default::default()
                });
        }

        // Wire door↔patch connections.  In C++ (`RHpatch.cpp:300-308`)
        // the two relationships are *mutually exclusive* — a patch's
        // door list is either consumed as door→patch links (every door
        // gets `mpPatch = this`) when `mbDoorTriggered`, or as the
        // patch→door swap-rights list (`maDoors`) when `mbTriggersDoor`.
        // Mirror that here: gate `Door::patch_index` on `door_triggered`
        // and `Patch::door_indices` on `triggers_door`.
        let mut door_triggered_count = 0_usize;
        let mut triggers_door_count = 0_usize;
        for (patch_idx, raw) in loaded
            .proto
            .patches
            .iter()
            .chain(&loaded.mission.mission_patches)
            .enumerate()
        {
            let patch_door_indices: Vec<u32> = raw
                .door_indices
                .iter()
                .map(|&raw_idx| u32::from(raw_idx))
                .collect();
            if !patch_door_indices.is_empty() && !raw.door_triggered && !raw.triggers_door {
                tracing::warn!(
                    "Patch {} lists {} door(s) but neither door_triggered nor \
                     triggers_door is set — wiring will be skipped",
                    patch_idx,
                    patch_door_indices.len()
                );
            }
            if raw.door_triggered {
                for &door_idx in &patch_door_indices {
                    self.script_domains.interactables.doors[door_idx as usize].patch_index =
                        crate::patch::PatchIndex::new(patch_idx as u32);
                }
                if !patch_door_indices.is_empty() {
                    door_triggered_count += 1;
                }
            }
            if raw.triggers_door
                && let Some(patch) = self.script_domains.interactables.patches.get_mut(patch_idx)
            {
                let n = patch_door_indices.len();
                patch.door_indices = patch_door_indices;
                if n > 0 {
                    triggers_door_count += 1;
                }
            }
        }
        tracing::debug!(
            door_triggered_patches = door_triggered_count,
            triggers_door_patches = triggers_door_count,
            "patch↔door wiring complete"
        );

        // ── Patch animation entities ──
        // Transfer the entity handle mapping computed during entity spawning.
        tracing::info!(
            "Populated {} canonical patches from level data ({} with FX entities)",
            self.script_domains.interactables.patches.len(),
            assets
                .entities
                .patch_animation_entities
                .iter()
                .filter(|h| h.is_some())
                .count(),
        );
        debug_assert_eq!(
            patch_count,
            self.script_domains.interactables.patches.len(),
            "preflight patch count drifted during construction"
        );
    }

    fn build_building_stage(&mut self, stages: &MissionLevelBuildPlan) {
        // ── Building occupants from tenant data ──
        for building in &stages.buildings.attachments {
            let bld_idx = building.building_index;
            if bld_idx >= self.script_domains.buildings.occupants.len() {
                self.script_domains
                    .buildings
                    .occupants
                    .resize(bld_idx + 1, Vec::new());
            }
            // Parallel array: propagate the `arrow_reserve` flag off the
            // same tenant chunk so `initialize_buildings` can copy it
            // into `ai::House::arrow_reserve`.  Consumer: AI's
            // `FleeingRunForArrowReserves` substate.
            if bld_idx >= self.script_domains.buildings.arrow_reserves.len() {
                self.script_domains
                    .buildings
                    .arrow_reserves
                    .resize(bld_idx + 1, false);
            }
            self.script_domains.buildings.arrow_reserves[bld_idx] = building.arrow_reserve;
            for &elem_idx in &building.tenant_element_indices {
                let actor_h = crate::natives::ScriptHandleCodec::actor_handle_from_index(
                    usize::from(elem_idx),
                );
                self.script_domains.buildings.occupants[bld_idx].push(actor_h);
                let bld_h = crate::natives::ScriptHandleCodec::building_handle_from_index(bld_idx);
                self.script_domains
                    .buildings
                    .actor_building
                    .insert(actor_h, bld_h);
            }
        }
    }

    /// Attach validated tenant entities in GUYS/CAVE authored order to each
    /// building's first gate, matching `RHSectorBuilding::InitOccupant`.
    fn attach_mission_level_stage(
        &mut self,
        stages: &MissionLevelBuildPlan,
    ) -> Result<(), MissionLevelBuildError> {
        debug_assert_eq!(
            stages.patch_count,
            self.script_domains.interactables.patches.len(),
            "validated patch stage drifted before attachment"
        );
        let lift_layer = if self.world.fast_grid.level.special_layer > 0 {
            self.world.fast_grid.lift_layer()
        } else {
            0
        };
        for building in &stages.buildings.attachments {
            if building.tenant_element_indices.is_empty() {
                continue;
            }
            let first_door_index =
                building
                    .first_door_index
                    .ok_or(MissionLevelBuildError::BuildingWithoutDoor {
                        building_index: building.building_index,
                    })?;
            let first_door = self
                .script_domains
                .interactables
                .doors
                .get(usize::from(first_door_index))
                .ok_or(MissionLevelBuildError::MissingCanonicalBuildingDoor {
                    building_index: building.building_index,
                    door_index: first_door_index.0,
                })?;
            // `RHSectorBuilding::InitOccupant` resolves GetGate(0) after
            // RHDoor::AdaptPoints, so attachment must use this canonical
            // point rather than the raw proto point staged before construction.
            let first_door_point_in = first_door.point_in;
            let first_door_sector_in = u16::from(first_door.sector_in);
            let first_door_sector_in_index = first_door.sector_in_index.unwrap_or_else(|| {
                panic!(
                    "building {} first door has no exact interior sector identity",
                    building.building_index
                )
            });
            for &element_index in &building.tenant_element_indices {
                let entity_id = self
                    .world
                    .entities
                    .id_at_legacy_slot(u32::from(element_index))
                    .ok_or(MissionLevelBuildError::MissingBuildingTenant {
                        building_index: building.building_index,
                        element_index,
                    })?;
                let carried = {
                    let entity = self.world.entities.get_mut(entity_id).ok_or(
                        MissionLevelBuildError::MissingBuildingTenant {
                            building_index: building.building_index,
                            element_index,
                        },
                    )?;
                    if !entity.is_human() {
                        return Err(MissionLevelBuildError::NonHumanBuildingTenant {
                            building_index: building.building_index,
                            element_index,
                        });
                    }
                    let carried = entity.pc_data().and_then(|pc| pc.carried);
                    let elem = entity.element_data_mut();
                    elem.active = false;
                    let pi = &mut elem.sprite.position_iface;
                    pi.set_map_position(first_door_point_in);
                    pi.set_layer(
                        crate::position_interface::Layer::new(lift_layer)
                            .expect("building lift layer equals the null sentinel"),
                    );
                    pi.set_sector_topology(
                        crate::position_interface::SectorHandle::new(first_door_sector_in),
                        Some(first_door_sector_in_index),
                    );
                    carried
                };
                // `RHElementActorHuman::ChangeLayerAndSector` copies the
                // building pointer and lift layer to a PC's carried actor.
                // `InitOccupant` then changes only the tenant's position.
                if let Some(carried_id) = carried {
                    let carried = self.world.entities.get_mut(carried_id).unwrap_or_else(|| {
                        panic!(
                            "building {} tenant {element_index} references missing carried actor {carried_id:?}",
                            building.building_index
                        )
                    });
                    let pi = &mut carried.element_data_mut().sprite.position_iface;
                    pi.set_layer(
                        crate::position_interface::Layer::new(lift_layer)
                            .expect("building lift layer equals the null sentinel"),
                    );
                    pi.set_sector_topology(
                        crate::position_interface::SectorHandle::new(first_door_sector_in),
                        Some(first_door_sector_in_index),
                    );
                }
            }
        }
        Ok(())
    }

    /// Harvest the Sherwood engine state into the campaign's production
    /// sectors just before exiting Sherwood: for every sector, capture
    /// amount from engine bonuses and occupants from script-zone
    /// membership.  Invoked at mission start.
    pub(crate) fn harvest_production_sector_state(&mut self, assets: &LevelAssets) {
        // Build a (production_type → occupants Vec) map by walking every
        // script zone that carries a production type.  Capturing occupants
        // here keeps the borrow of `campaign` short (we only update it
        // after all engine reads are done).
        let mut per_sector_occupants: std::collections::HashMap<
            crate::sector_production::Type,
            Vec<crate::sector_production::Occupant>,
        > = std::collections::HashMap::new();

        for (zone_idx, zone) in self.script_domains.zones.scripts.iter().enumerate() {
            let prod_type = zone.production_sector_type;
            if prod_type == crate::sector_production::Type::Unknown
                || prod_type == crate::sector_production::Type::Relic
            {
                // RELIC sectors don't accept occupants; UNKNOWN has no
                // associated sector.
                continue;
            }

            // TRAIN_BOW additionally requires the PC to own Action::Bow.
            let train_bow_filter = prod_type == crate::sector_production::Type::TrainBow;

            let captured = per_sector_occupants.entry(prod_type).or_default();

            for &elem_idx in &zone.occupant_indices {
                let slot = self.world.entities.get(elem_idx);
                let Some(entity) = slot else { continue };
                let crate::element::Entity::Pc(pc) = entity else {
                    continue;
                };

                let profile_idx = pc.pc.profile_index;

                if train_bow_filter {
                    let Some(_campaign) = Some(&self.mission_domain.campaign) else {
                        continue;
                    };
                    let Some(profile) = assets.profile_manager.get_character(profile_idx) else {
                        panic!(
                            "TRAIN_BOW occupant profile {profile_idx} missing from ProfileManager"
                        );
                    };
                    if !profile.has_action(crate::profiles::Action::Bow) {
                        continue;
                    }
                }

                // Find the PcDescription index (position in campaign.characters).
                let Some(campaign) = Some(&self.mission_domain.campaign) else {
                    continue;
                };
                let Some(pc_description_idx) = campaign
                    .characters
                    .iter()
                    .position(|desc| desc.character_profile_idx == Some(profile_idx))
                else {
                    // The zone contained a PC whose profile isn't in the
                    // campaign's PcDescription list — that's a state-
                    // consistency bug, not a fallback case.
                    panic!(
                        "production sector PC profile {profile_idx} has no PcDescription in campaign (handle={elem_idx}, zone={zone_idx})"
                    );
                };

                let obstacle = pc.element.obstacle_index().map(u16::from).unwrap_or(0xFFFF);

                captured.push(crate::sector_production::Occupant {
                    pc_description_idx,
                    x: pc.element.position_map().x,
                    y: pc.element.position_map().y,
                    obstacle,
                });
            }
        }

        // Now that engine reads are done, write into the campaign sectors:
        // amount harvest (from entities) + occupants (from zones above).
        let entities_snapshot = &self.world.entities;
        let Some(campaign) = Some(&mut self.mission_domain.campaign) else {
            return;
        };
        for sector in &mut campaign.production_sectors {
            // Bonus-amount branch runs for MAKE_* / any sector with an
            // associated action; no-op for TRAIN/HEAL/RELIC.
            sector.get_amount_from_current_mission(entities_snapshot);

            // Replace occupants with the fresh snapshot: sectors
            // whose type has no zone in the new map (or whose zone
            // produced no valid PCs) drop their previous occupants
            // instead of keeping them across mission cycles.
            //
            // Keying is still per-prod_type rather than per-sector
            // identity; stock data only ever has one zone per
            // production type so multiple SectorProductions sharing a
            // type all see the same captured set.  If future data
            // exposes multiple zones per type, switch this to a
            // per-sector zone index.
            sector.occupants.clear();
            if let Some(new_occupants) = per_sector_occupants.get(&sector.prod_type) {
                sector.occupants = new_occupants.clone();
            }

            // Clear `production_points` after the per-sector capture so
            // the next Sherwood load's script Initialize callbacks
            // (which re-run the `AddProductionPoint` opcodes)
            // doesn't accumulate duplicate points across visits — without
            // this, `plan_bonus_spawns` iterates more points than exist
            // in the level and raises `max_amount_reached` prematurely
            // while spawning duplicate-position bonuses.
            sector.production_points.clear();
            sector.script_zone = None;
        }
    }

    /// Apply stored production-sector state to the live Sherwood engine:
    /// finalize amounts (UpdateAmount), spawn MAKE_* bonuses at production
    /// points, spawn collected RELIC items, restore occupant PCs to their
    /// recorded positions, apply training XP (TRAIN_BOW / TRAIN_HAND_TO_HAND)
    /// and heal occupants (HEAL) for won missions.  Invoked at Sherwood
    /// entry.
    ///
    /// Must be called after mission-script Initialize so its synchronous
    /// production registrations have populated the points, and only when the
    /// current level is Sherwood.
    /// Called from `Engine::new` when Sherwood is the current mission.
    pub(crate) fn apply_production_sector_data(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &mut LevelAssets,
    ) {
        use crate::sector_production::Type as PT;

        // Resolve last-mission info — drives UpdateAmount/Experience/LifePoints.
        let (last_won, last_length) = {
            let Some(campaign) = Some(&self.mission_domain.campaign) else {
                return;
            };
            match campaign.last_mission_idx {
                Some(idx) => {
                    let mission = &campaign.missions[idx];
                    (
                        mission.status == crate::mission::MissionStatus::Won,
                        mission.profile(&assets.profile_manager).length,
                    )
                }
                None => (false, 0),
            }
        };

        // Resolve per-sector specialist flags + run UpdateAmount (adds
        // produced_amount into amount).  Collect a per-sector "plan" of
        // follow-up actions (spawn bonuses, spawn relics, restore occupants,
        // add XP, heal), along with the script-zone layer/sector needed to
        // position restored occupants.
        let points_count = assets
            .scripts
            .location_positions
            .len()
            .saturating_sub(self.script_domains.zones.scripts.len());

        struct SectorPlan {
            prod_type: PT,
            points: Vec<crate::sector_production::Point>,
            bonus_spawns: Vec<(usize, u16)>, // (point_idx, quantity)
            occupants: Vec<crate::sector_production::Occupant>,
            zone_layer: Option<u16>,
            zone_sector: Option<u16>,
            // Applied-in-engine post-mission PC updates
            xp_gain: u16,   // TRAIN_* only
            heal_gain: u16, // HEAL only
        }

        let mut plans: Vec<SectorPlan> = Vec::new();

        // Build a zone-type → (layer, sector) map for attaching sectors.
        let mut zone_location: std::collections::HashMap<PT, (u16, u16)> =
            std::collections::HashMap::new();
        for (zone_idx, zone) in self.script_domains.zones.scripts.iter().enumerate() {
            let pt = zone.production_sector_type;
            if pt == PT::Unknown {
                continue;
            }
            let loc_handle_idx = points_count + zone_idx; // 0-based index into script_location_*
            if let (Some(&layer), Some(&sector)) = (
                assets.scripts.location_layers.get(loc_handle_idx),
                assets.scripts.location_sectors.get(loc_handle_idx),
            ) {
                zone_location.entry(pt).or_insert((layer, sector));
            }
        }

        // Finalize amounts + gather plan data.
        {
            let Some(campaign) = Some(&mut self.mission_domain.campaign) else {
                return;
            };
            // snapshot specialist resolution before mutating occupants — it
            // reads the *current* occupants (captured just before mission).
            let has_specialist: Vec<bool> = campaign
                .production_sectors
                .iter()
                .map(|s| {
                    if s.associated_action().is_some()
                        || matches!(s.prod_type, PT::TrainBow | PT::TrainHandToHand | PT::Heal)
                    {
                        Self::sector_has_specialist_cached(campaign, &assets.profile_manager, s)
                    } else {
                        false
                    }
                })
                .collect();

            for (sector, &specialist) in campaign
                .production_sectors
                .iter_mut()
                .zip(has_specialist.iter())
            {
                match sector.prod_type {
                    PT::MakeArrow
                    | PT::MakePurse
                    | PT::MakeStone
                    | PT::MakeApple
                    | PT::MakeAle
                    | PT::MakeLamblegg
                    | PT::MakePlant
                    | PT::MakeNet
                    | PT::MakeWaspNest => {
                        if last_won {
                            sector.update_amount(last_length, specialist);
                        }
                        let bonus_spawns = sector.plan_bonus_spawns();
                        let (layer, sector_idx) =
                            zone_location.get(&sector.prod_type).copied().unzip();
                        plans.push(SectorPlan {
                            prod_type: sector.prod_type,
                            points: sector.production_points.clone(),
                            bonus_spawns,
                            occupants: sector.occupants.clone(),
                            zone_layer: layer,
                            zone_sector: sector_idx,
                            xp_gain: 0,
                            heal_gain: 0,
                        });
                    }
                    PT::TrainBow | PT::TrainHandToHand => {
                        let xp = if last_won {
                            // UpdateExperience:
                            //   training_speed = speed / 100.0
                            //   super = specialist ? 2.0 : 1.0
                            //   gain = super * training_speed * length
                            let training = (sector.speed as f32) / 100.0;
                            let super_mul = if specialist { 2.0 } else { 1.0 };
                            (super_mul * training * last_length as f32) as u16
                        } else {
                            0
                        };
                        let (layer, sector_idx) =
                            zone_location.get(&sector.prod_type).copied().unzip();
                        plans.push(SectorPlan {
                            prod_type: sector.prod_type,
                            points: Vec::new(),
                            bonus_spawns: Vec::new(),
                            occupants: sector.occupants.clone(),
                            zone_layer: layer,
                            zone_sector: sector_idx,
                            xp_gain: xp,
                            heal_gain: 0,
                        });
                    }
                    PT::Heal => {
                        let heal = if last_won {
                            // UpdateLifePoints:
                            //   healing = speed / 100.0
                            //   super = specialist ? 1.5 : 1.0
                            //   amt = super * healing * length
                            let healing = (sector.speed as f32) / 100.0;
                            let super_mul = if specialist { 1.5 } else { 1.0 };
                            (super_mul * healing * last_length as f32) as u16
                        } else {
                            0
                        };
                        let (layer, sector_idx) =
                            zone_location.get(&sector.prod_type).copied().unzip();
                        plans.push(SectorPlan {
                            prod_type: sector.prod_type,
                            points: Vec::new(),
                            bonus_spawns: Vec::new(),
                            occupants: sector.occupants.clone(),
                            zone_layer: layer,
                            zone_sector: sector_idx,
                            xp_gain: 0,
                            heal_gain: heal,
                        });
                    }
                    PT::Relic => {
                        let (layer, sector_idx) = zone_location.get(&PT::Relic).copied().unzip();
                        plans.push(SectorPlan {
                            prod_type: PT::Relic,
                            points: sector.production_points.clone(),
                            bonus_spawns: Vec::new(), // relics handled separately
                            occupants: Vec::new(),
                            zone_layer: layer,
                            zone_sector: sector_idx,
                            xp_gain: 0,
                            heal_gain: 0,
                        });
                    }
                    PT::Unknown => {}
                }
            }
        }

        // Snapshot collected relics for the RELIC branch.
        let collected_relics: Vec<u32> = Some(&self.mission_domain.campaign)
            .map(|c| c.collected_relics.clone())
            .unwrap_or_default();

        let char_base_dir = "Data/Characters";
        let bank_signature = assets.bank_signature;
        // Sherwood production-spawned bonuses are never blipped on the
        // minimap, even on non-forest maps.
        let blipped = false;

        for plan in &plans {
            // ── MAKE_*: spawn bonus entities at production points ──
            if let Some(raw_bonus) = plan.prod_type.bonus_raw_type()
                && let Some((sprite_file, profile_name, object_type)) =
                    bonus_type_to_sprite_asset(raw_bonus)
            {
                let bonus_kind = crate::element::BonusItemType::from_u16(raw_bonus)
                    .unwrap_or_else(|| panic!("unknown BonusItemType ordinal {raw_bonus}"));
                let associated_action = bonus_kind.to_action();
                for &(point_idx, quantity) in &plan.bonus_spawns {
                    let Some(point) = plan.points.get(point_idx) else {
                        continue;
                    };
                    let mut sprite = crate::sprite::Sprite::default();
                    if let Err(e) = sprite.load_frame_info(
                        assets.sprite_scriptor_mut(),
                        crate::sprite_script::FrameKind::Object,
                        char_base_dir,
                        sprite_file,
                        profile_name,
                        bank_signature,
                        Some(self.world.weather.ambiance.to_sprite_ambiance()),
                    ) {
                        tracing::error!(
                            "Sherwood production bonus sprite '{sprite_file}' / '{profile_name}' failed: {e}"
                        );
                        continue;
                    }
                    sprite.force_random_sprite_frame(
                        sim,
                        crate::sim_rng::RngSite::SherwoodProductionBonusFrame,
                    );
                    sprite.apply_placement(
                        MapPoint::new(point.x, point.y),
                        point.layer,
                        crate::position_interface::SectorHandle::new(point.sector),
                        0,
                        crate::element::GameMaterial::default(),
                        crate::position_interface::ObstacleHandle::new(point.obstacle),
                        crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                            crate::position_interface::ObstacleHandle::new(point.obstacle),
                            assets.static_sight_obstacles.as_slice(),
                        ),
                    );
                    let entity = crate::element::Entity::Bonus(crate::element::ElementBonus {
                        element: crate::element::ElementData {
                            kind: crate::element::ElementKind::ObjectBonus,
                            blipped,
                            sprite,
                            ..Default::default()
                        },
                        object: crate::element::ObjectData {
                            quantity,
                            object_type,
                            associated_action,
                            ..Default::default()
                        },
                    });
                    self.add_entity(entity);
                }
            }

            // ── RELIC: spawn one bonus entity per collected relic, one per point ──
            if plan.prod_type == PT::Relic {
                for (relic_idx, &relic_raw) in collected_relics.iter().enumerate() {
                    let Some(point) = plan.points.get(relic_idx) else {
                        break;
                    };
                    let Some((sprite_file, profile_name, object_type)) =
                        bonus_type_to_sprite_asset(relic_raw as u16)
                    else {
                        tracing::warn!(
                            "Unknown relic raw type {relic_raw} — cannot resolve sprite"
                        );
                        continue;
                    };
                    let bonus_kind = crate::element::BonusItemType::from_u16(relic_raw as u16)
                        .unwrap_or_else(|| panic!("unknown BonusItemType ordinal {relic_raw}"));
                    let associated_action = bonus_kind.to_action();
                    let mut sprite = crate::sprite::Sprite::default();
                    if let Err(e) = sprite.load_frame_info(
                        assets.sprite_scriptor_mut(),
                        crate::sprite_script::FrameKind::Object,
                        char_base_dir,
                        sprite_file,
                        profile_name,
                        bank_signature,
                        Some(self.world.weather.ambiance.to_sprite_ambiance()),
                    ) {
                        tracing::error!(
                            "Sherwood relic sprite '{sprite_file}' / '{profile_name}' failed: {e}"
                        );
                        continue;
                    }
                    sprite.force_random_sprite_frame(
                        sim,
                        crate::sim_rng::RngSite::SherwoodRelicFrame,
                    );
                    sprite.apply_placement(
                        MapPoint::new(point.x, point.y),
                        point.layer,
                        crate::position_interface::SectorHandle::new(point.sector),
                        0,
                        crate::element::GameMaterial::default(),
                        crate::position_interface::ObstacleHandle::new(point.obstacle),
                        crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                            crate::position_interface::ObstacleHandle::new(point.obstacle),
                            assets.static_sight_obstacles.as_slice(),
                        ),
                    );
                    let entity = crate::element::Entity::Bonus(crate::element::ElementBonus {
                        element: crate::element::ElementData {
                            kind: crate::element::ElementKind::ObjectBonus,
                            blipped,
                            sprite,
                            ..Default::default()
                        },
                        object: crate::element::ObjectData {
                            quantity: 1,
                            object_type,
                            associated_action,
                            ..Default::default()
                        },
                    });
                    self.add_entity(entity);
                }
            }

            // ── Occupant restore + work-icon set + XP/heal ──
            for occupant in &plan.occupants {
                // Resolve the PC description → profile_index → find live entity.
                let profile_idx = {
                    let Some(campaign) = Some(&self.mission_domain.campaign) else {
                        continue;
                    };
                    let Some(desc) = campaign.characters.get(occupant.pc_description_idx) else {
                        panic!(
                            "occupant pc_description_idx {} out of range",
                            occupant.pc_description_idx
                        );
                    };
                    match desc.character_profile_idx {
                        Some(p) => p,
                        None => continue, // unlinked description — skip
                    }
                };

                let Some(pc_id) = self
                    .world
                    .entities
                    .pcs()
                    .find_map(|(id, pc)| (pc.pc.profile_index == profile_idx).then_some(id))
                else {
                    // PC isn't present in this Sherwood load (e.g. not yet
                    // rescued, or lost) — skip silently.
                    continue;
                };
                let entity_id = EntityId::Pc(pc_id);

                let (Some(layer), Some(sector_idx)) = (plan.zone_layer, plan.zone_sector) else {
                    // No script-zone for this production type — cannot
                    // position.  Leave the PC where it is.
                    continue;
                };

                // Teleport, then refresh obstacle + plane + material via
                // the shared `set_obstacle_and_material` helper: with an
                // obstacle, pull the obstacle's top-plane and material;
                // without one, clear the plane and resolve the footstep
                // material from the SECTOR_SOUND polygons at the current
                // map position (or `default_material` when none contain
                // the point).
                if let Some(crate::element::Entity::Pc(pc)) = self.world.entities.get_mut(entity_id)
                {
                    pc.element
                        .set_position_map(MapPoint::new(occupant.x, occupant.y));
                    pc.element.set_layer(layer);
                    pc.element
                        .set_sector(crate::position_interface::SectorHandle::new(sector_idx));
                    {
                        let pi = &mut pc.element.sprite.position_iface;
                        pi.set_map_position(MapPoint::new(occupant.x, occupant.y));
                    }
                }
                // Release the `entities` borrow before calling helpers
                // that take `&mut self`.
                let occupant_obstacle_opt = if occupant.obstacle == 0xFFFF {
                    None
                } else {
                    Some(occupant.obstacle)
                };
                self.set_obstacle_and_material(assets, entity_id, occupant_obstacle_opt);
                if let Some(crate::element::Entity::Pc(pc)) = self.world.entities.get_mut(entity_id)
                {
                    pc.element.update_grid_cell();
                }

                // Set the work icon for the production type.
                let pt = plan.prod_type;
                self.apply_production_work_icon(entity_id, pt, true);

                // XP / heal on the live PC.
                if plan.xp_gain > 0 {
                    let skill = if plan.prod_type == PT::TrainBow {
                        crate::pc_status::SkillName::Bow
                    } else {
                        crate::pc_status::SkillName::HandToHand
                    };
                    // TRAIN_BOW's occupant filter already ensured
                    // Action::Bow at harvest time — no re-check here.
                    //
                    // Sherwood training uses `human_status.add_experience`
                    // directly rather than `campaign.add_pc_experience`,
                    // which deliberately bypasses the campaign-score
                    // bonus that the PC override awards when capacity
                    // crosses a 100-XP boundary.  Going through
                    // `add_pc_experience` would over-credit Sherwood
                    // training by 100 Score per skill-capacity threshold
                    // crossed.
                    if let Some(campaign) = Some(&mut self.mission_domain.campaign)
                        && let Some(desc) = campaign.characters.get_mut(occupant.pc_description_idx)
                    {
                        desc.status
                            .human_status
                            .add_experience(skill, plan.xp_gain as u32);
                    }
                }
                if plan.heal_gain > 0 {
                    let amount = plan.heal_gain.min(i16::MAX as u16) as i16;
                    if let Some(crate::element::Entity::Pc(pc)) =
                        self.world.entities.get_mut(entity_id)
                    {
                        crate::pc_status::heal(&mut pc.pc.life_points, amount, false);
                    }
                    if let Some(campaign) = Some(&mut self.mission_domain.campaign)
                        && let Some(desc) = campaign.characters.get_mut(occupant.pc_description_idx)
                    {
                        crate::pc_status::heal(&mut desc.status.life_points, amount, false);
                    }
                }
            }
        }

        // After teleporting occupants to their recorded positions,
        // rebuild zone membership against the new positions without
        // firing any `EnterZone` scripts.  Without this,
        // `initialize_zone_occupants` (which ran earlier against
        // pre-teleport positions) has left stale entries that the
        // per-frame `tick_zone_occupants` would reconcile by
        // dispatching spurious `ExitZone` / `EnterZone` events.
        self.refresh_zone_occupants_silent(assets);
    }

    /// Free-function variant of `Campaign::sector_has_specialist` that takes
    /// the campaign by shared ref.  Exists so we can call it while holding
    /// a `&mut Vec<SectorProduction>` borrow at the same time.
    fn sector_has_specialist_cached(
        campaign: &crate::campaign::Campaign,
        profiles: &crate::profiles::ProfileManager,
        sector: &crate::sector_production::SectorProduction,
    ) -> bool {
        campaign.sector_has_specialist(sector, profiles)
    }
}

fn shuffle_sherwood_slots(
    sim: &crate::sim_rng::SimulationContext,
    n: usize,
    mut swap: impl FnMut(usize, usize),
) {
    assert!(
        n > 0,
        "Sherwood beam-me shuffle requires a non-empty slot list"
    );
    for _ in 0..100 {
        let a = crate::sim_rng::usize(sim, crate::sim_rng::RngSite::SherwoodBeamMeShuffle, 0..n);
        let b = crate::sim_rng::usize(sim, crate::sim_rng::RngSite::SherwoodBeamMeShuffle, 0..n);
        swap(a, b);
    }
}

#[cfg(test)]
mod lift_endpoint_tests {
    use super::lift_endpoint_door_indices;
    use crate::coordinates::MapPoint;
    use crate::gate::Door;
    use crate::sector::SectorNumber;

    fn lift_door(lift_sector: SectorNumber, point_in: MapPoint, point_out: MapPoint) -> Door {
        Door {
            sector_in: lift_sector,
            owning_lift_sector: Some(lift_sector),
            point_in,
            point_out,
            ..Door::default()
        }
    }

    #[test]
    fn lift_endpoint_identity_uses_point_out_but_caches_point_in() {
        let lift = SectorNumber::new(42);
        let doors = vec![
            // High by the outside endpoint, despite having the lower
            // lift-side PointIn in screen coordinates.
            lift_door(lift, MapPoint::new(10.0, 200.0), MapPoint::new(11.0, 10.0)),
            // Low by the outside endpoint, despite having the higher
            // lift-side PointIn in screen coordinates.
            lift_door(lift, MapPoint::new(20.0, 0.0), MapPoint::new(21.0, 100.0)),
        ];

        let (low, high) = lift_endpoint_door_indices(&doors, lift).unwrap();

        assert_eq!((low, high), (1, 0));
        assert_eq!(doors[low as usize].point_in, MapPoint::new(20.0, 0.0));
        assert_eq!(doors[high as usize].point_in, MapPoint::new(10.0, 200.0));
    }

    #[test]
    fn lift_endpoint_identity_excludes_reverse_edges_and_preserves_ties() {
        let lift = SectorNumber::new(42);
        let outside = SectorNumber::new(7);
        let mut reverse_edge = lift_door(
            outside,
            MapPoint::new(99.0, 99.0),
            MapPoint::new(99.0, 1000.0),
        );
        reverse_edge.sector_out = lift;
        let doors = vec![
            lift_door(lift, MapPoint::new(10.0, 10.0), MapPoint::new(10.0, 10.0)),
            lift_door(lift, MapPoint::new(20.0, 100.0), MapPoint::new(20.0, 100.0)),
            reverse_edge,
            // Original uses strict comparisons, so an equal low endpoint
            // retains the first authored door.
            lift_door(lift, MapPoint::new(30.0, 100.0), MapPoint::new(30.0, 100.0)),
        ];

        assert_eq!(lift_endpoint_door_indices(&doors, lift), Some((1, 0)));
    }

    #[test]
    fn lift_endpoint_identity_excludes_foreign_door_entering_same_sector() {
        let lift = SectorNumber::new(42);
        let mut foreign = Door {
            sector_in: lift,
            point_out: MapPoint::new(30.0, 101.0),
            ..Door::default()
        };
        foreign.owning_lift_sector = None;
        let doors = vec![
            foreign,
            lift_door(lift, MapPoint::new(10.0, 10.0), MapPoint::new(10.0, 10.0)),
            lift_door(lift, MapPoint::new(20.0, 100.0), MapPoint::new(20.0, 100.0)),
        ];

        assert_eq!(lift_endpoint_door_indices(&doors, lift), Some((2, 1)));
    }

    #[test]
    #[should_panic(expected = "fewer than two distinct high/low endpoint doors")]
    fn lift_endpoint_identity_rejects_one_owned_door() {
        let lift = SectorNumber::new(42);
        let doors = vec![lift_door(
            lift,
            MapPoint::new(10.0, 10.0),
            MapPoint::new(10.0, 10.0),
        )];

        let _ = lift_endpoint_door_indices(&doors, lift);
    }

    #[test]
    #[should_panic(expected = "fewer than two distinct high/low endpoint doors")]
    fn lift_endpoint_identity_rejects_equal_outside_heights() {
        let lift = SectorNumber::new(42);
        let doors = vec![
            lift_door(lift, MapPoint::new(10.0, 10.0), MapPoint::new(10.0, 50.0)),
            lift_door(lift, MapPoint::new(20.0, 20.0), MapPoint::new(20.0, 50.0)),
        ];

        let _ = lift_endpoint_door_indices(&doors, lift);
    }
}

#[cfg(test)]
mod rng_order_tests {
    use super::shuffle_sherwood_slots;
    use crate::sim_rng::RngSite;

    #[test]
    fn returning_pc_placement_draws_precede_all_beam_me_shuffle_draws() {
        crate::sim_rng::with_seed(0xA036, |sim| {
            let (_, trace) = crate::sim_rng::with_draw_trace(|| {
                let _ = super::super::teleport::roll_sherwood_placement(sim);
                shuffle_sherwood_slots(sim, 4, |_, _| {});
            });
            assert_eq!(trace.len(), 203);
            assert_eq!(
                &trace[..3],
                &[
                    RngSite::SherwoodReturningPcPlacement,
                    RngSite::SherwoodReturningPcPlacement,
                    RngSite::SherwoodReturningPcPlacement,
                ]
            );
            assert!(
                trace[3..]
                    .iter()
                    .all(|site| *site == RngSite::SherwoodBeamMeShuffle)
            );
        });
    }
}
