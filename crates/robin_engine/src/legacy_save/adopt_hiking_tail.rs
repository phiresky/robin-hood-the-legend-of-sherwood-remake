//! Atomic adoption of HikingGuide VM state and the small engine-owned tail
//! references surrounding Original's transient trajectory preview.
//!
//! Original load order is significant:
//! `RHHikingGuide::Serialize` restores waypoint object members before the
//! engine-global VM, while `RHEngine::Serialize` later restores `mpDeadPC`,
//! the two-stage shield input state, and finally invalidates the deserialized
//! trajectory scratch object. The plan keeps simulation state and host output
//! separate and performs all fallible reference/schema checks before apply.

use thiserror::Error;

use crate::{
    ai::PathId,
    element::{Entity, EntityId},
    engine::{EngineInner, LevelAssets},
    natives::{ComputedScriptLocation, ScriptHandleCodec},
    scb::TypeTag,
};

use super::{
    adopt::{LegacyEntityFixups, LegacySaveAdoptError},
    payload_base::LegacyElementRef,
    payload_vm::{LegacyVmMemberKind, LegacyVmMemberSchema, LegacyVmMemberValue},
    post_hiking::{LegacyHikingGuideState, LegacyProjectileTrajectorySection},
    post_tail::{LegacyEnginePostTitbitsTail, LegacyPendingShieldState},
};

const HANDLE_INDEX_MAX: usize = 0x0fff_ffff;

#[derive(Debug, Error)]
pub enum LegacyHikingTailAdoptError {
    #[error("saved HikingGuide state exists, but the initialized mission has no script runtime")]
    MissingMissionScript,
    #[error("saved HikingGuide has {saved} paths, initialized mission has {runtime}")]
    PathCountMismatch { saved: usize, runtime: usize },
    #[error(
        "saved HikingGuide path {path} has {saved} waypoints, initialized mission has {runtime}"
    )]
    WaypointCountMismatch {
        path: usize,
        saved: usize,
        runtime: usize,
    },
    #[error("HikingGuide path index {path} is not representable as a runtime PathId")]
    InvalidPathId { path: usize },
    #[error("HikingGuide waypoint index {waypoint} on path {path} exceeds the u8 runtime identity")]
    InvalidWaypointId { path: usize, waypoint: usize },
    #[error(
        "saved waypoint VM exists at path {path}, waypoint {waypoint}, but no runtime VM exists"
    )]
    MissingWaypointVm { path: usize, waypoint: usize },
    #[error(
        "saved waypoint VM presence at path {path}, waypoint {waypoint} is {saved}, but initialized topology requires {runtime}"
    )]
    WaypointVmPresenceMismatch {
        path: usize,
        waypoint: usize,
        saved: bool,
        runtime: bool,
    },
    #[error(
        "saved waypoint VM class at path {path}, waypoint {waypoint} is {saved:?}, runtime is {runtime:?}"
    )]
    WaypointClassMismatch {
        path: usize,
        waypoint: usize,
        saved: String,
        runtime: String,
    },
    #[error(
        "saved waypoint VM member count at path {path}, waypoint {waypoint} is {saved}, runtime class {class_name:?} has {runtime}"
    )]
    WaypointMemberCountMismatch {
        path: usize,
        waypoint: usize,
        class_name: String,
        saved: usize,
        runtime: usize,
    },
    #[error(
        "saved waypoint VM member {member} at path {path}, waypoint {waypoint} is incompatible: {detail}"
    )]
    WaypointSchemaMismatch {
        path: usize,
        waypoint: usize,
        member: usize,
        detail: String,
    },
    #[error(
        "waypoint VM member {member:?} at path {path}, waypoint {waypoint} requires heap range {address}..{end}, but heap has {heap_len} bytes"
    )]
    WaypointHeapRange {
        path: usize,
        waypoint: usize,
        member: String,
        heap_len: usize,
        address: usize,
        end: usize,
    },
    #[error(transparent)]
    EntityReference(#[from] LegacySaveAdoptError),
    #[error("saved {field} resolves to wrong entity class {entity_id}; expected {expected}")]
    WrongEntityClass {
        field: String,
        entity_id: EntityId,
        expected: &'static str,
    },
    #[error("saved {field} requires unrepresentable script handle index {index}")]
    HandleIndexOverflow { field: String, index: usize },
    #[error("saved {field} names sector {sector}, but initialized topology has {count}")]
    MissingLocationSector {
        field: String,
        sector: u16,
        count: usize,
    },
    #[error("saved {field} names layer {layer}, but initialized topology has {count}")]
    MissingLocationLayer {
        field: String,
        layer: u16,
        count: usize,
    },
    #[error("saved pending shield danger point contains a non-finite coordinate")]
    InvalidShieldDangerPoint,
    #[error("saved shield state is waiting for a danger-point click but has no protected PC")]
    MissingPendingShieldProtectedPc,
    #[error("decoded value variant for saved VM member {field} does not match its schema")]
    MemberValueMismatch { field: String },
}

/// Host-only post-load consequence of the serialized trajectory scratch
/// object. Original restores the bytes for stream compatibility and then
/// unconditionally sets `mbValidTrajectory = false`; no simulation state may
/// consume the stale projectile, jumper, or jumped payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacyTrajectoryHostOutput {
    pub clear_preview: bool,
}

#[derive(Debug)]
pub(crate) struct LegacyHikingTailAdoptionPlan {
    waypoint_heaps: Vec<(PathId, u8, Vec<u8>)>,
    waypoint_locations: Vec<Option<ComputedScriptLocation>>,
    dead_pc: Option<EntityId>,
    shield_is_protected: bool,
    shield_protected_pc: Option<EntityId>,
    shield_danger_point: crate::coordinates::WorldPoint3D,
    host: LegacyTrajectoryHostOutput,
}

impl LegacyHikingTailAdoptionPlan {
    pub(crate) fn preflight(
        engine: &EngineInner,
        assets: &LevelAssets,
        hiking: &LegacyHikingGuideState,
        trajectory: &LegacyProjectileTrajectorySection,
        tail: &LegacyEnginePostTitbitsTail,
        shield_is_protected: bool,
        entities: &LegacyEntityFixups,
    ) -> Result<Self, LegacyHikingTailAdoptError> {
        let (waypoint_heaps, waypoint_locations) =
            preflight_waypoints(engine, assets, hiking, entities)?;
        let dead_pc = resolve_typed(
            engine,
            entities,
            "dead_pc",
            tail.dead_pc,
            "PC",
            Entity::is_pc,
        )?;
        let shield_protected_pc =
            preflight_shield(engine, entities, &tail.shield, shield_is_protected)?;

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
            waypoint_locations,
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
            mission.state.computed_locations = self.waypoint_locations;
        }
        engine.mission_domain.dead_pc = self.dead_pc;
        engine.world.shield.is_protected = self.shield_is_protected;
        engine.world.shield.protected_pc = self.shield_protected_pc;
        engine.world.shield.danger_point = self.shield_danger_point;
        // `muwSelectedLayer` is transient scratch and Original resets it to
        // zero in the same post-load block which invalidates trajectory state.
        engine.world.shield.danger_point_layer = 0;
        self.host
    }

    pub(crate) fn native_location_count(&self) -> usize {
        self.waypoint_locations.len()
    }
}

fn preflight_shield(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    shield: &LegacyPendingShieldState,
    is_protected: bool,
) -> Result<Option<EntityId>, LegacyHikingTailAdoptError> {
    if !shield.danger_point.x.is_finite()
        || !shield.danger_point.y.is_finite()
        || !shield.danger_point.z.is_finite()
    {
        return Err(LegacyHikingTailAdoptError::InvalidShieldDangerPoint);
    }
    let protected = resolve_typed(
        engine,
        entities,
        "shield.protected_pc",
        shield.protected_pc,
        "PC",
        Entity::is_pc,
    )?;
    if !is_protected && protected.is_none() {
        return Err(LegacyHikingTailAdoptError::MissingPendingShieldProtectedPc);
    }
    Ok(protected)
}

fn preflight_waypoints(
    engine: &EngineInner,
    assets: &LevelAssets,
    hiking: &LegacyHikingGuideState,
    entities: &LegacyEntityFixups,
) -> Result<
    (
        Vec<(PathId, u8, Vec<u8>)>,
        Vec<Option<ComputedScriptLocation>>,
    ),
    LegacyHikingTailAdoptError,
> {
    if hiking.paths.len() != assets.hiking_paths.len() {
        return Err(LegacyHikingTailAdoptError::PathCountMismatch {
            saved: hiking.paths.len(),
            runtime: assets.hiking_paths.len(),
        });
    }
    let mission = engine.scripts.mission.as_ref();
    let mut heaps = Vec::new();
    let mut locations = Vec::new();
    for (path_index, (saved_path, runtime_path)) in hiking
        .paths
        .iter()
        .zip(assets.hiking_paths.iter())
        .enumerate()
    {
        if saved_path.waypoints.len() != runtime_path.waypoints.len() {
            return Err(LegacyHikingTailAdoptError::WaypointCountMismatch {
                path: path_index,
                saved: saved_path.waypoints.len(),
                runtime: runtime_path.waypoints.len(),
            });
        }
        let path_raw = u16::try_from(path_index)
            .map_err(|_| LegacyHikingTailAdoptError::InvalidPathId { path: path_index })?;
        let path = PathId::new(path_raw)
            .ok_or(LegacyHikingTailAdoptError::InvalidPathId { path: path_index })?;
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
                return Err(LegacyHikingTailAdoptError::WaypointVmPresenceMismatch {
                    path: path_index,
                    waypoint: waypoint_index,
                    saved: saved_waypoint.script_members.is_some(),
                    runtime: runtime_has_vm,
                });
            }
            let Some(saved_members) = saved_waypoint.script_members.as_ref() else {
                continue;
            };
            let waypoint = u8::try_from(waypoint_index).map_err(|_| {
                LegacyHikingTailAdoptError::InvalidWaypointId {
                    path: path_index,
                    waypoint: waypoint_index,
                }
            })?;
            let mission = mission.ok_or(LegacyHikingTailAdoptError::MissingMissionScript)?;
            let (class, current_heap) = mission.waypoint_vm_class_and_heap(path, waypoint).ok_or(
                LegacyHikingTailAdoptError::MissingWaypointVm {
                    path: path_index,
                    waypoint: waypoint_index,
                },
            )?;
            if saved_members.class_name != class.class_name {
                return Err(LegacyHikingTailAdoptError::WaypointClassMismatch {
                    path: path_index,
                    waypoint: waypoint_index,
                    saved: saved_members.class_name.clone(),
                    runtime: class.class_name.clone(),
                });
            }
            if saved_members.members.len() != class.member_variables.len() {
                return Err(LegacyHikingTailAdoptError::WaypointMemberCountMismatch {
                    path: path_index,
                    waypoint: waypoint_index,
                    class_name: class.class_name.clone(),
                    saved: saved_members.members.len(),
                    runtime: class.member_variables.len(),
                });
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
                let end = address.checked_add(4).unwrap_or(usize::MAX);
                if end > heap.len() {
                    return Err(LegacyHikingTailAdoptError::WaypointHeapRange {
                        path: path_index,
                        waypoint: waypoint_index,
                        member: saved_member.schema.name.clone(),
                        heap_len: heap.len(),
                        address,
                        end,
                    });
                }
                let field = format!(
                    "hiking_guide.paths[{path_index}].waypoints[{waypoint_index}].{}",
                    saved_member.schema.name
                );
                let bits = convert_member(
                    engine,
                    assets,
                    entities,
                    &field,
                    &saved_member.schema.kind,
                    &saved_member.value,
                    &mut locations,
                )?;
                heap[address..end].copy_from_slice(&bits.to_le_bytes());
            }
            heaps.push((path, waypoint, heap));
        }
    }
    Ok((heaps, locations))
}

fn validate_schema(
    path: usize,
    waypoint: usize,
    member: usize,
    saved: &LegacyVmMemberSchema,
    runtime: &crate::scb::MemberVariable,
) -> Result<(), LegacyHikingTailAdoptError> {
    let expected = if runtime.ty.tag == TypeTag::NativeType {
        match runtime.ty.native_type_name.as_str() {
            "Actor" => LegacyVmMemberKind::ActorRef,
            "Scroll" => LegacyVmMemberKind::ScrollRef,
            "Location" => LegacyVmMemberKind::Location,
            other => {
                return Err(LegacyHikingTailAdoptError::WaypointSchemaMismatch {
                    path,
                    waypoint,
                    member,
                    detail: format!("runtime class uses unsupported native type {other:?}"),
                });
            }
        }
    } else {
        LegacyVmMemberKind::Raw32 {
            tag: runtime.ty.tag,
        }
    };
    if saved.name != runtime.name
        || i32::try_from(saved.address).ok() != Some(runtime.address)
        || saved.kind != expected
    {
        return Err(LegacyHikingTailAdoptError::WaypointSchemaMismatch {
            path,
            waypoint,
            member,
            detail: format!(
                "saved ({:?}, {}, {:?}) != runtime ({:?}, {}, {:?})",
                saved.name, saved.address, saved.kind, runtime.name, runtime.address, expected
            ),
        });
    }
    Ok(())
}

fn convert_member(
    engine: &EngineInner,
    assets: &LevelAssets,
    entities: &LegacyEntityFixups,
    field: &str,
    kind: &LegacyVmMemberKind,
    value: &LegacyVmMemberValue,
    locations: &mut Vec<Option<ComputedScriptLocation>>,
) -> Result<u32, LegacyHikingTailAdoptError> {
    match (kind, value) {
        (LegacyVmMemberKind::Raw32 { .. }, LegacyVmMemberValue::Raw32 { bits }) => Ok(*bits),
        (LegacyVmMemberKind::ActorRef, LegacyVmMemberValue::ActorRef(reference)) => resolve_handle(
            engine,
            entities,
            field,
            *reference,
            "Actor",
            Entity::is_actor,
        ),
        (LegacyVmMemberKind::ScrollRef, LegacyVmMemberValue::ScrollRef(reference)) => {
            resolve_handle(engine, entities, field, *reference, "Scroll", |entity| {
                matches!(entity, Entity::Scroll(_))
            })
        }
        (LegacyVmMemberKind::Location, LegacyVmMemberValue::Location(location)) => {
            let slot = locations.len();
            let bits = if let Some(location) = location {
                let sector_count = assets
                    .legacy_grid_topology
                    .as_ref()
                    .map_or(engine.world.fast_grid.level.sectors.len(), |topology| {
                        topology.sectors.len()
                    });
                if let Some(sector) = location.sector.0
                    && usize::from(sector) >= sector_count
                {
                    return Err(LegacyHikingTailAdoptError::MissingLocationSector {
                        field: field.to_owned(),
                        sector,
                        count: sector_count,
                    });
                }
                let layer_count = engine.world.fast_grid.level.layers.len();
                if usize::from(location.layer) >= layer_count {
                    return Err(LegacyHikingTailAdoptError::MissingLocationLayer {
                        field: field.to_owned(),
                        layer: location.layer,
                        count: layer_count,
                    });
                }
                let index = assets.scripts.location_count.checked_add(slot).ok_or(
                    LegacyHikingTailAdoptError::HandleIndexOverflow {
                        field: field.to_owned(),
                        index: usize::MAX,
                    },
                )?;
                if index > HANDLE_INDEX_MAX {
                    return Err(LegacyHikingTailAdoptError::HandleIndexOverflow {
                        field: field.to_owned(),
                        index,
                    });
                }
                locations.push(Some(ComputedScriptLocation {
                    position: (location.position.x, location.position.y),
                    layer: Some(location.layer),
                    sector: location.sector.0,
                    active: location.active,
                    legacy_dummy: location.legacy_dummy,
                }));
                ScriptHandleCodec::location_handle_from_index(index) as u32
            } else {
                locations.push(None);
                0
            };
            Ok(bits)
        }
        _ => Err(LegacyHikingTailAdoptError::MemberValueMismatch {
            field: field.to_owned(),
        }),
    }
}

fn resolve_handle(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    field: &str,
    reference: LegacyElementRef,
    expected: &'static str,
    predicate: impl FnOnce(&Entity) -> bool,
) -> Result<u32, LegacyHikingTailAdoptError> {
    let Some(entity) = resolve_typed(engine, entities, field, reference, expected, predicate)?
    else {
        return Ok(0);
    };
    let index = entity.index() as usize;
    if index > HANDLE_INDEX_MAX {
        return Err(LegacyHikingTailAdoptError::HandleIndexOverflow {
            field: field.to_owned(),
            index,
        });
    }
    Ok(ScriptHandleCodec::actor_handle(entity) as u32)
}

fn resolve_typed(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    field: &str,
    reference: LegacyElementRef,
    expected: &'static str,
    predicate: impl FnOnce(&Entity) -> bool,
) -> Result<Option<EntityId>, LegacyHikingTailAdoptError> {
    let Some(entity_id) = entities.resolve_element(reference)? else {
        return Ok(None);
    };
    let entity = engine.world.entities.get(entity_id).ok_or_else(|| {
        LegacyHikingTailAdoptError::WrongEntityClass {
            field: field.to_owned(),
            entity_id,
            expected,
        }
    })?;
    if !predicate(entity) {
        return Err(LegacyHikingTailAdoptError::WrongEntityClass {
            field: field.to_owned(),
            entity_id,
            expected,
        });
    }
    Ok(Some(entity_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scb::{MemberVariable, ScType};

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
            Err(LegacyHikingTailAdoptError::WaypointSchemaMismatch {
                path: 2,
                waypoint: 3,
                member: 0,
                ..
            })
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
}
