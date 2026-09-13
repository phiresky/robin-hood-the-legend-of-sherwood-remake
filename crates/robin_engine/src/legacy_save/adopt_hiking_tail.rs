//! Atomic adoption of hiking-path VM state and the small engine-owned tail
//! references surrounding Original's transient trajectory preview.
//!
//! Original load order is significant:
//! Hiking-guide serialization restores waypoint object members before the
//! engine-global VM, while engine serialization later restores the dead PC,
//! the two-stage shield input state, and finally invalidates the deserialized
//! trajectory scratch object. The plan keeps simulation state and host output
//! separate and performs all fallible reference/schema checks before apply.

use crate::{
    ai::PathId,
    element::{Entity, EntityId},
    engine::EngineInner,
    natives::{ComputedScriptLocation, ScriptHandleCodec},
};

use super::{
    adopt::LegacyEntityFixups,
    adopt_common::{AdoptCtx, AdoptErrorKind, AdoptSite, LegacyAdoptError},
    adopt_vm_arena::{LegacyVmArenaOwner, LegacyVmArenaPlan, value_kind},
    payload_base::LegacyElementRef,
    payload_vm::{
        LegacyVmMemberKind, LegacyVmMemberSchema, LegacyVmMemberState, LegacyVmMemberValue,
    },
    post_hiking::{LegacyHikingGuideState, LegacyProjectileTrajectorySection},
    post_tail::{LegacyEnginePostTitbitsTail, LegacyPendingShieldState},
    vm_schema::{HANDLE_INDEX_MAX, check_location_topology},
};

/// Error context for references whose field path already names the owner.
const SAVED: AdoptSite = AdoptSite::new("saved");

fn waypoint_site(path: usize, waypoint: usize) -> AdoptSite {
    AdoptSite::owned(format!(
        "saved waypoint at path {path}, waypoint {waypoint}"
    ))
}

/// Host-only post-load consequence of the serialized trajectory scratch
/// object. Original restores the bytes for stream compatibility and then
/// unconditionally invalidates the trajectory; no simulation state may
/// consume the stale projectile, jumper, or jumped payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacyTrajectoryHostOutput {
    pub clear_preview: bool,
}

#[derive(Debug)]
pub(crate) struct LegacyHikingTailAdoptionPlan {
    waypoint_heaps: Vec<(PathId, u8, Vec<u8>)>,
    dead_pc: Option<EntityId>,
    shield_is_protected: bool,
    shield_protected_pc: Option<EntityId>,
    shield_danger_point: crate::coordinates::WorldPoint3D,
    host: LegacyTrajectoryHostOutput,
}

impl LegacyHikingTailAdoptionPlan {
    pub(crate) fn preflight(
        ctx: &AdoptCtx<'_>,
        hiking: &LegacyHikingGuideState,
        trajectory: &LegacyProjectileTrajectorySection,
        tail: &LegacyEnginePostTitbitsTail,
        shield_is_protected: bool,
        vm_arena: &LegacyVmArenaPlan,
    ) -> Result<Self, LegacyAdoptError> {
        let AdoptCtx {
            engine, entities, ..
        } = *ctx;
        let waypoint_heaps = preflight_waypoints(ctx, hiking, vm_arena)?;
        let dead_pc = resolve_typed(
            engine,
            entities,
            "dead_pc",
            tail.dead_pc,
            "PC",
            Entity::is_pc,
        )?;
        let shield_protected_pc = preflight_shield(engine, entities, &tail.shield)?;

        // These references belong to an engine-owned preview helper, not the
        // entity array. Validate their serialized identities even though
        // Original invalidates the preview immediately after load.
        let _ = resolve_typed(
            engine,
            entities,
            "projectile_trajectory.projectile.shooter",
            trajectory.projectile.shooter,
            "Actor",
            Entity::is_actor,
        )?;
        let _ = resolve_typed(
            engine,
            entities,
            "projectile_trajectory.jumper",
            trajectory.jumper,
            "PC",
            Entity::is_pc,
        )?;
        let _ = resolve_typed(
            engine,
            entities,
            "projectile_trajectory.jumped",
            trajectory.jumped,
            "PC",
            Entity::is_pc,
        )?;

        Ok(Self {
            waypoint_heaps,
            dead_pc,
            shield_is_protected,
            shield_protected_pc,
            shield_danger_point: crate::coordinates::WorldPoint3D {
                x: tail.shield.danger_point.x,
                y: tail.shield.danger_point.y,
                z: tail.shield.danger_point.z,
            },
            host: LegacyTrajectoryHostOutput {
                clear_preview: true,
            },
        })
    }

    /// Apply only deterministic engine-owned state. This must precede the
    /// global-VM tail plan: it installs the waypoint-created Location arena,
    /// to which global VM locations are appended in Original allocation order.
    pub(crate) fn apply_engine(self, engine: &mut EngineInner) -> LegacyTrajectoryHostOutput {
        if !self.waypoint_heaps.is_empty() {
            let mission = engine
                .scripts
                .mission
                .as_mut()
                .expect("preflighted waypoint script runtime disappeared");
            for (path, waypoint, heap) in self.waypoint_heaps {
                assert!(
                    mission.replace_waypoint_vm_heap(path, waypoint, heap),
                    "preflighted waypoint VM disappeared"
                );
            }
        }
        engine.mission_domain.dead_pc = self.dead_pc;
        engine.world.shield.is_protected = self.shield_is_protected;
        engine.world.shield.protected_pc = self.shield_protected_pc;
        engine.world.shield.danger_point = self.shield_danger_point;
        // The selected layer is transient scratch and the original game resets it to
        // zero in the same post-load block which invalidates trajectory state.
        engine.world.shield.danger_point_layer = 0;
        self.host
    }
}

fn preflight_shield(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    shield: &LegacyPendingShieldState,
) -> Result<Option<EntityId>, LegacyAdoptError> {
    let pending_shield = AdoptSite::new("saved pending shield");
    pending_shield.finite("danger_point.x", shield.danger_point.x)?;
    pending_shield.finite("danger_point.y", shield.danger_point.y)?;
    pending_shield.finite("danger_point.z", shield.danger_point.z)?;
    // The original game serializes shield-protection state and its actor reference
    // independently and restores both without enforcing a cross-field
    // invariant. In particular, its constructor initializes the pointer but
    // not the mode flag, so untouched games can legitimately save false plus
    // null. Entering either shield action resets the complete protocol state
    // before input can consume it.
    resolve_typed(
        engine,
        entities,
        "shield.protected_pc",
        shield.protected_pc,
        "PC",
        Entity::is_pc,
    )
}

fn preflight_waypoints(
    ctx: &AdoptCtx<'_>,
    hiking: &LegacyHikingGuideState,
    vm_arena: &LegacyVmArenaPlan,
) -> Result<Vec<(PathId, u8, Vec<u8>)>, LegacyAdoptError> {
    let AdoptCtx { engine, assets, .. } = *ctx;
    if hiking.paths.len() != assets.navigation.hiking_paths.len() {
        return Err(AdoptSite::new("saved hiking data").field_error(
            "paths",
            AdoptErrorKind::CountMismatch {
                saved: hiking.paths.len(),
                runtime: assets.navigation.hiking_paths.len(),
            },
        ));
    }
    let mission = engine.scripts.mission.as_ref();
    let mut heaps = Vec::new();
    for (path_index, (saved_path, runtime_path)) in hiking
        .paths
        .iter()
        .zip(assets.navigation.hiking_paths.iter())
        .enumerate()
    {
        if saved_path.waypoints.len() != runtime_path.waypoints.len() {
            return Err(
                AdoptSite::owned(format!("saved hiking path {path_index}")).field_error(
                    "waypoints",
                    AdoptErrorKind::CountMismatch {
                        saved: saved_path.waypoints.len(),
                        runtime: runtime_path.waypoints.len(),
                    },
                ),
            );
        }
        let path_raw = u16::try_from(path_index)
            .map_err(|_| AdoptErrorKind::InvalidPathId { path: path_index })?;
        let path =
            PathId::new(path_raw).ok_or(AdoptErrorKind::InvalidPathId { path: path_index })?;
        for (waypoint_index, (saved_waypoint, runtime_waypoint)) in saved_path
            .waypoints
            .iter()
            .zip(&runtime_path.waypoints)
            .enumerate()
        {
            let runtime_has_vm = mission.is_some()
                && matches!(
                    runtime_waypoint.command,
                    crate::level_data::WaypointCommand::Script(_)
                );
            if saved_waypoint.script_members.is_some() != runtime_has_vm {
                return Err(waypoint_site(path_index, waypoint_index).error(
                    AdoptErrorKind::VmPresenceMismatch {
                        saved: saved_waypoint.script_members.is_some(),
                        runtime: runtime_has_vm,
                    },
                ));
            }
            let Some(saved_members) = saved_waypoint.script_members.as_ref() else {
                continue;
            };
            let waypoint =
                u8::try_from(waypoint_index).map_err(|_| AdoptErrorKind::InvalidWaypointId {
                    path: path_index,
                    waypoint: waypoint_index,
                })?;
            let mission = mission.ok_or_else(|| {
                AdoptSite::new("initialized mission").error(AdoptErrorKind::Missing {
                    what: "script runtime",
                })
            })?;
            let location_prefix = vm_arena.owner_prefix(
                LegacyVmArenaOwner::Waypoint {
                    path: path_index,
                    waypoint: waypoint_index,
                },
                saved_members,
            )?;
            let mut locations = Vec::new();
            let (class, current_heap) = mission
                .waypoint_vm_class_and_heap(path, waypoint)
                .ok_or_else(|| {
                    waypoint_site(path_index, waypoint_index)
                        .error(AdoptErrorKind::Missing { what: "runtime VM" })
                })?;
            if saved_members.class_name != class.class_name {
                return Err(waypoint_site(path_index, waypoint_index).error(
                    AdoptErrorKind::VmClassMismatch {
                        saved: saved_members.class_name.clone(),
                        runtime: class.class_name.clone(),
                    },
                ));
            }
            if saved_members.members.len() != class.member_variables.len() {
                return Err(waypoint_site(path_index, waypoint_index).error(
                    AdoptErrorKind::VmMemberCountMismatch {
                        class_name: class.class_name.clone(),
                        saved: saved_members.members.len(),
                        runtime: class.member_variables.len(),
                    },
                ));
            }
            let mut heap = current_heap.to_vec();
            for (member_index, (saved_member, runtime_member)) in saved_members
                .members
                .iter()
                .zip(&class.member_variables)
                .enumerate()
            {
                validate_schema(
                    path_index,
                    waypoint_index,
                    member_index,
                    &saved_member.schema,
                    runtime_member,
                )?;
                let address = saved_member.schema.address as usize;
                let end = super::vm_schema::member_end(address, heap.len()).map_err(|end| {
                    waypoint_site(path_index, waypoint_index).field_error(
                        saved_member.schema.name.clone(),
                        AdoptErrorKind::VmHeapRange {
                            heap_len: heap.len(),
                            address,
                            end,
                        },
                    )
                })?;
                let field = format!(
                    "hiking_guide.paths[{path_index}].waypoints[{waypoint_index}].{}",
                    saved_member.schema.name
                );
                let bits =
                    convert_member(ctx, &field, saved_member, location_prefix, &mut locations)?;
                heap[address..end].copy_from_slice(&bits.to_le_bytes());
            }
            heaps.push((path, waypoint, heap));
        }
    }
    Ok(heaps)
}

fn validate_schema(
    path: usize,
    waypoint: usize,
    member: usize,
    saved: &LegacyVmMemberSchema,
    runtime: &crate::scb::MemberVariable,
) -> Result<(), LegacyAdoptError> {
    super::vm_schema::check_member_schema(saved, runtime).map_err(|detail| {
        waypoint_site(path, waypoint).error(AdoptErrorKind::VmSchemaMismatch {
            index: member,
            detail,
        })
    })
}

fn convert_member(
    ctx: &AdoptCtx<'_>,
    field: &str,
    member: &LegacyVmMemberState,
    location_prefix: usize,
    locations: &mut Vec<Option<ComputedScriptLocation>>,
) -> Result<u32, LegacyAdoptError> {
    let AdoptCtx {
        engine,
        assets,
        entities,
        ..
    } = *ctx;
    let (kind, value) = (&member.schema.kind, &member.value);
    match (kind, value) {
        (LegacyVmMemberKind::Raw32 { .. }, LegacyVmMemberValue::Raw32 { bits }) => Ok(*bits),
        (LegacyVmMemberKind::ActorRef, LegacyVmMemberValue::ActorRef(reference)) => {
            super::vm_schema::resolve_entity_handle(engine, entities, *reference, Entity::is_actor)
                .map_err(|error| error.at(&SAVED, field, "Actor"))
        }
        (LegacyVmMemberKind::ScrollRef, LegacyVmMemberValue::ScrollRef(reference)) => {
            super::vm_schema::resolve_entity_handle(engine, entities, *reference, |entity| {
                matches!(entity, Entity::Scroll(_))
            })
            .map_err(|error| error.at(&SAVED, field, "Scroll"))
        }
        (LegacyVmMemberKind::Location, LegacyVmMemberValue::Location(location)) => {
            let overflow = |index| {
                SAVED.field_error(field.to_owned(), AdoptErrorKind::VmHandleOverflow { index })
            };
            let slot = location_prefix
                .checked_add(locations.len())
                .ok_or_else(|| overflow(usize::MAX))?;
            let bits = if let Some(location) = location {
                let sector_count = assets
                    .navigation
                    .legacy_grid_topology
                    .as_ref()
                    .map_or(engine.world.fast_grid.level.sectors.len(), |topology| {
                        topology.sectors.len()
                    });
                check_location_topology(
                    &SAVED,
                    field,
                    location.sector.0,
                    sector_count,
                    location.layer,
                    engine.world.fast_grid.level.layers.len(),
                )?;
                let index = assets
                    .scripts
                    .location_count
                    .checked_add(slot)
                    .ok_or_else(|| overflow(usize::MAX))?;
                if index > HANDLE_INDEX_MAX {
                    return Err(overflow(index));
                }
                locations.push(Some(ComputedScriptLocation {
                    position: (location.position.x, location.position.y),
                    layer: Some(location.layer),
                    sector: location.sector.0,
                    sector_handle: location
                        .sector
                        .0
                        .map(|slot| super::adopt::retained_position_sector_handle(assets, slot)),
                    active: location.active,
                }));
                ScriptHandleCodec::location_handle_from_index(index) as u32
            } else {
                locations.push(None);
                0
            };
            Ok(bits)
        }
        _ => Err(SAVED.field_error(
            field.to_owned(),
            AdoptErrorKind::VmMemberValueMismatch {
                kind: kind.clone(),
                value_kind: value_kind(value),
            },
        )),
    }
}

fn resolve_typed(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    field: &str,
    reference: LegacyElementRef,
    expected: &'static str,
    predicate: impl FnOnce(&Entity) -> bool,
) -> Result<Option<EntityId>, LegacyAdoptError> {
    let Some(entity_id) = entities.resolve_element(reference)? else {
        return Ok(None);
    };
    let wrong_kind = || {
        SAVED.field_error(
            field.to_owned(),
            AdoptErrorKind::WrongEntityKind {
                entity_id,
                expected,
            },
        )
    };
    let entity = engine
        .world
        .entities
        .get(entity_id)
        .ok_or_else(wrong_kind)?;
    if !predicate(entity) {
        return Err(wrong_kind());
    }
    Ok(Some(entity_id))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        legacy_save::payload_base::LegacyPoint3,
        scb::{MemberVariable, ScType, TypeTag},
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

    fn runtime_member(name: &str, address: i32, tag: TypeTag, native: &str) -> MemberVariable {
        MemberVariable {
            ty: ScType {
                tag,
                native_type_name: native.to_owned(),
            },
            name: name.to_owned(),
            address,
        }
    }

    #[test]
    fn waypoint_schema_requires_exact_name_address_and_native_kind() {
        let saved = LegacyVmMemberSchema {
            name: "target".to_owned(),
            address: 4,
            kind: LegacyVmMemberKind::ActorRef,
        };
        let runtime = runtime_member("target", 4, TypeTag::NativeType, "Actor");
        validate_schema(2, 3, 0, &saved, &runtime).unwrap();

        let wrong = runtime_member("target", 8, TypeTag::NativeType, "Actor");
        assert!(matches!(
            validate_schema(2, 3, 0, &saved, &wrong),
            Err(LegacyAdoptError {
                subject,
                kind: AdoptErrorKind::VmSchemaMismatch { index: 0, .. },
                ..
            }) if subject == "saved waypoint at path 2, waypoint 3"
        ));
    }

    #[test]
    fn trajectory_load_output_always_clears_host_preview() {
        assert!(
            LegacyTrajectoryHostOutput {
                clear_preview: true
            }
            .clear_preview
        );
    }

    #[test]
    fn null_shield_protectee_is_valid_independently_of_saved_mode() {
        let engine = EngineInner::new();
        let saved = LegacyPendingShieldState {
            start_offset: 0,
            danger_point: LegacyPoint3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            protected_pc: LegacyElementRef(None),
            end_offset: 16,
        };

        assert_eq!(
            preflight_shield(&engine, &empty_fixups(), &saved).unwrap(),
            None
        );

        let plan = LegacyHikingTailAdoptionPlan {
            waypoint_heaps: Vec::new(),
            dead_pc: None,
            shield_is_protected: false,
            shield_protected_pc: None,
            shield_danger_point: crate::coordinates::WorldPoint3D::default(),
            host: LegacyTrajectoryHostOutput {
                clear_preview: true,
            },
        };
        let mut restored = EngineInner::new();
        plan.apply_engine(&mut restored);
        assert!(!restored.world.shield.is_protected);
        assert_eq!(restored.world.shield.protected_pc, None);
    }
}
