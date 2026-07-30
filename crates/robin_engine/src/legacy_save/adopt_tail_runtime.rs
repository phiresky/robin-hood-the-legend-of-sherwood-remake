//! Atomic adoption of the first Original v48 post-titbits tail slice.
//!
//! Original load order (`original-code/RHengine.cpp:2795-2885`) restores the
//! engine-global VM members, the separate script-global integer array, then
//! timer and camera sequence-element pointers. This module preflights that
//! complete slice against the initialized mission and the converted sequence
//! plan before changing any candidate state.

use thiserror::Error;

use crate::{
    element::{Command, Entity},
    engine::{EngineInner, LevelAssets, TimerEntry},
    natives::{ComputedScriptLocation, ScriptHandleCodec},
    scb::TypeTag,
    sequence::{Field, FieldValue, SequenceElementRef, SequenceState},
};

use super::{
    adopt::{LegacyEntityFixups, LegacySaveAdoptError},
    adopt_sequences::{LegacySequenceAdoptError, LegacySequenceAdoptionPlan},
    adopt_vm_arena::{LegacyVmArenaError, LegacyVmArenaOwner, LegacyVmArenaPlan},
    payload_vm::{
        LegacyVmMemberKind, LegacyVmMemberSchema, LegacyVmMemberSection, LegacyVmMemberValue,
    },
    post_tail::{LegacyScriptGlobals, LegacyTimerSequenceState},
};

const HANDLE_INDEX_MAX: usize = 0x0fff_ffff;

#[derive(Debug, Error)]
pub enum LegacyTailRuntimeAdoptError {
    #[error(transparent)]
    VmArena(#[from] LegacyVmArenaError),
    #[error("saved global VM members exist, but the initialized mission has no global script VM")]
    MissingGlobalVm,
    #[error(
        "saved global VM class is {saved:?}, but the initialized global VM class is {runtime:?}"
    )]
    GlobalVmClassMismatch { saved: String, runtime: String },
    #[error(
        "saved global VM member count is {saved}, but initialized class {class_name:?} has {runtime}"
    )]
    GlobalVmMemberCountMismatch {
        class_name: String,
        saved: usize,
        runtime: usize,
    },
    #[error("saved global VM member {index} schema mismatch: {detail}")]
    GlobalVmSchemaMismatch { index: usize, detail: String },
    #[error(
        "initialized global VM heap has {heap_len} bytes, but member {member:?} requires byte range {address}..{end}"
    )]
    GlobalVmHeapRange {
        member: String,
        heap_len: usize,
        address: usize,
        end: usize,
    },
    #[error(transparent)]
    EntityReference(#[from] LegacySaveAdoptError),
    #[error("saved global VM {kind} member {member:?} resolves to wrong entity class {entity_id}")]
    WrongEntityClass {
        kind: &'static str,
        member: String,
        entity_id: crate::element::EntityId,
    },
    #[error(
        "saved global VM member {member:?} requires unrepresentable script handle index {index}"
    )]
    HandleIndexOverflow { member: String, index: usize },
    #[error(
        "saved global VM location member {member:?} names sector {sector}, outside Original sector topology count {count}"
    )]
    MissingLocationSector {
        member: String,
        sector: u16,
        count: usize,
    },
    #[error(
        "saved global VM location member {member:?} names layer {layer}, outside initialized layer count {count}"
    )]
    MissingLocationLayer {
        member: String,
        layer: u16,
        count: usize,
    },
    #[error(transparent)]
    SequenceReference(#[from] LegacySequenceAdoptError),
    #[error("saved timer list entry {index} contains a null sequence-element pointer")]
    NullTimerReference { index: usize },
    #[error(
        "saved timer list entry {index} resolves to command {command:?} in state {state:?}; expected an active Timer"
    )]
    InvalidTimerElement {
        index: usize,
        command: Command,
        state: SequenceState,
    },
    #[error("saved timer list entry {index} has no integer Timer property")]
    InvalidTimerProperty { index: usize },
    #[error("saved camera-present flag contains a null sequence-element pointer")]
    NullCameraReference,
    #[error(
        "saved camera sequence resolves to command {command:?} in state {state:?}; expected an in-progress CameraGoto or ZoomLevel"
    )]
    InvalidCameraElement {
        command: Command,
        state: SequenceState,
    },
}

/// Mutation-only state for the first tail slice. Apply this after applying
/// the `LegacySequenceAdoptionPlan` used during preflight.
#[derive(Debug)]
pub(crate) struct LegacyTailRuntimeAdoptionPlan {
    global_vm: Option<PlannedGlobalVm>,
    script_globals: Vec<i32>,
    timers: Vec<TimerEntry>,
    camera_element: Option<SequenceElementRef>,
}

#[derive(Debug)]
struct PlannedGlobalVm {
    heap: Vec<u8>,
}

impl LegacyTailRuntimeAdoptionPlan {
    pub(crate) fn preflight(
        engine: &EngineInner,
        assets: &LevelAssets,
        global_members: Option<&LegacyVmMemberSection>,
        script_globals: &LegacyScriptGlobals,
        timers: &LegacyTimerSequenceState,
        entities: &LegacyEntityFixups,
        sequences: &LegacySequenceAdoptionPlan,
        vm_arena: &LegacyVmArenaPlan,
    ) -> Result<Self, LegacyTailRuntimeAdoptError> {
        let global_vm = global_members
            .map(|members| {
                let location_prefix = vm_arena.owner_prefix(LegacyVmArenaOwner::Global, members)?;
                preflight_global_vm(engine, assets, members, entities, location_prefix)
            })
            .transpose()?;

        let mut planned_timers = Vec::with_capacity(timers.timer_elements.len());
        for (index, &saved_ref) in timers.timer_elements.iter().enumerate() {
            let (element_ref, element) = sequences
                .resolve_element("timer_elements", saved_ref)?
                .ok_or(LegacyTailRuntimeAdoptError::NullTimerReference { index })?;
            if !is_active_original_timer(element.command, element.state) {
                return Err(LegacyTailRuntimeAdoptError::InvalidTimerElement {
                    index,
                    command: element.command,
                    state: element.state,
                });
            }
            let remaining = match element.get_property(Field::Timer) {
                Some(FieldValue::Integer(value)) => *value,
                _ => {
                    return Err(LegacyTailRuntimeAdoptError::InvalidTimerProperty { index });
                }
            };
            planned_timers.push(TimerEntry {
                remaining,
                element_ref,
            });
        }

        let camera_element = match timers.camera_element {
            None => None,
            Some(saved_ref) => {
                let (element_ref, element) = sequences
                    .resolve_element("camera_element", saved_ref)?
                    .ok_or(LegacyTailRuntimeAdoptError::NullCameraReference)?;
                if !matches!(element.command, Command::CameraGoto | Command::ZoomLevel)
                    || element.state != SequenceState::InProgress
                {
                    return Err(LegacyTailRuntimeAdoptError::InvalidCameraElement {
                        command: element.command,
                        state: element.state,
                    });
                }
                Some(element_ref)
            }
        };

        Ok(Self {
            global_vm,
            script_globals: script_globals.values.clone(),
            timers: planned_timers,
            camera_element,
        })
    }

    pub(crate) fn apply(self, engine: &mut EngineInner) {
        if let Some(global) = self.global_vm {
            let mission = engine
                .scripts
                .mission
                .as_mut()
                .expect("preflighted global VM disappeared");
            mission.replace_global_vm_heap(global.heap);
        }
        engine.scripts.globals = self.script_globals;
        engine.orders.timer_elements = self.timers;
        engine.feedback.cutscene_camera.sequence_element = self.camera_element;
    }
}

fn is_active_original_timer(command: Command, state: SequenceState) -> bool {
    command == Command::Timer
        && matches!(state, SequenceState::Todo | SequenceState::InProgress)
}

fn preflight_global_vm(
    engine: &EngineInner,
    assets: &LevelAssets,
    saved: &LegacyVmMemberSection,
    entities: &LegacyEntityFixups,
    preserved_location_prefix: usize,
) -> Result<PlannedGlobalVm, LegacyTailRuntimeAdoptError> {
    let mission = engine
        .scripts
        .mission
        .as_ref()
        .ok_or(LegacyTailRuntimeAdoptError::MissingGlobalVm)?;
    let (class, current_heap) = mission.global_vm_class_and_heap();
    if saved.class_name != class.class_name {
        return Err(LegacyTailRuntimeAdoptError::GlobalVmClassMismatch {
            saved: saved.class_name.clone(),
            runtime: class.class_name.clone(),
        });
    }
    if saved.members.len() != class.member_variables.len() {
        return Err(LegacyTailRuntimeAdoptError::GlobalVmMemberCountMismatch {
            class_name: class.class_name.clone(),
            saved: saved.members.len(),
            runtime: class.member_variables.len(),
        });
    }

    let mut heap = current_heap.to_vec();
    let mut computed_locations = Vec::new();
    let sector_count = assets
        .legacy_grid_topology
        .as_ref()
        .map_or(engine.world.fast_grid.level.sectors.len(), |topology| {
            topology.sectors.len()
        });
    let layer_count = engine.world.fast_grid.level.layers.len();

    for (index, (saved_member, runtime_member)) in saved
        .members
        .iter()
        .zip(&class.member_variables)
        .enumerate()
    {
        validate_member_schema(index, &saved_member.schema, runtime_member)?;
        let address = usize::try_from(saved_member.schema.address)
            .expect("u32 member address is representable on supported hosts");
        let end = address.checked_add(4).ok_or_else(|| {
            LegacyTailRuntimeAdoptError::GlobalVmHeapRange {
                member: saved_member.schema.name.clone(),
                heap_len: heap.len(),
                address,
                end: usize::MAX,
            }
        })?;
        if end > heap.len() {
            return Err(LegacyTailRuntimeAdoptError::GlobalVmHeapRange {
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
                    saved_member.schema.name.as_str(),
                    *reference,
                    "Actor",
                    Entity::is_actor,
                )?
            }
            (LegacyVmMemberKind::ScrollRef, LegacyVmMemberValue::ScrollRef(reference)) => {
                resolve_entity_handle(
                    engine,
                    entities,
                    saved_member.schema.name.as_str(),
                    *reference,
                    "Scroll",
                    |entity| matches!(entity, Entity::Scroll(_)),
                )?
            }
            (LegacyVmMemberKind::Location, LegacyVmMemberValue::Location(location)) => {
                let storage_index = preserved_location_prefix
                    .checked_add(computed_locations.len())
                    .ok_or_else(|| LegacyTailRuntimeAdoptError::HandleIndexOverflow {
                        member: saved_member.schema.name.clone(),
                        index: usize::MAX,
                    })?;
                let bits = if let Some(location) = location {
                    if let Some(sector) = location.sector.0 {
                        if usize::from(sector) >= sector_count {
                            return Err(LegacyTailRuntimeAdoptError::MissingLocationSector {
                                member: saved_member.schema.name.clone(),
                                sector,
                                count: sector_count,
                            });
                        }
                    }
                    if usize::from(location.layer) >= layer_count {
                        return Err(LegacyTailRuntimeAdoptError::MissingLocationLayer {
                            member: saved_member.schema.name.clone(),
                            layer: location.layer,
                            count: layer_count,
                        });
                    }
                    let handle_index = assets
                        .scripts
                        .location_count
                        .checked_add(storage_index)
                        .ok_or_else(|| LegacyTailRuntimeAdoptError::HandleIndexOverflow {
                            member: saved_member.schema.name.clone(),
                            index: usize::MAX,
                        })?;
                    if handle_index > HANDLE_INDEX_MAX {
                        return Err(LegacyTailRuntimeAdoptError::HandleIndexOverflow {
                            member: saved_member.schema.name.clone(),
                            index: handle_index,
                        });
                    }
                    computed_locations.push(Some(ComputedScriptLocation {
                        position: (location.position.x, location.position.y),
                        layer: Some(location.layer),
                        sector: location.sector.0,
                        active: location.active,
                        legacy_dummy: location.legacy_dummy,
                    }));
                    ScriptHandleCodec::location_handle_from_index(handle_index) as u32
                } else {
                    // SerializeLocation inserts null into the Original's
                    // location-storage list, so preserve the allocation hole.
                    computed_locations.push(None);
                    0
                };
                bits
            }
            _ => {
                return Err(LegacyTailRuntimeAdoptError::GlobalVmSchemaMismatch {
                    index,
                    detail: "decoded value variant does not match decoded member kind".to_owned(),
                });
            }
        };
        heap[address..end].copy_from_slice(&bits.to_le_bytes());
    }

    Ok(PlannedGlobalVm { heap })
}

fn validate_member_schema(
    index: usize,
    saved: &LegacyVmMemberSchema,
    runtime: &crate::scb::MemberVariable,
) -> Result<(), LegacyTailRuntimeAdoptError> {
    let expected_kind = if runtime.ty.tag == TypeTag::NativeType {
        match runtime.ty.native_type_name.as_str() {
            "Actor" => LegacyVmMemberKind::ActorRef,
            "Scroll" => LegacyVmMemberKind::ScrollRef,
            "Location" => LegacyVmMemberKind::Location,
            other => {
                return Err(LegacyTailRuntimeAdoptError::GlobalVmSchemaMismatch {
                    index,
                    detail: format!("initialized class uses unsupported native type {other:?}"),
                });
            }
        }
    } else {
        LegacyVmMemberKind::Raw32 {
            tag: runtime.ty.tag,
        }
    };
    let runtime_address = u32::try_from(runtime.address).ok();
    if saved.name != runtime.name
        || Some(saved.address) != runtime_address
        || saved.kind != expected_kind
    {
        return Err(LegacyTailRuntimeAdoptError::GlobalVmSchemaMismatch {
            index,
            detail: format!(
                "saved ({:?}, {}, {:?}) != runtime ({:?}, {}, {:?})",
                saved.name, saved.address, saved.kind, runtime.name, runtime.address, expected_kind
            ),
        });
    }
    Ok(())
}

fn resolve_entity_handle(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    member: &str,
    reference: super::payload_base::LegacyElementRef,
    kind: &'static str,
    predicate: impl FnOnce(&Entity) -> bool,
) -> Result<u32, LegacyTailRuntimeAdoptError> {
    let Some(entity_id) = entities.resolve_element(reference)? else {
        return Ok(0);
    };
    let entity = engine.world.entities.get(entity_id).ok_or_else(|| {
        LegacyTailRuntimeAdoptError::WrongEntityClass {
            kind,
            member: member.to_owned(),
            entity_id,
        }
    })?;
    if !predicate(entity) {
        return Err(LegacyTailRuntimeAdoptError::WrongEntityClass {
            kind,
            member: member.to_owned(),
            entity_id,
        });
    }
    let index = entity_id.index() as usize;
    if index > HANDLE_INDEX_MAX {
        return Err(LegacyTailRuntimeAdoptError::HandleIndexOverflow {
            member: member.to_owned(),
            index,
        });
    }
    Ok(ScriptHandleCodec::actor_handle(entity_id) as u32)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        engine::MissionScript,
        scb::{ClassEntry, MemberVariable, ScType, ScbFile},
    };

    fn empty_fixups() -> LegacyEntityFixups {
        LegacyEntityFixups {
            by_creation_order: BTreeMap::new(),
            by_saved_slot: Vec::new(),
            creation_order_by_entity: BTreeMap::new(),
        }
    }

    fn global_vm_fixture() -> (EngineInner, LevelAssets, LegacyVmMemberSection) {
        let class = ClassEntry {
            source_file: "fixture.sc".to_owned(),
            class_name: "StartUp".to_owned(),
            size_of_member_variables: 8,
            member_variables: vec![
                MemberVariable {
                    ty: ScType {
                        tag: TypeTag::Int,
                        native_type_name: String::new(),
                    },
                    name: "score".to_owned(),
                    address: 0,
                },
                MemberVariable {
                    ty: ScType {
                        tag: TypeTag::NativeType,
                        native_type_name: "Location".to_owned(),
                    },
                    name: "target".to_owned(),
                    address: 4,
                },
            ],
            functions: Vec::new(),
            quads: Vec::new(),
        };
        let mission = MissionScript::from_scb(ScbFile {
            version: 1.0,
            classes: vec![class],
        })
        .unwrap();
        let mut engine = EngineInner::new();
        engine.scripts.install_mission(mission);
        let saved = LegacyVmMemberSection {
            class_name: "StartUp".to_owned(),
            members: vec![
                super::super::payload_vm::LegacyVmMemberState {
                    schema: LegacyVmMemberSchema {
                        name: "score".to_owned(),
                        address: 0,
                        kind: LegacyVmMemberKind::Raw32 { tag: TypeTag::Int },
                    },
                    value: LegacyVmMemberValue::Raw32 { bits: 0x89ab_cdef },
                },
                super::super::payload_vm::LegacyVmMemberState {
                    schema: LegacyVmMemberSchema {
                        name: "target".to_owned(),
                        address: 4,
                        kind: LegacyVmMemberKind::Location,
                    },
                    value: LegacyVmMemberValue::Location(None),
                },
            ],
        };
        (engine, LevelAssets::new(), saved)
    }

    #[test]
    fn global_vm_preflight_preserves_raw_bits_and_writes_a_null_location_handle() {
        let (engine, assets, saved) = global_vm_fixture();
        let planned = preflight_global_vm(&engine, &assets, &saved, &empty_fixups(), 0).unwrap();
        assert_eq!(&planned.heap[0..4], &0x89ab_cdef_u32.to_le_bytes());
        assert_eq!(&planned.heap[4..8], &0_u32.to_le_bytes());
    }

    #[test]
    fn global_vm_schema_mismatch_is_rejected_before_mutation() {
        let (engine, assets, mut saved) = global_vm_fixture();
        saved.members[0].schema.name = "wrong".to_owned();
        assert!(matches!(
            preflight_global_vm(&engine, &assets, &saved, &empty_fixups(), 0),
            Err(LegacyTailRuntimeAdoptError::GlobalVmSchemaMismatch { index: 0, .. })
        ));
        let (_, heap) = engine
            .scripts
            .mission
            .as_ref()
            .unwrap()
            .global_vm_class_and_heap();
        assert_eq!(heap, [0; 8]);
    }

    #[test]
    fn apply_replaces_script_globals_and_tail_owned_runtime_lists_together() {
        let mut engine = EngineInner::new();
        engine.scripts.globals = vec![1, 2, 3];
        let plan = LegacyTailRuntimeAdoptionPlan {
            global_vm: None,
            script_globals: vec![-7, 11],
            timers: Vec::new(),
            camera_element: None,
        };
        plan.apply(&mut engine);
        assert_eq!(engine.scripts.globals, [-7, 11]);
        assert!(engine.orders.timer_elements.is_empty());
        assert!(engine.feedback.cutscene_camera.sequence_element.is_none());
    }

    #[test]
    fn original_timer_list_accepts_todo_and_in_progress_sequence_storage() {
        // RHSequenceElement::Go dispatches ownerless Timer to
        // RHEngine::PerformExecuteCommand, whose Timer arm only adds it to
        // mlistTimerElements. It does not transition RHSEQ_TODO to
        // RHSEQ_INPROGRESS, so a live serialized timer is normally still Todo.
        assert!(is_active_original_timer(
            Command::Timer,
            SequenceState::Todo
        ));
        assert!(is_active_original_timer(
            Command::Timer,
            SequenceState::InProgress
        ));
        assert!(!is_active_original_timer(
            Command::Timer,
            SequenceState::Terminated
        ));
        assert!(!is_active_original_timer(
            Command::Wait,
            SequenceState::Todo
        ));
    }
}
