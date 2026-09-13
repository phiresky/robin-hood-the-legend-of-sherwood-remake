//! Atomic adoption of the first Original v48 post-titbits tail slice.
//!
//! The original game's load order restores the
//! engine-global VM members, the separate script-global integer array, then
//! timer and camera sequence-element pointers. This module preflights that
//! complete slice against the initialized mission and the converted sequence
//! plan before changing any candidate state.

use crate::{
    element::{Command, Entity},
    engine::{EngineInner, LevelAssets, TimerEntry},
    natives::{ComputedScriptLocation, ScriptHandleCodec},
    sequence::{Field, FieldValue, SequenceElementRef, SequenceState},
};

use super::{
    adopt::LegacyEntityFixups,
    adopt_common::{AdoptCtx, AdoptErrorKind, AdoptSite, LegacyAdoptError},
    adopt_sequences::LegacySequenceAdoptionPlan,
    adopt_vm_arena::{LegacyVmArenaOwner, LegacyVmArenaPlan},
    payload_vm::{
        LegacyVmMemberKind, LegacyVmMemberSchema, LegacyVmMemberSection, LegacyVmMemberValue,
    },
    post_tail::LegacyEnginePostTitbitsTail,
    vm_schema::{HANDLE_INDEX_MAX, check_location_topology},
};

const GLOBAL_VM: AdoptSite = AdoptSite::new("saved global");
const TIMERS: AdoptSite = AdoptSite::new("saved timer list");

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
        ctx: &AdoptCtx<'_>,
        tail: &LegacyEnginePostTitbitsTail,
        sequences: &LegacySequenceAdoptionPlan,
        vm_arena: &LegacyVmArenaPlan,
    ) -> Result<Self, LegacyAdoptError> {
        let AdoptCtx {
            engine,
            assets,
            entities,
            ..
        } = *ctx;
        let global_members = tail.global_script_members.as_ref();
        let script_globals = &tail.script_globals;
        let timers = &tail.timers;
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
                .ok_or_else(|| {
                    TIMERS.field_error(
                        format!("timer_elements[{index}]"),
                        AdoptErrorKind::NullReference,
                    )
                })?;
            if !is_active_original_timer(element.command, element.state) {
                return Err(TIMERS.field_error(
                    format!("timer_elements[{index}]"),
                    AdoptErrorKind::InvalidTimerElement {
                        command: element.command,
                        state: element.state,
                    },
                ));
            }
            let remaining = match element.get_property(Field::Timer) {
                // Signed `int` in the Original; the property word is stored
                // unsigned, so a countdown that already went negative in the
                // saved game must reinterpret rather than saturate.
                Some(FieldValue::Integer(value)) => *value as i32,
                _ => {
                    return Err(TIMERS.field_error(
                        format!("timer_elements[{index}]"),
                        AdoptErrorKind::Missing {
                            what: "integer Timer property",
                        },
                    ));
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
                    .ok_or_else(|| {
                        AdoptSite::new("saved camera sequence")
                            .field_error("camera_element", AdoptErrorKind::NullReference)
                    })?;
                if !matches!(element.command, Command::CameraGoto | Command::ZoomLevel)
                    || !matches!(
                        element.state,
                        SequenceState::InProgress | SequenceState::Todo | SequenceState::Postponed
                    )
                {
                    return Err(AdoptSite::new("saved camera sequence").field_error(
                        "camera_element",
                        AdoptErrorKind::InvalidCameraElement {
                            command: element.command,
                            state: element.state,
                        },
                    ));
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
    command == Command::Timer && matches!(state, SequenceState::Todo | SequenceState::InProgress)
}

fn preflight_global_vm(
    engine: &EngineInner,
    assets: &LevelAssets,
    saved: &LegacyVmMemberSection,
    entities: &LegacyEntityFixups,
    preserved_location_prefix: usize,
) -> Result<PlannedGlobalVm, LegacyAdoptError> {
    let mission = engine.scripts.mission.as_ref().ok_or_else(|| {
        AdoptSite::new("initialized mission").error(AdoptErrorKind::Missing {
            what: "global script VM",
        })
    })?;
    let (class, current_heap) = mission.global_vm_class_and_heap();
    if saved.class_name != class.class_name {
        return Err(GLOBAL_VM.error(AdoptErrorKind::VmClassMismatch {
            saved: saved.class_name.clone(),
            runtime: class.class_name.clone(),
        }));
    }
    if saved.members.len() != class.member_variables.len() {
        return Err(GLOBAL_VM.error(AdoptErrorKind::VmMemberCountMismatch {
            class_name: class.class_name.clone(),
            saved: saved.members.len(),
            runtime: class.member_variables.len(),
        }));
    }

    let mut heap = current_heap.to_vec();
    let mut computed_locations = Vec::new();
    let sector_count = assets
        .navigation
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
        let end = super::vm_schema::member_end(address, heap.len()).map_err(|end| {
            GLOBAL_VM.field_error(
                saved_member.schema.name.clone(),
                AdoptErrorKind::VmHeapRange {
                    heap_len: heap.len(),
                    address,
                    end,
                },
            )
        })?;
        let overflow = |index| {
            GLOBAL_VM.field_error(
                saved_member.schema.name.clone(),
                AdoptErrorKind::VmHandleOverflow { index },
            )
        };

        let bits = match (&saved_member.schema.kind, &saved_member.value) {
            (LegacyVmMemberKind::Raw32 { .. }, LegacyVmMemberValue::Raw32 { bits }) => *bits,
            (LegacyVmMemberKind::ActorRef, LegacyVmMemberValue::ActorRef(reference)) => {
                super::vm_schema::resolve_entity_handle(
                    engine,
                    entities,
                    *reference,
                    Entity::is_actor,
                )
                .map_err(|error| error.at(&GLOBAL_VM, &saved_member.schema.name, "Actor"))?
            }
            (LegacyVmMemberKind::ScrollRef, LegacyVmMemberValue::ScrollRef(reference)) => {
                super::vm_schema::resolve_entity_handle(engine, entities, *reference, |entity| {
                    matches!(entity, Entity::Scroll(_))
                })
                .map_err(|error| error.at(&GLOBAL_VM, &saved_member.schema.name, "Scroll"))?
            }
            (LegacyVmMemberKind::Location, LegacyVmMemberValue::Location(location)) => {
                let storage_index = preserved_location_prefix
                    .checked_add(computed_locations.len())
                    .ok_or_else(|| overflow(usize::MAX))?;

                if let Some(location) = location {
                    check_location_topology(
                        &GLOBAL_VM,
                        &saved_member.schema.name,
                        location.sector.0,
                        sector_count,
                        location.layer,
                        layer_count,
                    )?;
                    let handle_index = assets
                        .scripts
                        .location_count
                        .checked_add(storage_index)
                        .ok_or_else(|| overflow(usize::MAX))?;
                    if handle_index > HANDLE_INDEX_MAX {
                        return Err(overflow(handle_index));
                    }
                    computed_locations.push(Some(ComputedScriptLocation {
                        position: (location.position.x, location.position.y),
                        layer: Some(location.layer),
                        sector: location.sector.0,
                        sector_handle: location.sector.0.map(|slot| {
                            super::adopt::retained_position_sector_handle(assets, slot)
                        }),
                        active: location.active,
                    }));
                    ScriptHandleCodec::location_handle_from_index(handle_index) as u32
                } else {
                    // Location deserialization inserts null into the
                    // location-storage list, so preserve the allocation hole.
                    computed_locations.push(None);
                    0
                }
            }
            _ => {
                return Err(GLOBAL_VM.error(AdoptErrorKind::VmSchemaMismatch {
                    index,
                    detail: "decoded value variant does not match decoded member kind".to_owned(),
                }));
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
) -> Result<(), LegacyAdoptError> {
    super::vm_schema::check_member_schema(saved, runtime)
        .map_err(|detail| GLOBAL_VM.error(AdoptErrorKind::VmSchemaMismatch { index, detail }))
}

#[cfg(test)]
mod tests {
    use crate::scb::TypeTag;
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
            mobile_by_creation_order: BTreeMap::new(),
            mobile_owner_by_creation_order: BTreeMap::new(),
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
            Err(LegacyAdoptError {
                kind: AdoptErrorKind::VmSchemaMismatch { index: 0, .. },
                ..
            })
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
        let (mut engine, _, _) = global_vm_fixture();
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
    fn imported_global_padding_is_live_native_storage_without_reinitializing() {
        let (mut engine, _, _) = global_vm_fixture();
        let mut globals = vec![0; 16];
        globals[0] = 7;
        LegacyTailRuntimeAdoptionPlan {
            global_vm: None,
            script_globals: globals.clone(),
            timers: Vec::new(),
            camera_element: None,
        }
        .apply(&mut engine);
        let assets = LevelAssets::new();
        engine.scripts.attach_native_capabilities(&assets);
        let sim = crate::sim_rng::test_context();
        assert_eq!(
            engine.call_external_native(&sim, &assets, "GetGlobal", &[1]),
            Ok(0)
        );
        assert_eq!(
            engine.call_external_native(&sim, &assets, "SetGlobal", &[15, 9]),
            Ok(0)
        );
        globals[15] = 9;
        assert_eq!(engine.scripts.globals, globals);
        assert_eq!(
            engine.call_external_native(&sim, &assets, "GetGlobal", &[16]),
            Ok(-1)
        );
    }

    #[test]
    fn original_timer_list_accepts_todo_and_in_progress_sequence_storage() {
        // Original-game sequence dispatch sends an ownerless timer to
        // engine command execution, whose timer arm only adds it to
        // the timer-element list. It does not transition RHSEQ_TODO to
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
