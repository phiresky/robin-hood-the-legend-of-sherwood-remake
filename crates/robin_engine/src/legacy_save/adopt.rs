//! Validated identity and reference planning for Original v48 save adoption.
//!
//! Original element pointers use creation-order IDs, while AI-local element
//! pointers use indices into the serialized `marrayElements`. Rust entity IDs
//! are intentionally unrelated to both spaces. Adoption therefore starts by
//! constructing one explicit isomorphic map before mutating the initialized
//! mission.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::{
    element::EntityId,
    engine::{EngineInner, LevelAssets},
};

use super::{
    body::LegacySaveBody,
    elements::{LegacyElementClass, LegacyElementEnvelope, LegacyElementResolution},
    payload_base::{LegacyAiElementRef, LegacyElementRef},
    topology_adapter::{
        LegacyStaticElementTopology, LegacyTopologyAdapterError, derive_static_element_topology,
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
}

/// Complete translation between the two Original reference spaces and stable
/// Rust entity IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyEntityFixups {
    pub by_creation_order: BTreeMap<u32, EntityId>,
    pub by_saved_slot: Vec<EntityId>,
    pub creation_order_by_entity: BTreeMap<EntityId, u32>,
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
        let mut by_saved_slot = Vec::new();
        by_saved_slot.reserve(envelope.records.len());
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
                                return Err(LegacySaveAdoptError::UnsupportedMobileMaster {
                                    slot: record.slot,
                                    creation_order: record.creation_order,
                                });
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
            by_saved_slot.push(entity_id);
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
                self.by_saved_slot.get(usize::from(slot)).copied().ok_or(
                    LegacySaveAdoptError::MissingAiElementSlot {
                        slot,
                        element_count: self.by_saved_slot.len(),
                    },
                )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::EntityIdKind;
    use crate::legacy_save::{
        elements::{LegacyElementFixupTable, LegacyElementRecord, LegacyElementResolution},
        payload_context::{LegacyElementPayloadMetadata, LegacyMissionPayloadMetadata},
    };

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
}
