//! One allocation-order plan for every `Location` native restored from a save.
//!
//! Original's native-object storage is shared by every script VM.  Handles
//! therefore depend on the order in which owners occur in the serialization
//! stream, not on the subsystem which happens to adopt a heap in Rust.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    element::Entity,
    engine::{EngineInner, LevelAssets},
    natives::{ComputedScriptLocation, ScriptHandleCodec},
    scb::TypeTag,
};

use super::{
    adopt::{LegacyEntityFixups, LegacySaveAdoptError},
    payload_base::LegacyElementRef,
    payload_dispatch::{LegacyElementPayload, LegacyElementPayloadStream},
    payload_vm::{LegacyVmMemberKind, LegacyVmMemberSection, LegacyVmMemberValue},
    post_grid::LegacyFastFindGridState,
    post_hiking::LegacyHikingGuideState,
};

const HANDLE_INDEX_MAX: usize = 0x0fff_ffff;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LegacyVmArenaOwner {
    Element(u32),
    ScriptZone(usize),
    Waypoint { path: usize, waypoint: usize },
    Global,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyVmArenaSlice {
    start: usize,
    len: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyVmArenaPlan {
    slices: BTreeMap<LegacyVmArenaOwner, LegacyVmArenaSlice>,
    locations: Vec<Option<ComputedScriptLocation>>,
}

#[derive(Debug, Error)]
pub enum LegacyVmArenaError {
    #[error(transparent)]
    Reference(#[from] LegacySaveAdoptError),
    #[error("serialized VM owner {owner:?} occurs more than once")]
    DuplicateOwner { owner: LegacyVmArenaOwner },
    #[error(
        "serialized VM owner {owner:?} member {member} declares {kind:?}, but its decoded value is {value_kind}"
    )]
    MemberValueMismatch {
        owner: LegacyVmArenaOwner,
        member: String,
        kind: LegacyVmMemberKind,
        value_kind: &'static str,
    },
    #[error(
        "serialized VM owner {owner:?} location member {member:?} references sector {sector}, but initialized topology has {count} sector slots"
    )]
    MissingSector {
        owner: LegacyVmArenaOwner,
        member: String,
        sector: u16,
        count: usize,
    },
    #[error(
        "serialized VM owner {owner:?} location member {member:?} references layer {layer}, but initialized topology has {count} layers"
    )]
    MissingLayer {
        owner: LegacyVmArenaOwner,
        member: String,
        layer: u16,
        count: usize,
    },
    #[error(
        "serialized VM owner {owner:?} location member {member:?} requires unrepresentable script handle index {index}"
    )]
    HandleOverflow {
        owner: LegacyVmArenaOwner,
        member: String,
        index: usize,
    },
    #[error("VM adoption requested unplanned serialized owner {owner:?}")]
    MissingOwner { owner: LegacyVmArenaOwner },
    #[error(
        "VM adoption owner {owner:?} has {actual} serialized Location members, but the shared arena reserved {expected}"
    )]
    OwnerLocationCountMismatch {
        owner: LegacyVmArenaOwner,
        expected: usize,
        actual: usize,
    },
    #[error(
        "serialized VM owner {owner:?} class is {saved:?}, but initialized class is {runtime:?}"
    )]
    ClassMismatch {
        owner: LegacyVmArenaOwner,
        saved: String,
        runtime: String,
    },
    #[error(
        "serialized VM owner {owner:?} member count is {saved}, but initialized class {class_name:?} has {runtime}"
    )]
    MemberCountMismatch {
        owner: LegacyVmArenaOwner,
        class_name: String,
        saved: usize,
        runtime: usize,
    },
    #[error("serialized VM owner {owner:?} member {index} schema mismatch: {detail}")]
    SchemaMismatch {
        owner: LegacyVmArenaOwner,
        index: usize,
        detail: String,
    },
    #[error(
        "serialized VM owner {owner:?} member {member:?} requires bytes {address}..{end}, outside initialized heap length {heap_len}"
    )]
    HeapRange {
        owner: LegacyVmArenaOwner,
        member: String,
        heap_len: usize,
        address: usize,
        end: usize,
    },
    #[error(
        "serialized VM owner {owner:?} {member_kind} member {member:?} resolves to wrong entity {entity_id}"
    )]
    WrongEntity {
        owner: LegacyVmArenaOwner,
        member_kind: &'static str,
        member: String,
        entity_id: crate::element::EntityId,
    },
}

impl LegacyVmArenaPlan {
    /// Walk owners in the exact order used by Linux-v48 serialization.
    pub(crate) fn preflight(
        engine: &EngineInner,
        assets: &LevelAssets,
        payloads: &LegacyElementPayloadStream,
        grid: &LegacyFastFindGridState,
        hiking: &LegacyHikingGuideState,
        global: Option<&LegacyVmMemberSection>,
    ) -> Result<Self, LegacyVmArenaError> {
        let mut builder = LegacyVmArenaBuilder {
            engine,
            assets,
            slices: BTreeMap::new(),
            locations: Vec::new(),
        };

        for record in &payloads.records {
            let members = match &record.payload {
                LegacyElementPayload::ActorPc(pc) => pc.human.actor.script_members.as_ref(),
                LegacyElementPayload::ActorNpcSoldier(soldier) => {
                    soldier.npc.human.actor.script_members.as_ref()
                }
                LegacyElementPayload::ActorNpcCivilian(civilian) => {
                    civilian.npc.human.actor.script_members.as_ref()
                }
                LegacyElementPayload::Scroll(scroll) => scroll.script_members.as_ref(),
                LegacyElementPayload::Target(target) => target.script_members.as_ref(),
                LegacyElementPayload::ObjectItem(_)
                | LegacyElementPayload::Bonus(_)
                | LegacyElementPayload::Fx(_)
                | LegacyElementPayload::FxMasked(_) => None,
            };
            if let Some(members) = members {
                builder.push(
                    LegacyVmArenaOwner::Element(record.header.creation_order),
                    members,
                )?;
            }
        }

        for (zone, state) in grid.script_sectors.iter().enumerate() {
            if let Some(members) = state.script_members.as_ref() {
                builder.push(LegacyVmArenaOwner::ScriptZone(zone), members)?;
            }
        }
        for (path, state) in hiking.paths.iter().enumerate() {
            for (waypoint, state) in state.waypoints.iter().enumerate() {
                if let Some(members) = state.script_members.as_ref() {
                    builder.push(LegacyVmArenaOwner::Waypoint { path, waypoint }, members)?;
                }
            }
        }
        if let Some(global) = global {
            builder.push(LegacyVmArenaOwner::Global, global)?;
        }

        Ok(Self {
            slices: builder.slices,
            locations: builder.locations,
        })
    }

    /// Return the absolute allocation prefix for one heap converter and also
    /// prove that it is looking at the section used to construct this plan.
    pub(crate) fn owner_prefix(
        &self,
        owner: LegacyVmArenaOwner,
        members: &LegacyVmMemberSection,
    ) -> Result<usize, LegacyVmArenaError> {
        let slice = self
            .slices
            .get(&owner)
            .ok_or(LegacyVmArenaError::MissingOwner { owner })?;
        let actual = location_member_count(owner, members)?;
        if actual != slice.len {
            return Err(LegacyVmArenaError::OwnerLocationCountMismatch {
                owner,
                expected: slice.len,
                actual,
            });
        }
        Ok(slice.start)
    }

    pub(crate) fn location_count(&self) -> usize {
        self.locations.len()
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests() -> Self {
        Self {
            slices: BTreeMap::new(),
            locations: Vec::new(),
        }
    }

    /// Convert a VM heap against its initialized class using this owner's
    /// absolute Location allocation prefix.
    pub(crate) fn preflight_heap(
        &self,
        engine: &EngineInner,
        assets: &LevelAssets,
        entities: &LegacyEntityFixups,
        owner: LegacyVmArenaOwner,
        saved: &LegacyVmMemberSection,
        class: &crate::scb::ClassEntry,
        current_heap: &[u8],
    ) -> Result<Vec<u8>, LegacyVmArenaError> {
        let location_prefix = self.owner_prefix(owner, saved)?;
        if saved.class_name != class.class_name {
            return Err(LegacyVmArenaError::ClassMismatch {
                owner,
                saved: saved.class_name.clone(),
                runtime: class.class_name.clone(),
            });
        }
        if saved.members.len() != class.member_variables.len() {
            return Err(LegacyVmArenaError::MemberCountMismatch {
                owner,
                class_name: class.class_name.clone(),
                saved: saved.members.len(),
                runtime: class.member_variables.len(),
            });
        }
        let mut heap = current_heap.to_vec();
        let mut location_ordinal = 0;
        for (index, (saved_member, runtime_member)) in saved
            .members
            .iter()
            .zip(&class.member_variables)
            .enumerate()
        {
            let expected_kind = if runtime_member.ty.tag == TypeTag::NativeType {
                match runtime_member.ty.native_type_name.as_str() {
                    "Actor" => LegacyVmMemberKind::ActorRef,
                    "Scroll" => LegacyVmMemberKind::ScrollRef,
                    "Location" => LegacyVmMemberKind::Location,
                    other => {
                        return Err(LegacyVmArenaError::SchemaMismatch {
                            owner,
                            index,
                            detail: format!(
                                "initialized class uses unsupported native type {other:?}"
                            ),
                        });
                    }
                }
            } else {
                LegacyVmMemberKind::Raw32 {
                    tag: runtime_member.ty.tag,
                }
            };
            if saved_member.schema.name != runtime_member.name
                || i32::try_from(saved_member.schema.address).ok() != Some(runtime_member.address)
                || saved_member.schema.kind != expected_kind
            {
                return Err(LegacyVmArenaError::SchemaMismatch {
                    owner,
                    index,
                    detail: format!(
                        "saved ({:?}, {}, {:?}) != runtime ({:?}, {}, {:?})",
                        saved_member.schema.name,
                        saved_member.schema.address,
                        saved_member.schema.kind,
                        runtime_member.name,
                        runtime_member.address,
                        expected_kind
                    ),
                });
            }
            let address = saved_member.schema.address as usize;
            let end = address.checked_add(4).unwrap_or(usize::MAX);
            if end > heap.len() {
                return Err(LegacyVmArenaError::HeapRange {
                    owner,
                    member: saved_member.schema.name.clone(),
                    heap_len: heap.len(),
                    address,
                    end,
                });
            }
            let bits = match (&saved_member.schema.kind, &saved_member.value) {
                (LegacyVmMemberKind::Raw32 { .. }, LegacyVmMemberValue::Raw32 { bits }) => *bits,
                (LegacyVmMemberKind::ActorRef, LegacyVmMemberValue::ActorRef(reference)) => {
                    resolve_entity_handle(
                        engine,
                        entities,
                        owner,
                        &saved_member.schema.name,
                        "Actor",
                        *reference,
                        Entity::is_actor,
                    )?
                }
                (LegacyVmMemberKind::ScrollRef, LegacyVmMemberValue::ScrollRef(reference)) => {
                    resolve_entity_handle(
                        engine,
                        entities,
                        owner,
                        &saved_member.schema.name,
                        "Scroll",
                        *reference,
                        |entity| matches!(entity, Entity::Scroll(_)),
                    )?
                }
                (LegacyVmMemberKind::Location, LegacyVmMemberValue::Location(location)) => {
                    let storage_index =
                        location_prefix
                            .checked_add(location_ordinal)
                            .ok_or_else(|| LegacyVmArenaError::HandleOverflow {
                                owner,
                                member: saved_member.schema.name.clone(),
                                index: usize::MAX,
                            })?;
                    location_ordinal += 1;
                    if location.is_none() {
                        0
                    } else {
                        let handle_index = assets
                            .scripts
                            .location_count
                            .checked_add(storage_index)
                            .ok_or_else(|| LegacyVmArenaError::HandleOverflow {
                                owner,
                                member: saved_member.schema.name.clone(),
                                index: usize::MAX,
                            })?;
                        if handle_index > HANDLE_INDEX_MAX {
                            return Err(LegacyVmArenaError::HandleOverflow {
                                owner,
                                member: saved_member.schema.name.clone(),
                                index: handle_index,
                            });
                        }
                        ScriptHandleCodec::location_handle_from_index(handle_index) as u32
                    }
                }
                _ => {
                    return Err(LegacyVmArenaError::MemberValueMismatch {
                        owner,
                        member: saved_member.schema.name.clone(),
                        kind: saved_member.schema.kind.clone(),
                        value_kind: value_kind(&saved_member.value),
                    });
                }
            };
            heap[address..end].copy_from_slice(&bits.to_le_bytes());
        }
        Ok(heap)
    }

    /// Install the arena once, after every heap plan has been preflighted.
    pub(crate) fn apply(self, engine: &mut EngineInner) {
        if self.locations.is_empty() && engine.scripts.mission.is_none() {
            return;
        }
        engine
            .scripts
            .mission
            .as_mut()
            .expect("preflighted shared VM arena mission disappeared")
            .state
            .computed_locations = self.locations;
    }
}

fn resolve_entity_handle(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    owner: LegacyVmArenaOwner,
    member: &str,
    member_kind: &'static str,
    reference: LegacyElementRef,
    predicate: impl FnOnce(&Entity) -> bool,
) -> Result<u32, LegacyVmArenaError> {
    let Some(entity_id) = entities.resolve_element(reference)? else {
        return Ok(0);
    };
    if !engine.world.entities.get(entity_id).is_some_and(predicate) {
        return Err(LegacyVmArenaError::WrongEntity {
            owner,
            member_kind,
            member: member.to_owned(),
            entity_id,
        });
    }
    Ok(ScriptHandleCodec::actor_handle(entity_id) as u32)
}

struct LegacyVmArenaBuilder<'a> {
    engine: &'a EngineInner,
    assets: &'a LevelAssets,
    slices: BTreeMap<LegacyVmArenaOwner, LegacyVmArenaSlice>,
    locations: Vec<Option<ComputedScriptLocation>>,
}

impl LegacyVmArenaBuilder<'_> {
    fn push(
        &mut self,
        owner: LegacyVmArenaOwner,
        members: &LegacyVmMemberSection,
    ) -> Result<(), LegacyVmArenaError> {
        let start = self.locations.len();
        if self.slices.contains_key(&owner) {
            return Err(LegacyVmArenaError::DuplicateOwner { owner });
        }
        for member in &members.members {
            validate_value_variant(
                owner,
                &member.schema.name,
                &member.schema.kind,
                &member.value,
            )?;
            let LegacyVmMemberValue::Location(location) = &member.value else {
                continue;
            };
            let storage_index = self.locations.len();
            let handle_index = self
                .assets
                .scripts
                .location_count
                .checked_add(storage_index)
                .ok_or_else(|| LegacyVmArenaError::HandleOverflow {
                    owner,
                    member: member.schema.name.clone(),
                    index: usize::MAX,
                })?;
            if handle_index > HANDLE_INDEX_MAX {
                return Err(LegacyVmArenaError::HandleOverflow {
                    owner,
                    member: member.schema.name.clone(),
                    index: handle_index,
                });
            }
            let converted = location
                .as_ref()
                .map(|location| {
                    let sector_count = self.assets.legacy_grid_topology.as_ref().map_or(
                        self.engine.world.fast_grid.level.sectors.len(),
                        |topology| topology.sectors.len(),
                    );
                    if let Some(sector) = location.sector.0
                        && usize::from(sector) >= sector_count
                    {
                        return Err(LegacyVmArenaError::MissingSector {
                            owner,
                            member: member.schema.name.clone(),
                            sector,
                            count: sector_count,
                        });
                    }
                    let layer_count = self.engine.world.fast_grid.level.layers.len();
                    if usize::from(location.layer) >= layer_count {
                        return Err(LegacyVmArenaError::MissingLayer {
                            owner,
                            member: member.schema.name.clone(),
                            layer: location.layer,
                            count: layer_count,
                        });
                    }
                    Ok(ComputedScriptLocation {
                        position: (location.position.x, location.position.y),
                        layer: Some(location.layer),
                        sector: location.sector.0,
                        active: location.active,
                        legacy_dummy: location.legacy_dummy,
                    })
                })
                .transpose()?;
            // SerializeLocation inserts a null slot too.
            self.locations.push(converted);
        }
        self.slices.insert(
            owner,
            LegacyVmArenaSlice {
                start,
                len: self.locations.len() - start,
            },
        );
        Ok(())
    }
}

fn location_member_count(
    owner: LegacyVmArenaOwner,
    members: &LegacyVmMemberSection,
) -> Result<usize, LegacyVmArenaError> {
    let mut count = 0;
    for member in &members.members {
        validate_value_variant(
            owner,
            &member.schema.name,
            &member.schema.kind,
            &member.value,
        )?;
        if matches!(member.schema.kind, LegacyVmMemberKind::Location) {
            count += 1;
        }
    }
    Ok(count)
}

fn validate_value_variant(
    owner: LegacyVmArenaOwner,
    member: &str,
    kind: &LegacyVmMemberKind,
    value: &LegacyVmMemberValue,
) -> Result<(), LegacyVmArenaError> {
    let matches = matches!(
        (kind, value),
        (
            LegacyVmMemberKind::Raw32 { .. },
            LegacyVmMemberValue::Raw32 { .. }
        ) | (
            LegacyVmMemberKind::ActorRef,
            LegacyVmMemberValue::ActorRef(_)
        ) | (
            LegacyVmMemberKind::ScrollRef,
            LegacyVmMemberValue::ScrollRef(_)
        ) | (
            LegacyVmMemberKind::Location,
            LegacyVmMemberValue::Location(_)
        )
    );
    if matches {
        return Ok(());
    }
    let value_kind = value_kind(value);
    Err(LegacyVmArenaError::MemberValueMismatch {
        owner,
        member: member.to_owned(),
        kind: kind.clone(),
        value_kind,
    })
}

fn value_kind(value: &LegacyVmMemberValue) -> &'static str {
    match value {
        LegacyVmMemberValue::Raw32 { .. } => "Raw32",
        LegacyVmMemberValue::ActorRef(_) => "ActorRef",
        LegacyVmMemberValue::ScrollRef(_) => "ScrollRef",
        LegacyVmMemberValue::Location(_) => "Location",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_save::payload_base::{LegacyPoint2, LegacySectorRef};
    use crate::legacy_save::payload_vm::{
        LegacyVmLocation, LegacyVmMemberSchema, LegacyVmMemberState,
    };

    fn one_location(value: Option<LegacyVmLocation>) -> LegacyVmMemberSection {
        LegacyVmMemberSection {
            class_name: "Fixture".to_owned(),
            members: vec![LegacyVmMemberState {
                schema: LegacyVmMemberSchema {
                    name: "where".to_owned(),
                    address: 0,
                    kind: LegacyVmMemberKind::Location,
                },
                value: LegacyVmMemberValue::Location(value),
            }],
        }
    }

    #[test]
    fn owner_lookup_preserves_interleaved_absolute_slices() {
        let owners = [
            LegacyVmArenaOwner::Element(10),
            LegacyVmArenaOwner::Element(11),
            LegacyVmArenaOwner::ScriptZone(0),
            LegacyVmArenaOwner::Waypoint {
                path: 0,
                waypoint: 2,
            },
            LegacyVmArenaOwner::Global,
        ];
        let plan = LegacyVmArenaPlan {
            slices: owners
                .into_iter()
                .enumerate()
                .map(|(start, owner)| (owner, LegacyVmArenaSlice { start, len: 1 }))
                .collect(),
            locations: vec![None; owners.len()],
        };
        let section = one_location(None);
        for (expected, owner) in owners.into_iter().enumerate() {
            assert_eq!(plan.owner_prefix(owner, &section).unwrap(), expected);
        }
    }

    #[test]
    fn owner_lookup_rejects_a_section_with_a_different_location_shape() {
        let owner = LegacyVmArenaOwner::Global;
        let plan = LegacyVmArenaPlan {
            slices: BTreeMap::from([(owner, LegacyVmArenaSlice { start: 4, len: 0 })]),
            locations: vec![None; 4],
        };
        assert!(matches!(
            plan.owner_prefix(owner, &one_location(None)),
            Err(LegacyVmArenaError::OwnerLocationCountMismatch {
                expected: 0,
                actual: 1,
                ..
            })
        ));
    }

    #[test]
    fn decoded_kind_and_value_must_match_before_slots_are_counted() {
        let mut section = one_location(Some(LegacyVmLocation {
            legacy_dummy: false,
            position: LegacyPoint2 { x: 1.0, y: 2.0 },
            layer: 0,
            active: true,
            sector: LegacySectorRef(None),
        }));
        section.members[0].schema.kind = LegacyVmMemberKind::ActorRef;
        assert!(matches!(
            location_member_count(LegacyVmArenaOwner::Element(7), &section),
            Err(LegacyVmArenaError::MemberValueMismatch { .. })
        ));
    }
}
