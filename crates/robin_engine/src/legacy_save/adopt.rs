//! Validated identity and reference planning for Original v48 save adoption.
//!
//! Original element pointers use creation-order IDs, while AI-local element
//! pointers use indices into the serialized `marrayElements`. Rust entity IDs
//! are intentionally unrelated to both spaces. Adoption therefore starts by
//! constructing one explicit isomorphic map before mutating the initialized
//! mission.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::fast_find_grid::{GridSector, SectorIndex};
use crate::{
    coordinates::{MapBBox, MapPoint, MapVec, WorldPoint3D, WorldVec3D},
    element::EntityId,
    engine::{EngineInner, LevelAssets},
    jump_line::JumpLineIndex,
    position_interface::{
        Direction, DoorHandle, IncrementComputed, Layer, ObstacleHandle, PlaneZCoeffs,
        PositionComputed, PositionInterfaceV48State, SectorHandle,
    },
};

use super::{
    body::LegacySaveBody,
    elements::{LegacyElementClass, LegacyElementEnvelope, LegacyElementResolution},
    gate_topology::derive_legacy_gate_order,
    payload_base::{
        LegacyAiElementRef, LegacyBoundingBox2, LegacyElementRef, LegacyLineRef, LegacyPoint2,
        LegacyPoint3, LegacyPositionPayload,
    },
    topology_adapter::{
        LegacyMissingTopologyFact, LegacyStaticElementTopology, LegacyTopologyAdapterError,
        derive_static_element_topology,
    },
};

#[derive(Debug, Error)]
pub enum LegacySaveAdoptError {
    #[error(transparent)]
    Topology(#[from] LegacyTopologyAdapterError),
    #[error(
        "saved static element slot {slot}, creation order {creation_order}, class {class:?} has no initialized Rust entity"
    )]
    MissingStaticEntity {
        slot: usize,
        creation_order: u32,
        class: LegacyElementClass,
    },
    #[error(
        "saved static mobile master slot {slot}, creation order {creation_order} requires mobile-state adoption"
    )]
    UnsupportedMobileMaster { slot: usize, creation_order: u32 },
    #[error(
        "saved dynamic element slot {slot}, creation order {creation_order}, class {class:?} requires dynamic factory adoption"
    )]
    UnsupportedDynamicElement {
        slot: usize,
        creation_order: u32,
        class: LegacyElementClass,
    },
    #[error(
        "initialized entity {entity_id} occurs at both Original creation orders {first_creation_order} and {second_creation_order}"
    )]
    DuplicateInitializedEntity {
        entity_id: EntityId,
        first_creation_order: u32,
        second_creation_order: u32,
    },
    #[error("save references absent Original element creation order {creation_order}")]
    MissingCreationOrderReference { creation_order: u32 },
    #[error(
        "save references AI element slot {slot}, but the serialized element array contains only {element_count} records"
    )]
    MissingAiElementSlot { slot: u16, element_count: usize },
    #[error(
        "save references Original mobile master at AI element slot {slot}; mobile masters are not actors"
    )]
    MobileMasterAiReference { slot: u16 },
    #[error("saved position field {field} has value {value}; expected {expected}")]
    InvalidPositionField {
        field: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error(
        "saved position field {field} references index {index}, but initialized topology contains only {count} entries"
    )]
    MissingPositionTopologyEntry {
        field: &'static str,
        index: usize,
        count: usize,
    },
    #[error(
        "saved position field {field} references Original sector slot {index}, which has no Rust position-sector counterpart"
    )]
    UnmappedPositionSector { field: &'static str, index: usize },
    #[error("cannot derive Original position topology: {detail}")]
    InvalidPositionTopology { detail: String },
}

/// Rust obstacle identity and the top plane paired with it.
///
/// Original load fixup in `RHPositionInterface::Serialize` restores
/// `mpPlane = mpObstacle->GetTopPlane()` immediately after resolving the
/// saved obstacle pointer. Keeping the pair pre-resolved makes the eventual
/// state install infallible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegacyPositionObstacleBinding {
    /// Rust authored-obstacle identity. The synthetic Original ground
    /// projection area has no Rust obstacle and therefore stores `None`.
    pub obstacle: Option<ObstacleHandle>,
    pub plane: PlaneZCoeffs,
}

/// Mission-created arrays used by Original position-pointer fixups.
///
/// Projection-area and sight-obstacle indices are separate Original spaces.
/// `muwLayer == 0xffff` selects sight obstacles; every other layer selects
/// projection areas (`original-code/RHpositioninterface.cpp:760-773`).
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyPositionTopology {
    /// Original sparse `marraySectors` slot to Rust's compact runtime sector.
    /// Constructor holes and non-position sector objects remain `None`.
    pub sectors: Vec<Option<SectorHandle>>,
    /// Exact runtime arena identity paired slot-for-slot with `sectors`.
    pub sector_indices: Vec<Option<SectorIndex>>,
    /// Original sparse sector slots whose object is an `RHSectorDoor`.
    pub sector_doors: Vec<Option<DoorHandle>>,
    pub doors: Vec<DoorHandle>,
    pub projection_areas: Vec<LegacyPositionObstacleBinding>,
    pub sight_obstacles: Vec<LegacyPositionObstacleBinding>,
}

/// Complete Rust counterpart of one non-null Original `RHSector*`.
///
/// `public` is the compact sector number consumed by the existing gate graph;
/// `runtime_index` is the exact polygon/object identity corresponding to the
/// Original sparse `marraySectors` slot. They are intentionally independent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacyPositionSectorIdentity {
    pub public: SectorHandle,
    pub runtime_index: SectorIndex,
}

pub(crate) fn retained_position_sector_handle(
    assets: &LevelAssets,
    sparse_slot: u16,
) -> SectorHandle {
    let retained = assets
        .legacy_grid_topology
        .as_ref()
        .expect("legacy computed location requires retained sparse sector topology");
    let slot = usize::from(sparse_slot);
    let public = retained
        .position_sector_numbers
        .get(slot)
        .copied()
        .flatten()
        .unwrap_or_else(|| {
            panic!("legacy computed location references non-position sector slot {sparse_slot}")
        });
    let arena = retained
        .position_sector_indices
        .get(slot)
        .copied()
        .flatten()
        .unwrap_or_else(|| {
            panic!("legacy computed location sector slot {sparse_slot} has no arena identity")
        });
    let public =
        u16::try_from(public).expect("legacy computed location sector has negative public number");
    SectorHandle::new(public)
        .expect("legacy computed location sector equals null sentinel")
        .with_arena_index(arena)
}

/// Exact Original jump-line pointer space.
///
/// Original serializes a line pointer as its layer plus its ordinal inside
/// that layer's combined `RHLine` array. Rust stores jump lines separately,
/// so neither that ordinal nor jump-only load order is a runtime identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacyLineTopology {
    by_original_identity: BTreeMap<(u16, i16), JumpLineIndex>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LegacyLineTopologyError {
    #[error("initialized jump-line runtime index {index} exceeds u32")]
    RuntimeIndexOverflow { index: usize },
    #[error("initialized jump-line runtime index equals the null sentinel")]
    RuntimeIndexNullSentinel,
    #[error("initialized jump-line layer {layer} contains more than i16::MAX entries")]
    LayerIndexOverflow { layer: u16 },
    #[error(
        "saved jump-line field {field} has inconsistent null identity layer={layer:?}, index={index:?}"
    )]
    InconsistentNull {
        field: &'static str,
        layer: Option<u16>,
        index: Option<i16>,
    },
    #[error(
        "saved jump-line field {field} references missing Original line layer {layer}, index {index}"
    )]
    Missing {
        field: &'static str,
        layer: u16,
        index: i16,
    },
    #[error(
        "saved jump-line field {field} references shifted Original line layer {layer}, index {index}, but owner {owner} and primary target {target} do not identify a reciprocal table-swordfight line"
    )]
    MissingGeometryIdentity {
        field: &'static str,
        layer: u16,
        index: i16,
        owner: u32,
        target: u32,
    },
    #[error(
        "saved jump-line field {field} references shifted Original line layer {layer}, index {index}, but owner {owner} and primary target {target} ambiguously identify runtime lines {candidates:?}"
    )]
    AmbiguousGeometryIdentity {
        field: &'static str,
        layer: u16,
        index: i16,
        owner: u32,
        target: u32,
        candidates: Vec<u32>,
    },
    #[error("initialized mission retained {retained} jump-line identities for {runtime} lines")]
    RetainedIdentityCountMismatch { retained: usize, runtime: usize },
    #[error(
        "initialized mission retained duplicate jump-line identity layer {layer}, index {index}"
    )]
    DuplicateIdentity { layer: u16, index: i16 },
}

impl LegacyLineTopology {
    /// Reconstruct exact `(layer, combined-line ordinal)` identities retained
    /// while the initialized mission's complete line arrays were built.
    pub fn derive(
        engine: &EngineInner,
        assets: &LevelAssets,
    ) -> Result<Self, LegacyLineTopologyError> {
        let runtime = engine.world.fast_grid.level.jump_lines.len();
        let Some(retained) = assets.legacy_grid_topology.as_ref() else {
            if runtime == 0 {
                return Ok(Self::default());
            }
            return Err(LegacyLineTopologyError::RetainedIdentityCountMismatch {
                retained: 0,
                runtime,
            });
        };
        if retained.jump_line_identities.len() != runtime {
            return Err(LegacyLineTopologyError::RetainedIdentityCountMismatch {
                retained: retained.jump_line_identities.len(),
                runtime,
            });
        }
        Self::derive_from_identities(retained.jump_line_identities.iter().copied())
    }

    fn derive_from_identities(
        identities: impl IntoIterator<Item = (u16, i16)>,
    ) -> Result<Self, LegacyLineTopologyError> {
        let mut by_original_identity = BTreeMap::new();
        for (runtime_index, identity) in identities.into_iter().enumerate() {
            let raw_runtime_index = u32::try_from(runtime_index).map_err(|_| {
                LegacyLineTopologyError::RuntimeIndexOverflow {
                    index: runtime_index,
                }
            })?;
            let handle = JumpLineIndex::new(raw_runtime_index)
                .ok_or(LegacyLineTopologyError::RuntimeIndexNullSentinel)?;
            if by_original_identity.insert(identity, handle).is_some() {
                return Err(LegacyLineTopologyError::DuplicateIdentity {
                    layer: identity.0,
                    index: identity.1,
                });
            }
        }
        Ok(Self {
            by_original_identity,
        })
    }

    #[cfg(test)]
    fn derive_from_layers(
        layers: impl IntoIterator<Item = u16>,
    ) -> Result<Self, LegacyLineTopologyError> {
        let mut next_in_layer = BTreeMap::<u16, i16>::new();
        let mut by_original_identity = BTreeMap::new();
        for (runtime_index, layer) in layers.into_iter().enumerate() {
            let index_in_layer = next_in_layer.entry(layer).or_default();
            let raw_runtime_index = u32::try_from(runtime_index).map_err(|_| {
                LegacyLineTopologyError::RuntimeIndexOverflow {
                    index: runtime_index,
                }
            })?;
            let handle = JumpLineIndex::new(raw_runtime_index)
                .ok_or(LegacyLineTopologyError::RuntimeIndexNullSentinel)?;
            by_original_identity.insert((layer, *index_in_layer), handle);
            *index_in_layer = index_in_layer
                .checked_add(1)
                .ok_or(LegacyLineTopologyError::LayerIndexOverflow { layer })?;
        }
        Ok(Self {
            by_original_identity,
        })
    }

    pub fn resolve(
        &self,
        field: &'static str,
        reference: LegacyLineRef,
    ) -> Result<Option<JumpLineIndex>, LegacyLineTopologyError> {
        match (reference.layer, reference.index) {
            (None, None) => Ok(None),
            (Some(layer), Some(index)) if index >= 0 => self
                .by_original_identity
                .get(&(layer, index))
                .copied()
                .map(Some)
                .ok_or(LegacyLineTopologyError::Missing {
                    field,
                    layer,
                    index,
                }),
            (layer, index) => Err(LegacyLineTopologyError::InconsistentNull {
                field,
                layer,
                index,
            }),
        }
    }

    /// Resolve a shifted retail `RHArtificialMalignity::mpMyLineJump` from
    /// the same owner/primary-target geometry that authored it.
    ///
    /// `IsTableSwordfightNeeded` finds a line in the primary target's sector
    /// whose reciprocal lives in the owner sector, then returns that
    /// reciprocal owner-side line. A shifted combined-line ordinal is not an
    /// identity, so equal-distance candidates are rejected rather than using
    /// runtime load order as an accidental tie-break.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_enemy_jump_line(
        &self,
        field: &'static str,
        reference: LegacyLineRef,
        fast_grid: &crate::fast_find_grid::FastFindGrid,
        owner: u32,
        owner_sector: crate::position_interface::SectorHandle,
        target: u32,
        target_sector: crate::position_interface::SectorHandle,
        target_position: crate::coordinates::MapPoint,
        maximal_sword_range: f32,
    ) -> Result<Option<JumpLineIndex>, LegacyLineTopologyError> {
        let (layer, index) = match (reference.layer, reference.index) {
            (None, None) => return Ok(None),
            (Some(layer), Some(index)) if index >= 0 => (layer, index),
            (layer, index) => {
                return Err(LegacyLineTopologyError::InconsistentNull {
                    field,
                    layer,
                    index,
                });
            }
        };
        if let Some(line) = self.by_original_identity.get(&(layer, index)).copied() {
            return Ok(Some(line));
        }

        let Some(owner_sector_index) = owner_sector.arena_index() else {
            return Err(LegacyLineTopologyError::MissingGeometryIdentity {
                field,
                layer,
                index,
                owner,
                target,
            });
        };
        let Some(target_sector_index) = target_sector.arena_index() else {
            return Err(LegacyLineTopologyError::MissingGeometryIdentity {
                field,
                layer,
                index,
                owner,
                target,
            });
        };
        if owner_sector_index == target_sector_index {
            return Err(LegacyLineTopologyError::MissingGeometryIdentity {
                field,
                layer,
                index,
                owner,
                target,
            });
        }
        let Some(target_sector_data) = fast_grid
            .level
            .sectors
            .get(usize::from(target_sector_index))
        else {
            return Err(LegacyLineTopologyError::MissingGeometryIdentity {
                field,
                layer,
                index,
                owner,
                target,
            });
        };

        let mut candidates = Vec::<(JumpLineIndex, f32)>::new();
        for &target_line_index in &target_sector_data.jump_line_indices {
            let Some(target_line) = fast_grid
                .level
                .jump_lines
                .get(usize::from(target_line_index))
            else {
                continue;
            };
            let Some(owner_line_raw) = target_line.associated_line_index else {
                continue;
            };
            let Some(owner_line_index) = JumpLineIndex::new(owner_line_raw) else {
                continue;
            };
            let Some(owner_line) = fast_grid.level.jump_lines.get(owner_line_raw as usize) else {
                continue;
            };
            if owner_line.associated_line_index != Some(target_line_index.get())
                || owner_line.layer != layer
                || owner_line.sector_index != Some(owner_sector_index)
            {
                continue;
            }
            let target_distance = target_line.compute_distance(target_position);
            if target_distance >= maximal_sword_range {
                continue;
            }
            candidates.push((owner_line_index, target_distance));
        }
        candidates.sort_by(|left, right| left.1.total_cmp(&right.1));
        let Some(&(best, best_distance)) = candidates.first() else {
            return Err(LegacyLineTopologyError::MissingGeometryIdentity {
                field,
                layer,
                index,
                owner,
                target,
            });
        };
        let tied = candidates
            .iter()
            .take_while(|(_, distance)| *distance == best_distance)
            .map(|(line, _)| line.get())
            .collect::<Vec<_>>();
        if tied.len() != 1 {
            return Err(LegacyLineTopologyError::AmbiguousGeometryIdentity {
                field,
                layer,
                index,
                owner,
                target,
                candidates: tied,
            });
        }
        let owner_line = &fast_grid.level.jump_lines[usize::from(best)];
        let target_line = &fast_grid.level.jump_lines[owner_line
            .associated_line_index
            .expect("candidate was reciprocal")
            as usize];
        if (owner_line.z_a - target_line.z_a).abs() > 40.0 {
            return Err(LegacyLineTopologyError::MissingGeometryIdentity {
                field,
                layer,
                index,
                owner,
                target,
            });
        }
        let owner_mid = owner_line.get_middle_point();
        let target_mid = target_line.get_middle_point();
        let middle_distance =
            ((owner_mid.x - target_mid.x).powi(2) + (owner_mid.y - target_mid.y).powi(2)).sqrt();
        if middle_distance + best_distance > maximal_sword_range {
            return Err(LegacyLineTopologyError::MissingGeometryIdentity {
                field,
                layer,
                index,
                owner,
                target,
            });
        }
        Ok(Some(best))
    }
}

/// Complete translation between the two Original reference spaces and stable
/// Rust entity IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyEntityFixups {
    pub by_creation_order: BTreeMap<u32, EntityId>,
    /// `None` is an Original mobile master, whose runtime identity lives in
    /// `mobile_by_creation_order` instead of Rust's entity arena.
    pub by_saved_slot: Vec<Option<EntityId>>,
    pub creation_order_by_entity: BTreeMap<EntityId, u32>,
    pub mobile_by_creation_order: BTreeMap<u32, usize>,
    /// Mobile masters execute sequence commands through their first masked FX
    /// child in Rust's sequence runtime.
    pub mobile_owner_by_creation_order: BTreeMap<u32, EntityId>,
}

impl LegacyEntityFixups {
    /// Preflight the saved element array against one initialized mission.
    ///
    /// This slice deliberately refuses dynamic elements and mobile masters:
    /// returning a partial map would let later state conversion silently bind
    /// references to the wrong entity. Their exact constructors are added by
    /// subsequent adoption stages.
    pub fn build(
        envelope: &LegacyElementEnvelope,
        topology: &LegacyStaticElementTopology,
    ) -> Result<Self, LegacySaveAdoptError> {
        let mut initialized_by_creation_order = BTreeMap::new();
        for (&entity_id, &creation_order) in &topology.creation_order_by_entity {
            initialized_by_creation_order.insert(creation_order, entity_id);
        }

        let mut by_creation_order = BTreeMap::new();
        let mut by_saved_slot = Vec::with_capacity(envelope.records.len());
        let mut creation_order_by_entity = BTreeMap::new();

        for record in &envelope.records {
            let entity_id = match record.resolution {
                LegacyElementResolution::ResolveStatic { .. } => {
                    match initialized_by_creation_order
                        .get(&record.creation_order)
                        .copied()
                    {
                        Some(entity_id) => entity_id,
                        None => {
                            let is_mobile = topology
                                .payload_metadata
                                .by_creation_order
                                .get(&record.creation_order)
                                .is_some_and(|metadata| {
                                    metadata.class == LegacyElementClass::Mobile
                                });
                            if is_mobile {
                                by_saved_slot.push(None);
                                continue;
                            }
                            return Err(LegacySaveAdoptError::MissingStaticEntity {
                                slot: record.slot,
                                creation_order: record.creation_order,
                                class: record.class,
                            });
                        }
                    }
                }
                LegacyElementResolution::ConstructDynamic { .. } => {
                    return Err(LegacySaveAdoptError::UnsupportedDynamicElement {
                        slot: record.slot,
                        creation_order: record.creation_order,
                        class: record.class,
                    });
                }
            };

            by_creation_order.insert(record.creation_order, entity_id);
            by_saved_slot.push(Some(entity_id));
            if let Some(first_creation_order) =
                creation_order_by_entity.insert(entity_id, record.creation_order)
            {
                return Err(LegacySaveAdoptError::DuplicateInitializedEntity {
                    entity_id,
                    first_creation_order,
                    second_creation_order: record.creation_order,
                });
            }
        }

        Ok(Self {
            by_creation_order,
            by_saved_slot,
            creation_order_by_entity,
            mobile_by_creation_order: topology.mobile_index_by_creation_order.clone(),
            mobile_owner_by_creation_order: BTreeMap::new(),
        })
    }

    pub fn resolve_element(
        &self,
        reference: LegacyElementRef,
    ) -> Result<Option<EntityId>, LegacySaveAdoptError> {
        reference
            .0
            .map(|creation_order| {
                self.by_creation_order
                    .get(&creation_order)
                    .copied()
                    .ok_or(LegacySaveAdoptError::MissingCreationOrderReference { creation_order })
            })
            .transpose()
    }

    /// Resolve the AI serializer's u16 `marrayElements` index.
    pub fn resolve_ai_element(
        &self,
        reference: LegacyAiElementRef,
    ) -> Result<Option<EntityId>, LegacySaveAdoptError> {
        reference
            .0
            .map(|slot| {
                self.by_saved_slot
                    .get(usize::from(slot))
                    .copied()
                    .ok_or(LegacySaveAdoptError::MissingAiElementSlot {
                        slot,
                        element_count: self.by_saved_slot.len(),
                    })?
                    .ok_or(LegacySaveAdoptError::MobileMasterAiReference { slot })
            })
            .transpose()
    }
}

/// Derive and validate the complete entity-reference plan without mutating the
/// initialized engine.
pub fn preflight_initialized_v48_adoption(
    engine: &EngineInner,
    assets: &LevelAssets,
    body: &LegacySaveBody,
) -> Result<LegacyEntityFixups, LegacySaveAdoptError> {
    let topology = derive_static_element_topology(engine, assets)?;
    LegacyEntityFixups::build(&body.element_envelope, &topology)
}

/// Reconstruct the mission-created arrays used by position pointer fixups.
///
/// Original provenance:
///
/// - `RHFastFindGrid::InitializeSightObstaclesFromProtoStream` inserts a
///   synthetic flat ground into `marrayProjectionAreas` before every authored
///   projection obstacle.
/// - `SerializeSightObstaclePointer` stores `RHSightObstacle::mulID - 1`.
///   Authored obstacles are constructed immediately after that synthetic
///   ground, so Rust's retained zero-based authored obstacle ID is the saved
///   sight-obstacle index.
/// - `SerializeGatePointer` indexes the complete `marrayGates`; Rust retains
///   that same array in `interactables.doors`.
/// - sector pointers store their sparse `marraySectors` slot directly.
fn build_position_sector_identities(
    retained: &crate::engine::LegacyGridTopologyAssets,
    runtime_sectors: &[GridSector],
) -> Result<Vec<Option<LegacyPositionSectorIdentity>>, LegacySaveAdoptError> {
    if retained.position_sector_numbers.len() != retained.sectors.len()
        || retained.position_sector_indices.len() != retained.sectors.len()
    {
        return Err(position_topology_detail(format!(
            "retained position-sector arrays disagree: objects={}, public_numbers={}, runtime_indices={}",
            retained.sectors.len(),
            retained.position_sector_numbers.len(),
            retained.position_sector_indices.len(),
        )));
    }

    let mut claimed = std::collections::BTreeMap::new();
    retained
        .position_sector_numbers
        .iter()
        .copied()
        .zip(retained.position_sector_indices.iter().copied())
        .enumerate()
        .map(|(sparse_slot, (number, runtime_index))| match (number, runtime_index) {
            (None, None) => Ok(None),
            (Some(_), None) | (None, Some(_)) => Err(position_topology_detail(format!(
                "Original sparse sector slot {sparse_slot} has only one half of its public/runtime identity"
            ))),
            (Some(number), Some(runtime_index)) => {
                if let Some(previous_slot) = claimed.insert(runtime_index, sparse_slot) {
                    return Err(position_topology_detail(format!(
                        "Original sparse sector slots {previous_slot} and {sparse_slot} both map to runtime sector index {runtime_index}"
                    )));
                }
                let runtime = runtime_sectors.get(usize::from(runtime_index)).ok_or_else(|| {
                    position_topology_detail(format!(
                        "Original sparse sector slot {sparse_slot} maps to absent runtime sector index {runtime_index} (runtime count {})",
                        runtime_sectors.len()
                    ))
                })?;
                if i16::from(runtime.sector_number) != number {
                    return Err(position_topology_detail(format!(
                        "Original sparse sector slot {sparse_slot} maps to runtime sector index {runtime_index}, whose public number {} differs from retained {number}",
                        i16::from(runtime.sector_number)
                    )));
                }
                let raw = u16::try_from(number).map_err(|_| {
                    position_topology_detail(format!(
                        "runtime position-sector number {number} is negative"
                    ))
                })?;
                let public = SectorHandle::new(raw).ok_or_else(|| {
                    position_topology_detail(
                        "runtime position-sector number equals null sentinel 0xffff",
                    )
                })?;
                Ok(Some(LegacyPositionSectorIdentity {
                    public,
                    runtime_index,
                }))
            }
        })
        .collect()
}

pub fn derive_position_topology(
    engine: &EngineInner,
    assets: &LevelAssets,
) -> Result<LegacyPositionTopology, LegacySaveAdoptError> {
    let retained = assets.legacy_grid_topology.as_ref().ok_or({
        LegacySaveAdoptError::Topology(LegacyTopologyAdapterError::MissingRetainedFact {
            fact: LegacyMissingTopologyFact::GridSparseSectorOrder,
            original_owner: "RHFastFindGrid construction-time arrays",
            detail: "position adoption requires the retained sparse sector and gate arrays",
        })
    })?;
    let gate_order =
        derive_legacy_gate_order(&retained.gates, &engine.script_domains.interactables.doors)
            .map_err(|error| position_topology_detail(error.to_string()))?;
    let sector_identities =
        build_position_sector_identities(retained, &engine.world.fast_grid.level.sectors)?;
    // Compatibility surface until every RHposition consumer accepts the
    // paired identity. Do not reconstruct `runtime_index` from this vector;
    // the retained `sector_identities` above is the authoritative mapping.
    let sectors = sector_identities
        .iter()
        .copied()
        .map(|identity| identity.map(|identity| identity.public))
        .collect::<Vec<_>>();
    let sector_indices = sector_identities
        .into_iter()
        .map(|identity| identity.map(|identity| identity.runtime_index))
        .collect();
    let sector_doors = retained
        .sectors
        .iter()
        .map(|kind| {
            let crate::engine::LegacyGridSectorAsset::Door { gate_index } = kind else {
                return Ok(None);
            };
            let runtime = gate_order.get(*gate_index as usize).ok_or_else(|| {
                position_topology_detail(format!(
                    "sparse door sector references missing Original gate index {gate_index}"
                ))
            })?;
            if !matches!(
                retained.gates.get(*gate_index as usize),
                Some(crate::engine::LegacyGridGateAsset::Door)
            ) {
                return Err(position_topology_detail(format!(
                    "sparse door sector references non-door Original gate index {gate_index}"
                )));
            }
            Ok(Some(DoorHandle(runtime.0)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    build_position_topology(
        sectors,
        sector_indices,
        sector_doors,
        &gate_order,
        assets.static_sight_obstacles.as_slice(),
    )
}

fn build_position_topology(
    sectors: Vec<Option<SectorHandle>>,
    sector_indices: Vec<Option<SectorIndex>>,
    sector_doors: Vec<Option<DoorHandle>>,
    gate_order: &[crate::gate::DoorIndex],
    obstacles: &[crate::sight_obstacle::SightObstacle],
) -> Result<LegacyPositionTopology, LegacySaveAdoptError> {
    if sector_indices.len() != sectors.len() {
        return Err(position_topology_detail(format!(
            "position topology has {} public sector slots but {} runtime-index slots",
            sectors.len(),
            sector_indices.len()
        )));
    }
    let doors = gate_order
        .iter()
        .map(|index| DoorHandle(index.0))
        .collect::<Vec<_>>();

    // Rust stores only authored obstacles. Place each binding by the retained
    // zero-based source ID rather than assuming storage order happens to
    // remain identical.
    let mut authored_obstacles = vec![None; obstacles.len()];
    for (runtime_index, obstacle) in obstacles.iter().enumerate() {
        let saved_index = usize::try_from(obstacle.id)
            .map_err(|_| position_topology_detail("authored obstacle ID exceeds usize"))?;
        if saved_index >= authored_obstacles.len() {
            return Err(position_topology_detail(format!(
                "authored obstacle ID {} is outside the retained 0..{} sight-obstacle space",
                obstacle.id,
                authored_obstacles.len()
            )));
        }
        let runtime_index = u16::try_from(runtime_index)
            .map_err(|_| position_topology_detail("runtime authored obstacle index exceeds u16"))?;
        let obstacle_handle = ObstacleHandle::new(runtime_index).ok_or_else(|| {
            position_topology_detail("runtime authored obstacle index equals null sentinel 0xffff")
        })?;
        let binding = LegacyPositionObstacleBinding {
            obstacle: Some(obstacle_handle),
            plane: PlaneZCoeffs::from_plane_points(&obstacle.top_plane_points),
        };
        if authored_obstacles[saved_index]
            .replace((binding, obstacle.is_projection_area()))
            .is_some()
        {
            return Err(position_topology_detail(format!(
                "duplicate authored obstacle ID {}",
                obstacle.id
            )));
        }
    }
    let authored_obstacles = authored_obstacles
        .into_iter()
        .enumerate()
        .map(|(index, obstacle)| {
            obstacle.ok_or_else(|| {
                position_topology_detail(format!(
                    "authored obstacle ID space has no entry at saved index {index}"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let ground = LegacyPositionObstacleBinding {
        obstacle: None,
        plane: PlaneZCoeffs {
            az: 0.0,
            bz: 0.0,
            dz: 0.0,
        },
    };
    let projection_areas = std::iter::once(ground)
        .chain(
            authored_obstacles
                .iter()
                .copied()
                .filter_map(|(binding, projection)| projection.then_some(binding)),
        )
        .collect();
    let sight_obstacles = authored_obstacles
        .into_iter()
        .map(|(binding, _)| binding)
        .collect();

    Ok(LegacyPositionTopology {
        sectors,
        sector_indices,
        sector_doors,
        doors,
        projection_areas,
        sight_obstacles,
    })
}

fn position_topology_detail(detail: impl Into<String>) -> LegacySaveAdoptError {
    LegacySaveAdoptError::InvalidPositionTopology {
        detail: detail.into(),
    }
}

/// Validate and normalize one serialized position without mutating its owner.
///
/// The returned value is accepted infallibly by
/// `PositionInterface::restore_v48_serialized_state`. This split ensures a
/// bad enum or pointer cannot leave an entity half-restored.
pub(crate) fn preflight_v48_position(
    payload: &LegacyPositionPayload,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
) -> Result<PositionInterfaceV48State, LegacySaveAdoptError> {
    let computed_position =
        PositionComputed::from_bits(u8::try_from(payload.computed_position).map_err(|_| {
            invalid_position(
                "computed_position",
                payload.computed_position,
                "RHpositionComputed bit mask 0..7",
            )
        })?)
        .ok_or_else(|| {
            invalid_position(
                "computed_position",
                payload.computed_position,
                "RHpositionComputed bit mask 0..7",
            )
        })?;
    let computed_increment =
        IncrementComputed::from_bits(u8::try_from(payload.computed_increment).map_err(|_| {
            invalid_position(
                "computed_increment",
                payload.computed_increment,
                "RHincrementComputed bit mask 0..7",
            )
        })?)
        .ok_or_else(|| {
            invalid_position(
                "computed_increment",
                payload.computed_increment,
                "RHincrementComputed bit mask 0..7",
            )
        })?;
    // `CHECKENUM` copies the complete four-byte C++ enum storage without
    // validating it. Projectile Hourglass can copy a trajectory sentinel or
    // indeterminate trajectory material into RHPositionInterface before a
    // save, so preserve the raw bits. PositionInterface validates only if
    // gameplay later consumes the value as a material.
    let material = payload.material;
    let posture = crate::element::Posture::try_from(payload.posture).map_err(|_| {
        invalid_position(
            "posture",
            payload.posture,
            "serialized RHposture ordinal 0..24",
        )
    })?;
    let old_posture = crate::element::Posture::try_from(payload.old_posture).map_err(|_| {
        invalid_position(
            "old_posture",
            payload.old_posture,
            "serialized RHposture ordinal 0..24",
        )
    })?;
    let direction = checked_direction("direction", payload.direction)?;
    let direction_goal = checked_direction("direction_goal", payload.direction_goal)?;
    let sector = checked_sector("sector", payload.sector.0, &topology.sectors)?;
    let sector_index = checked_sector_index("sector", payload.sector.0, &topology.sector_indices)?;
    let sector_goal = checked_sector("sector_goal", payload.sector_goal.0, &topology.sectors)?;
    let sector_goal_index = checked_sector_index(
        "sector_goal",
        payload.sector_goal.0,
        &topology.sector_indices,
    )?;
    let door = checked_index("door", payload.door.0, &topology.doors)?
        .copied()
        .unwrap_or(DoorHandle::NULL);
    let obstacle_space = if payload.layer == u16::MAX {
        &topology.sight_obstacles
    } else {
        &topology.projection_areas
    };
    let obstacle_binding = checked_index("obstacle", payload.obstacle.0, obstacle_space)?.copied();

    Ok(PositionInterfaceV48State {
        computed_position,
        computed_increment,
        material,
        posture,
        old_posture,
        direction,
        direction_goal,
        slow_turn_count: payload.slow_turn_count,
        layer: Layer::from_saved_raw(payload.layer),
        layer_goal: Layer::from_saved_raw(payload.layer_goal),
        tolerance: payload.tolerance,
        directional_tolerance: payload.directional_tolerance,
        accumulate_movement_map: payload.accumulate_movement_map,
        anti_collision_on: payload.anti_collision_on,
        goal_next_valid: payload.goal_next_valid,
        deviated: payload.deviated,
        direction_count: payload.direction_count,
        door_direction: payload.door_direction,
        reversed_movement: payload.reversed_movement,
        blocked_count: payload.blocked_count,
        radius: payload.radius,
        use_emergency_lying_box: payload.use_emergency_lying_box,
        sector,
        sector_index,
        sector_goal,
        sector_goal_index,
        door,
        obstacle: obstacle_binding.and_then(|binding| binding.obstacle),
        plane: obstacle_binding.map(|binding| binding.plane),
        target_element: entities.resolve_element(payload.target_element)?,
        position: point3(payload.position),
        map: point2(payload.map),
        sprite: point2(payload.sprite),
        old_position: point3(payload.old_position),
        old_map: point2(payload.old_map),
        old_sprite: point2(payload.old_sprite),
        goal_map: point2(payload.goal_map),
        goal_next_map: point2(payload.goal_next_map),
        goal: point3(payload.goal),
        increment: vector3(payload.increment),
        increment_map: vector2(payload.increment_map),
        accumulated_movement_map: vector2(payload.accumulated_movement_map),
        forecasted_movement: vector3(payload.forecasted_movement),
        move_box_map: bounding_box(payload.move_box_map),
        blocked_box: bounding_box(payload.blocked_box),
    })
}

fn invalid_position(
    field: &'static str,
    value: impl std::fmt::Display,
    expected: &'static str,
) -> LegacySaveAdoptError {
    LegacySaveAdoptError::InvalidPositionField {
        field,
        value: value.to_string(),
        expected,
    }
}

fn checked_direction(field: &'static str, raw: i16) -> Result<Direction, LegacySaveAdoptError> {
    if !(0..16).contains(&raw) {
        return Err(invalid_position(field, raw, "direction sector 0..15"));
    }
    Ok(Direction::from_raw(i32::from(raw)))
}

fn checked_sector(
    field: &'static str,
    raw: Option<u16>,
    sectors: &[Option<SectorHandle>],
) -> Result<Option<SectorHandle>, LegacySaveAdoptError> {
    let Some(index) = raw else {
        return Ok(None);
    };
    let Some(sector) = sectors.get(usize::from(index)) else {
        return Err(LegacySaveAdoptError::MissingPositionTopologyEntry {
            field,
            index: usize::from(index),
            count: sectors.len(),
        });
    };
    (*sector)
        .map(Some)
        .ok_or(LegacySaveAdoptError::UnmappedPositionSector {
            field,
            index: usize::from(index),
        })
}

fn checked_sector_index(
    field: &'static str,
    raw: Option<u16>,
    sectors: &[Option<SectorIndex>],
) -> Result<Option<SectorIndex>, LegacySaveAdoptError> {
    let Some(index) = raw else {
        return Ok(None);
    };
    let Some(sector) = sectors.get(usize::from(index)) else {
        return Err(LegacySaveAdoptError::MissingPositionTopologyEntry {
            field,
            index: usize::from(index),
            count: sectors.len(),
        });
    };
    (*sector)
        .map(Some)
        .ok_or(LegacySaveAdoptError::UnmappedPositionSector {
            field,
            index: usize::from(index),
        })
}

fn checked_index<'a, T>(
    field: &'static str,
    raw: Option<i16>,
    values: &'a [T],
) -> Result<Option<&'a T>, LegacySaveAdoptError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let index = usize::try_from(raw)
        .map_err(|_| invalid_position(field, raw, "non-negative initialized-array index"))?;
    values
        .get(index)
        .map(Some)
        .ok_or(LegacySaveAdoptError::MissingPositionTopologyEntry {
            field,
            index,
            count: values.len(),
        })
}

fn point2(value: LegacyPoint2) -> MapPoint {
    MapPoint::new(value.x, value.y)
}

fn vector2(value: LegacyPoint2) -> MapVec {
    MapVec::new(value.x, value.y)
}

fn point3(value: LegacyPoint3) -> WorldPoint3D {
    WorldPoint3D::new(value.x, value.y, value.z)
}

fn vector3(value: LegacyPoint3) -> WorldVec3D {
    WorldVec3D::new(value.x, value.y, value.z)
}

fn bounding_box(value: LegacyBoundingBox2) -> MapBBox {
    if value.bounds_are_set {
        MapBBox::from_coords(
            value.top_left.x,
            value.top_left.y,
            value.bottom_right.x,
            value.bottom_right.y,
        )
    } else {
        MapBBox::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::EntityIdKind;
    use crate::legacy_save::{
        elements::{LegacyElementFixupTable, LegacyElementRecord, LegacyElementResolution},
        payload_base::{LegacySectorRef, LegacySignedIndexRef},
        payload_context::{LegacyElementPayloadMetadata, LegacyMissionPayloadMetadata},
    };
    use crate::position_interface::PositionInterface;

    fn identity_sectors(count: usize) -> Vec<Option<SectorHandle>> {
        (0..count)
            .map(|index| {
                Some(
                    SectorHandle::new(u16::try_from(index).expect("test sector index exceeds u16"))
                        .expect("test sector index equals null sentinel"),
                )
            })
            .collect()
    }

    fn identity_sector_indices(count: usize) -> Vec<Option<SectorIndex>> {
        (0..count)
            .map(|index| {
                SectorIndex::new(u32::try_from(index).expect("test sector index exceeds u32"))
            })
            .collect()
    }

    fn test_grid_sector(public_number: i16) -> GridSector {
        GridSector {
            points: Vec::new(),
            bounding_box: MapBBox::new(),
            sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            layer: 0,
            sector_number: crate::sector::SectorNumber::new(public_number),
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
    fn position_sector_identity_survives_overlapping_public_numbers() {
        let retained = crate::engine::LegacyGridTopologyAssets {
            sectors: vec![
                crate::engine::LegacyGridSectorAsset::NullOrOrdinary,
                crate::engine::LegacyGridSectorAsset::NullOrOrdinary,
                crate::engine::LegacyGridSectorAsset::NullOrOrdinary,
            ],
            position_sector_numbers: vec![Some(18), None, Some(18)],
            position_sector_indices: vec![SectorIndex::new(0), None, SectorIndex::new(1)],
            ..Default::default()
        };
        let runtime = vec![test_grid_sector(18), test_grid_sector(18)];

        let identities = build_position_sector_identities(&retained, &runtime).unwrap();
        let first = identities[0].unwrap();
        let second = identities[2].unwrap();

        assert_eq!(first.public, second.public);
        assert_ne!(first.runtime_index, second.runtime_index);
        assert_eq!(first.runtime_index, SectorIndex::new(0).unwrap());
        assert_eq!(second.runtime_index, SectorIndex::new(1).unwrap());
    }

    fn fixture() -> (LegacyElementEnvelope, LegacyStaticElementTopology, EntityId) {
        let entity_id = EntityId::new(9, EntityIdKind::Fx);
        let creation_order = 31;
        let record = LegacyElementRecord {
            slot: 0,
            class: LegacyElementClass::Fx,
            creation_order,
            pc_description_index: None,
            resolution: LegacyElementResolution::ResolveStatic {
                fallback_factory: None,
            },
            creation_order_offset: 0,
        };
        let envelope = LegacyElementEnvelope {
            start_offset: 0,
            phase2_offset: 0,
            records: vec![record],
            fixups: LegacyElementFixupTable {
                by_creation_order: BTreeMap::from([(creation_order, 0)]),
            },
        };
        let topology = LegacyStaticElementTopology {
            payload_metadata: LegacyMissionPayloadMetadata {
                static_creation_order_boundary: 32,
                by_creation_order: BTreeMap::from([(
                    creation_order,
                    LegacyElementPayloadMetadata {
                        class: LegacyElementClass::Fx,
                        script_class: None,
                        local_ai_kind: None,
                        mobile_sprite_count: None,
                    },
                )]),
            },
            creation_order_by_entity: BTreeMap::from([(entity_id, creation_order)]),
            mobile_index_by_creation_order: BTreeMap::new(),
            static_creation_order_boundary: 32,
        };
        (envelope, topology, entity_id)
    }

    #[test]
    fn maps_creation_order_and_ai_slot_spaces_independently() {
        let (envelope, topology, entity_id) = fixture();
        let fixups = LegacyEntityFixups::build(&envelope, &topology).unwrap();

        assert_eq!(
            fixups.resolve_element(LegacyElementRef(Some(31))).unwrap(),
            Some(entity_id)
        );
        assert_eq!(
            fixups
                .resolve_ai_element(LegacyAiElementRef(Some(0)))
                .unwrap(),
            Some(entity_id)
        );
        assert_eq!(
            fixups.resolve_element(LegacyElementRef(None)).unwrap(),
            None
        );
    }

    #[test]
    fn rejects_unknown_reference_in_each_original_id_space() {
        let (envelope, topology, _) = fixture();
        let fixups = LegacyEntityFixups::build(&envelope, &topology).unwrap();

        assert!(matches!(
            fixups.resolve_element(LegacyElementRef(Some(999))),
            Err(LegacySaveAdoptError::MissingCreationOrderReference {
                creation_order: 999
            })
        ));
        assert!(matches!(
            fixups.resolve_ai_element(LegacyAiElementRef(Some(1))),
            Err(LegacySaveAdoptError::MissingAiElementSlot {
                slot: 1,
                element_count: 1
            })
        ));
    }

    fn point2(x: f32, y: f32) -> LegacyPoint2 {
        LegacyPoint2 { x, y }
    }

    fn point3(x: f32, y: f32, z: f32) -> LegacyPoint3 {
        LegacyPoint3 { x, y, z }
    }

    fn bbox(seed: f32, bounds_are_set: bool) -> LegacyBoundingBox2 {
        LegacyBoundingBox2 {
            top_left: point2(seed, seed + 1.0),
            bottom_right: point2(seed + 2.0, seed + 3.0),
            bounds_are_set,
        }
    }

    fn position_payload() -> LegacyPositionPayload {
        LegacyPositionPayload {
            computed_position: 5,
            computed_increment: 6,
            material: 10,
            posture: 17,
            old_posture: 3,
            direction: 15,
            direction_goal: 2,
            slow_turn_count: 9,
            layer: u16::MAX,
            layer_goal: 4,
            tolerance: 12.5,
            directional_tolerance: true,
            accumulate_movement_map: true,
            anti_collision_on: false,
            goal_next_valid: true,
            deviated: true,
            direction_count: -2,
            door_direction: true,
            reversed_movement: true,
            blocked_count: 7,
            radius: 4.25,
            use_emergency_lying_box: true,
            sector: LegacySectorRef(Some(1)),
            sector_goal: LegacySectorRef(Some(2)),
            door: LegacySignedIndexRef(Some(0)),
            obstacle: LegacySignedIndexRef(Some(0)),
            target_element: LegacyElementRef(Some(31)),
            position: point3(1.0, 2.0, 3.0),
            map: point2(4.0, 5.0),
            sprite: point2(6.0, 7.0),
            old_position: point3(8.0, 9.0, 10.0),
            old_map: point2(11.0, 12.0),
            old_sprite: point2(13.0, 14.0),
            goal_map: point2(15.0, 16.0),
            goal_next_map: point2(17.0, 18.0),
            goal: point3(19.0, 20.0, 21.0),
            increment: point3(22.0, 23.0, 24.0),
            increment_map: point2(25.0, 26.0),
            accumulated_movement_map: point2(27.0, 28.0),
            forecasted_movement: point3(29.0, 30.0, 31.0),
            move_box_map: bbox(32.0, true),
            blocked_box: bbox(36.0, false),
        }
    }

    fn obstacle(id: u32, projection_area: bool, z: f32) -> crate::sight_obstacle::SightObstacle {
        let mut obstacle = crate::sight_obstacle::SightObstacle::new(
            id,
            if projection_area {
                crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA
            } else {
                0
            },
        );
        obstacle.top_plane_points = [[0.0, 0.0, z], [10.0, 0.0, z], [0.0, 10.0, z]];
        obstacle
    }

    #[test]
    fn position_topology_keeps_original_ground_and_obstacle_index_spaces() {
        // Deliberately scramble Rust storage order. Saved sight pointers use
        // the retained authored ID (`mulID - 1`), while Rust handles still
        // address the current storage index.
        let obstacles = vec![obstacle(1, false, 9.0), obstacle(0, true, 4.0)];
        let topology = build_position_topology(
            identity_sectors(17),
            identity_sector_indices(17),
            vec![None; 17],
            &[
                crate::gate::DoorIndex(2),
                crate::gate::DoorIndex(0),
                crate::gate::DoorIndex(1),
            ],
            &obstacles,
        )
        .unwrap();

        assert_eq!(topology.sectors, identity_sectors(17));
        assert_eq!(topology.sector_indices, identity_sector_indices(17));
        assert_eq!(
            topology.doors,
            vec![DoorHandle(2), DoorHandle(0), DoorHandle(1)]
        );
        assert_eq!(
            topology.sight_obstacles,
            vec![
                LegacyPositionObstacleBinding {
                    obstacle: Some(ObstacleHandle::new(1).unwrap()),
                    plane: PlaneZCoeffs {
                        az: 0.0,
                        bz: 0.0,
                        dz: 4.0,
                    },
                },
                LegacyPositionObstacleBinding {
                    obstacle: Some(ObstacleHandle::new(0).unwrap()),
                    plane: PlaneZCoeffs {
                        az: 0.0,
                        bz: 0.0,
                        dz: 9.0,
                    },
                },
            ]
        );
        assert_eq!(
            topology.projection_areas,
            vec![
                LegacyPositionObstacleBinding {
                    obstacle: None,
                    plane: PlaneZCoeffs {
                        az: 0.0,
                        bz: 0.0,
                        dz: 0.0,
                    },
                },
                topology.sight_obstacles[0],
            ],
            "Original projection index zero is synthetic ground, followed by authored projection areas in mulID order"
        );
    }

    #[test]
    fn synthetic_ground_restores_a_plane_without_inventing_an_obstacle_handle() {
        let (envelope, element_topology, _) = fixture();
        let entities = LegacyEntityFixups::build(&envelope, &element_topology).unwrap();
        let topology = build_position_topology(
            identity_sectors(3),
            identity_sector_indices(3),
            vec![None; 3],
            &[crate::gate::DoorIndex(0)],
            &[],
        )
        .unwrap();
        let mut payload = position_payload();
        payload.layer = 0;
        payload.obstacle = LegacySignedIndexRef(Some(0));

        let saved = preflight_v48_position(&payload, &entities, &topology).unwrap();

        assert_eq!(saved.obstacle, None);
        assert_eq!(
            saved.plane,
            Some(PlaneZCoeffs {
                az: 0.0,
                bz: 0.0,
                dz: 0.0,
            })
        );
    }

    #[test]
    fn position_topology_rejects_non_isomorphic_obstacle_arrays() {
        let duplicate_ids = vec![obstacle(0, false, 1.0), obstacle(0, true, 2.0)];
        let obstacle_error =
            build_position_topology(Vec::new(), Vec::new(), Vec::new(), &[], &duplicate_ids)
                .unwrap_err();
        assert!(matches!(
            obstacle_error,
            LegacySaveAdoptError::InvalidPositionTopology { .. }
        ));
    }

    #[test]
    fn preflights_and_atomically_restores_every_v48_position_field() {
        let (envelope, topology, target) = fixture();
        let entities = LegacyEntityFixups::build(&envelope, &topology).unwrap();
        let projection = LegacyPositionObstacleBinding {
            obstacle: Some(ObstacleHandle::new(3).unwrap()),
            plane: PlaneZCoeffs {
                az: 1.0,
                bz: 2.0,
                dz: 3.0,
            },
        };
        let sight = LegacyPositionObstacleBinding {
            obstacle: Some(ObstacleHandle::new(4).unwrap()),
            plane: PlaneZCoeffs {
                az: 4.0,
                bz: 5.0,
                dz: 6.0,
            },
        };
        let saved = preflight_v48_position(
            &position_payload(),
            &entities,
            &LegacyPositionTopology {
                sectors: vec![
                    SectorHandle::new(0),
                    SectorHandle::new(42),
                    SectorHandle::new(7),
                ],
                sector_indices: identity_sector_indices(3),
                sector_doors: vec![None; 3],
                doors: vec![DoorHandle(9)],
                projection_areas: vec![projection],
                sight_obstacles: vec![sight],
            },
        )
        .unwrap();

        assert_eq!(saved.layer.get(), u16::MAX);
        assert_eq!(saved.sector.map(u16::from), Some(42));
        assert_eq!(saved.sector_index, SectorIndex::new(1));
        assert_eq!(saved.sector_goal.map(u16::from), Some(7));
        assert_eq!(saved.sector_goal_index, SectorIndex::new(2));
        assert_eq!(saved.obstacle, sight.obstacle);
        assert_eq!(saved.plane, Some(sight.plane));
        assert_eq!(saved.target_element, Some(target));
        assert_eq!(saved.posture, crate::element::Posture::StuckUnderNet);
        assert_eq!(saved.old_posture, crate::element::Posture::Lying);
        assert!(saved.blocked_box.0.is_none());

        let mut position = PositionInterface::new();
        position.set_pathfinder_index(77);
        position.restore_v48_serialized_state(saved.clone());

        assert_eq!(position.v48_serialized_state(), saved);
        assert_eq!(
            position.get_pathfinder_index(),
            77,
            "Original does not serialize mission-derived pathfinder indices"
        );
    }

    #[test]
    fn position_preserves_unchecked_material_bits_until_live_use() {
        let (envelope, topology, _) = fixture();
        let entities = LegacyEntityFixups::build(&envelope, &topology).unwrap();
        let mut payload = position_payload();
        payload.material = 217_559_952;
        payload.obstacle = LegacySignedIndexRef(None);

        let saved = preflight_v48_position(
            &payload,
            &entities,
            &LegacyPositionTopology {
                sectors: identity_sectors(3),
                sector_indices: identity_sector_indices(3),
                sector_doors: vec![None; 3],
                doors: vec![DoorHandle(9)],
                projection_areas: Vec::new(),
                sight_obstacles: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(saved.material, payload.material);

        let mut position = PositionInterface::new();
        position.restore_v48_serialized_state(saved);
        assert_eq!(position.v48_serialized_state().material, payload.material);

        assert!(
            crate::element::GameMaterial::try_from_u32(payload.material).is_none(),
            "raw material must remain invalid rather than be clamped"
        );

        position.set_material(crate::element::GameMaterial::Water);
        assert_eq!(position.get_material(), crate::element::GameMaterial::Water);
        assert_eq!(
            position.v48_serialized_state().material,
            crate::element::GameMaterial::Water.as_u32()
        );
    }

    #[test]
    fn preflight_rejects_bad_values_without_mutating_the_position() {
        let (envelope, topology, _) = fixture();
        let entities = LegacyEntityFixups::build(&envelope, &topology).unwrap();
        let mut payload = position_payload();
        payload.direction = 16;
        let mut position = PositionInterface::new();
        position.set_pathfinder_index(23);
        let before = position.clone();

        let error = preflight_v48_position(
            &payload,
            &entities,
            &LegacyPositionTopology {
                sectors: identity_sectors(3),
                sector_indices: identity_sector_indices(3),
                sector_doors: vec![None; 3],
                doors: vec![DoorHandle(9)],
                projection_areas: vec![],
                sight_obstacles: vec![],
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            LegacySaveAdoptError::InvalidPositionField {
                field: "direction",
                ..
            }
        ));
        assert_eq!(
            position.get_pathfinder_index(),
            before.get_pathfinder_index()
        );
        assert_eq!(position.map_position(), before.map_position());
    }

    #[test]
    fn obstacle_pointer_space_follows_the_saved_layer_sentinel() {
        let (envelope, topology, _) = fixture();
        let entities = LegacyEntityFixups::build(&envelope, &topology).unwrap();
        let projection = LegacyPositionObstacleBinding {
            obstacle: Some(ObstacleHandle::new(3).unwrap()),
            plane: PlaneZCoeffs {
                az: 1.0,
                bz: 2.0,
                dz: 3.0,
            },
        };
        let sight = LegacyPositionObstacleBinding {
            obstacle: Some(ObstacleHandle::new(4).unwrap()),
            plane: PlaneZCoeffs {
                az: 4.0,
                bz: 5.0,
                dz: 6.0,
            },
        };
        let arrays = LegacyPositionTopology {
            sectors: identity_sectors(3),
            sector_indices: identity_sector_indices(3),
            sector_doors: vec![None; 3],
            doors: vec![DoorHandle(9)],
            projection_areas: vec![projection],
            sight_obstacles: vec![sight],
        };

        let projectile = preflight_v48_position(&position_payload(), &entities, &arrays).unwrap();
        let mut actor_payload = position_payload();
        actor_payload.layer = 1;
        let actor = preflight_v48_position(&actor_payload, &entities, &arrays).unwrap();

        assert_eq!(projectile.obstacle, sight.obstacle);
        assert_eq!(actor.obstacle, projection.obstacle);
    }

    #[test]
    fn line_topology_preserves_per_layer_original_ordinals() {
        let topology = LegacyLineTopology::derive_from_layers([2, 5, 2, 2, 5]).unwrap();
        let line = topology
            .resolve(
                "jump_line",
                LegacyLineRef {
                    layer: Some(2),
                    index: Some(2),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            line.get(),
            3,
            "third line in layer 2 is runtime line 3, not flat index 2"
        );
        assert_eq!(
            topology
                .resolve(
                    "jump_line",
                    LegacyLineRef {
                        layer: None,
                        index: None,
                    },
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn line_topology_rejects_half_null_and_missing_identities() {
        let topology = LegacyLineTopology::derive_from_layers([2]).unwrap();
        assert!(matches!(
            topology.resolve(
                "jump_line",
                LegacyLineRef {
                    layer: Some(2),
                    index: None,
                },
            ),
            Err(LegacyLineTopologyError::InconsistentNull { .. })
        ));
        assert!(matches!(
            topology.resolve(
                "jump_line",
                LegacyLineRef {
                    layer: Some(2),
                    index: Some(1),
                },
            ),
            Err(LegacyLineTopologyError::Missing { .. })
        ));
    }

    fn jump_line(
        ax: f32,
        ay: f32,
        bx: f32,
        by: f32,
        associated: u32,
        sector: u32,
    ) -> crate::jump_line::JumpLine {
        let mut line =
            crate::jump_line::JumpLine::new(MapPoint::new(ax, ay), MapPoint::new(bx, by), 0.0, 0.0);
        line.layer = 0;
        line.associated_line_index = Some(associated);
        line.sector_index = SectorIndex::new(sector);
        line
    }

    fn ambiguous_jump_grid(second_target_y: f32) -> crate::fast_find_grid::FastFindGrid {
        let mut grid = crate::fast_find_grid::FastFindGrid::default();
        let level = std::sync::Arc::make_mut(&mut grid.level);
        level.sectors = vec![test_grid_sector(10), test_grid_sector(20)];
        level.jump_lines = vec![
            jump_line(0.0, 0.0, 10.0, 0.0, 1, 0),
            jump_line(0.0, 4.0, 10.0, 4.0, 0, 1),
            jump_line(20.0, 0.0, 30.0, 0.0, 3, 0),
            jump_line(0.0, second_target_y, 10.0, second_target_y, 2, 1),
        ];
        level.sectors[0].jump_line_indices = vec![
            JumpLineIndex::new(0).unwrap(),
            JumpLineIndex::new(2).unwrap(),
        ];
        level.sectors[1].jump_line_indices = vec![
            JumpLineIndex::new(1).unwrap(),
            JumpLineIndex::new(3).unwrap(),
        ];
        grid
    }

    #[test]
    fn shifted_enemy_line_uses_unique_primary_target_geometry() {
        let grid = ambiguous_jump_grid(12.0);
        let topology = LegacyLineTopology::default();
        let resolved = topology
            .resolve_enemy_jump_line(
                "enemy.jump_line",
                LegacyLineRef {
                    layer: Some(0),
                    index: Some(1399),
                },
                &grid,
                126,
                SectorHandle::new(10)
                    .unwrap()
                    .with_arena_index(SectorIndex::new(0).unwrap()),
                172,
                SectorHandle::new(20)
                    .unwrap()
                    .with_arena_index(SectorIndex::new(1).unwrap()),
                MapPoint::new(5.0, 4.0),
                50.0,
            )
            .unwrap();
        assert_eq!(resolved, JumpLineIndex::new(0));
    }

    #[test]
    fn shifted_enemy_line_rejects_equal_geometry_instead_of_using_ordinal() {
        let grid = ambiguous_jump_grid(4.0);
        let topology = LegacyLineTopology::default();
        let error = topology
            .resolve_enemy_jump_line(
                "enemy.jump_line",
                LegacyLineRef {
                    layer: Some(0),
                    index: Some(1399),
                },
                &grid,
                126,
                SectorHandle::new(10)
                    .unwrap()
                    .with_arena_index(SectorIndex::new(0).unwrap()),
                172,
                SectorHandle::new(20)
                    .unwrap()
                    .with_arena_index(SectorIndex::new(1).unwrap()),
                MapPoint::new(5.0, 4.0),
                50.0,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            LegacyLineTopologyError::AmbiguousGeometryIdentity {
                candidates,
                ..
            } if candidates == vec![0, 2]
        ));
    }
}
