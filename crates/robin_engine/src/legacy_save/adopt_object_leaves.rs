//! Atomic adoption of Original v48 object, scroll, target, and FX leaf state.
//!
//! [`super::adopt_elements`] owns the common `RHElement`/`RHSprite` base.
//! This module owns only the concrete non-actor members serialized above that
//! base.  Conversion is deliberately strict and mutation-free: every entity
//! kind, enum, reference, patch index, and script heap is validated before
//! [`LegacyObjectLeafAdoptionPlan::apply`] touches the candidate engine.

use thiserror::Error;

use crate::{
    element::{Entity, EntityId, LegacyV48ObjectRepulsivePointState, ObjectData, ObjectType},
    engine::{EngineInner, LevelAssets},
    natives::{ComputedScriptLocation, ScriptHandleCodec},
    order::OrderType,
    patch::PatchIndex,
    profiles::Action,
    scb::TypeTag,
};

use super::{
    adopt::{LegacyEntityFixups, LegacySaveAdoptError},
    adopt_vm_arena::{LegacyVmArenaError, LegacyVmArenaOwner, LegacyVmArenaPlan},
    payload_base::{LegacyElementRef, LegacyFxPayload},
    payload_dispatch::{LegacyElementPayload, LegacyElementPayloadStream},
    payload_nonactors::{LegacyObjectPayload, LegacyRepulsivePointPayload},
    payload_objects::LegacyObjectItemPayload,
    payload_vm::{LegacyVmMemberKind, LegacyVmMemberSection, LegacyVmMemberValue},
};

const HANDLE_INDEX_MAX: usize = 0x0fff_ffff;

/// Serialized object fields that Original overwrites before consulting them,
/// or which have no gameplay reader after construction.
///
/// `GetRepulsiveObjects` rebuilds the point position and force from the
/// object's current position/radius immediately before collision avoidance
/// uses it (`original-code/RHelementobject.cpp:514-525`). The register number
/// is serialized but has no post-construction reader in the Original object
/// branch. The repulsive-point storage is retained bit-exactly in a dormant
/// sidecar because its default constructor leaves the four force scalars
/// uninitialized; it must not be treated as live numeric geometry.
pub const OVERWRITTEN_OR_UNUSED_OBJECT_FIELDS: &[&str] = &[
    "RHElementObject::muwRegisterNumber",
    "RHElementObject::mrepulsivePoint",
];

#[derive(Debug, Error)]
pub enum LegacyObjectLeafAdoptError {
    #[error(transparent)]
    VmArena(#[from] LegacyVmArenaError),
    #[error(transparent)]
    Reference(#[from] LegacySaveAdoptError),
    #[error("saved leaf creation order {creation_order} resolves to missing entity {entity_id}")]
    MissingEntity {
        creation_order: u32,
        entity_id: EntityId,
    },
    #[error(
        "saved {saved_kind} creation order {creation_order} resolves to incompatible Rust entity {entity_id} ({runtime_kind})"
    )]
    WrongEntityKind {
        creation_order: u32,
        saved_kind: &'static str,
        entity_id: EntityId,
        runtime_kind: &'static str,
    },
    #[error(
        "saved object creation order {creation_order} field {field} has unknown enum value {value}"
    )]
    UnknownEnum {
        creation_order: u32,
        field: &'static str,
        value: u32,
    },
    #[error(
        "saved leaf creation order {creation_order} field {field} contains non-finite value {value}"
    )]
    NonFinite {
        creation_order: u32,
        field: &'static str,
        value: f32,
    },
    #[error(
        "saved FX creation order {creation_order} references negative patch index {patch_index}"
    )]
    NegativePatch {
        creation_order: u32,
        patch_index: i16,
    },
    #[error(
        "saved FX creation order {creation_order} references patch {patch_index}, but initialized mission has {patch_count} patches"
    )]
    MissingPatch {
        creation_order: u32,
        patch_index: usize,
        patch_count: usize,
    },
    #[error("saved target creation order {creation_order} linked_fxs[{index}] is null")]
    NullLinkedFx { creation_order: u32, index: usize },
    #[error(
        "saved target creation order {creation_order} linked_fxs[{index}] resolves to non-FX entity {entity_id}"
    )]
    WrongLinkedFx {
        creation_order: u32,
        index: usize,
        entity_id: EntityId,
    },
    #[error(
        "saved {owner_kind} creation order {creation_order} script VM presence is {saved}, initialized presence is {runtime}"
    )]
    VmPresenceMismatch {
        owner_kind: &'static str,
        creation_order: u32,
        saved: bool,
        runtime: bool,
    },
    #[error(
        "saved {owner_kind} creation order {creation_order} VM class is {saved:?}, initialized class is {runtime:?}"
    )]
    VmClassMismatch {
        owner_kind: &'static str,
        creation_order: u32,
        saved: String,
        runtime: String,
    },
    #[error(
        "saved {owner_kind} creation order {creation_order} VM member count is {saved}, initialized class {class_name:?} has {runtime}"
    )]
    VmMemberCountMismatch {
        owner_kind: &'static str,
        creation_order: u32,
        class_name: String,
        saved: usize,
        runtime: usize,
    },
    #[error(
        "saved {owner_kind} creation order {creation_order} VM member {index} schema mismatch: {detail}"
    )]
    VmSchemaMismatch {
        owner_kind: &'static str,
        creation_order: u32,
        index: usize,
        detail: String,
    },
    #[error(
        "saved {owner_kind} creation order {creation_order} VM member {member:?} requires bytes {address}..{end}, outside initialized heap length {heap_len}"
    )]
    VmHeapRange {
        owner_kind: &'static str,
        creation_order: u32,
        member: String,
        heap_len: usize,
        address: usize,
        end: usize,
    },
    #[error(
        "saved {owner_kind} creation order {creation_order} VM {member_kind} member {member:?} resolves to wrong entity {entity_id}"
    )]
    VmWrongEntity {
        owner_kind: &'static str,
        creation_order: u32,
        member_kind: &'static str,
        member: String,
        entity_id: EntityId,
    },
    #[error(
        "saved {owner_kind} creation order {creation_order} VM member {member:?} requires unrepresentable handle index {index}"
    )]
    VmHandleOverflow {
        owner_kind: &'static str,
        creation_order: u32,
        member: String,
        index: usize,
    },
    #[error(
        "saved {owner_kind} creation order {creation_order} VM location {member:?} references sector {sector}, but initialized topology has {count} sector slots"
    )]
    VmMissingSector {
        owner_kind: &'static str,
        creation_order: u32,
        member: String,
        sector: u16,
        count: usize,
    },
    #[error(
        "saved {owner_kind} creation order {creation_order} VM location {member:?} references layer {layer}, but initialized topology has {count} layers"
    )]
    VmMissingLayer {
        owner_kind: &'static str,
        creation_order: u32,
        member: String,
        layer: u16,
        count: usize,
    },
    #[error(
        "saved object-item creation order {creation_order} is a mobile master; mobile state belongs to the mobile adoption stage"
    )]
    MobileMaster { creation_order: u32 },
}

#[derive(Debug)]
pub struct LegacyObjectLeafAdoptionPlan {
    records: Vec<PlannedLeaf>,
}

#[derive(Debug)]
enum PlannedLeaf {
    Object {
        entity: EntityId,
        state: PlannedObject,
    },
    Scroll {
        entity: EntityId,
        state: PlannedObject,
        status: i32,
        hourglass_timeout: u32,
        vm_heap: Option<Vec<u8>>,
    },
    Target {
        entity: EntityId,
        animation: OrderType,
        progression: u32,
        linked_fxs: Vec<EntityId>,
        fx: PlannedFx,
        vm_heap: Option<Vec<u8>>,
    },
    Fx {
        entity: EntityId,
        state: PlannedFx,
    },
    FxMasked {
        entity: EntityId,
        animation_speed: f32,
    },
}

#[derive(Debug)]
struct PlannedObject {
    terminate: bool,
    quantity: u16,
    animation: OrderType,
    object_type: ObjectType,
    associated_action: Action,
    belongs_to_beggar: bool,
    taken: bool,
    legacy_v48_repulsive_point: LegacyV48ObjectRepulsivePointState,
}

#[derive(Debug)]
struct PlannedFx {
    patch_index: Option<PatchIndex>,
    force_display: bool,
    restore_background: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum LegacyVmOwnerKind {
    Actor,
    Target,
    Scroll,
}

impl LegacyVmOwnerKind {
    fn name(self) -> &'static str {
        match self {
            Self::Actor => "actor",
            Self::Target => "target",
            Self::Scroll => "scroll",
        }
    }
}

impl LegacyObjectLeafAdoptionPlan {
    /// Validate and convert every concrete non-actor leaf without mutation.
    ///
    pub fn preflight(
        engine: &EngineInner,
        assets: &LevelAssets,
        payloads: &LegacyElementPayloadStream,
        entities: &LegacyEntityFixups,
        vm_arena: &LegacyVmArenaPlan,
    ) -> Result<Self, LegacyObjectLeafAdoptError> {
        let mut records = Vec::new();
        for record in &payloads.records {
            let creation_order = record.header.creation_order;
            let entity_id = entities
                .by_creation_order
                .get(&creation_order)
                .copied()
                .ok_or(LegacySaveAdoptError::MissingCreationOrderReference { creation_order })?;
            let runtime = engine.world.entities.get(entity_id).ok_or(
                LegacyObjectLeafAdoptError::MissingEntity {
                    creation_order,
                    entity_id,
                },
            )?;
            let planned = match &record.payload {
                LegacyElementPayload::ActorPc(_)
                | LegacyElementPayload::ActorNpcSoldier(_)
                | LegacyElementPayload::ActorNpcCivilian(_) => continue,
                LegacyElementPayload::Bonus(saved) => {
                    require_kind(
                        runtime,
                        entity_id,
                        creation_order,
                        "bonus",
                        Entity::is_bonus,
                    )?;
                    PlannedLeaf::Object {
                        entity: entity_id,
                        state: preflight_object(&saved.object, creation_order, entities)?,
                    }
                }
                LegacyElementPayload::Scroll(saved) => {
                    require_kind(runtime, entity_id, creation_order, "scroll", |entity| {
                        matches!(entity, Entity::Scroll(_))
                    })?;
                    let location_prefix = saved
                        .script_members
                        .as_ref()
                        .map(|members| {
                            vm_arena
                                .owner_prefix(LegacyVmArenaOwner::Element(creation_order), members)
                        })
                        .transpose()?
                        .unwrap_or(0);
                    let mut computed_locations = Vec::new();
                    let vm_heap = preflight_vm(
                        engine,
                        assets,
                        entities,
                        entity_id,
                        creation_order,
                        LegacyVmOwnerKind::Scroll,
                        saved.script_members.as_ref(),
                        location_prefix,
                        &mut computed_locations,
                    )?;
                    let status = i32::try_from(saved.status)
                        .ok()
                        .filter(|v| (0..=3).contains(v))
                        .ok_or(LegacyObjectLeafAdoptError::UnknownEnum {
                            creation_order,
                            field: "RHElementScroll::mStatus",
                            value: saved.status,
                        })?;
                    PlannedLeaf::Scroll {
                        entity: entity_id,
                        state: preflight_object(&saved.object, creation_order, entities)?,
                        status,
                        hourglass_timeout: saved.script_hourglass_timeout,
                        vm_heap,
                    }
                }
                LegacyElementPayload::Target(saved) => {
                    require_kind(runtime, entity_id, creation_order, "target", |entity| {
                        matches!(entity, Entity::Target(_))
                    })?;
                    let location_prefix = saved
                        .script_members
                        .as_ref()
                        .map(|members| {
                            vm_arena
                                .owner_prefix(LegacyVmArenaOwner::Element(creation_order), members)
                        })
                        .transpose()?
                        .unwrap_or(0);
                    let mut computed_locations = Vec::new();
                    let linked_fxs = saved
                        .linked_fxs
                        .iter()
                        .enumerate()
                        .map(|(index, reference)| {
                            let entity = entities.resolve_element(*reference)?.ok_or(
                                LegacyObjectLeafAdoptError::NullLinkedFx {
                                    creation_order,
                                    index,
                                },
                            )?;
                            if !matches!(engine.world.entities.get(entity), Some(Entity::Fx(_))) {
                                return Err(LegacyObjectLeafAdoptError::WrongLinkedFx {
                                    creation_order,
                                    index,
                                    entity_id: entity,
                                });
                            }
                            Ok(entity)
                        })
                        .collect::<Result<Vec<_>, LegacyObjectLeafAdoptError>>()?;
                    let vm_heap = preflight_vm(
                        engine,
                        assets,
                        entities,
                        entity_id,
                        creation_order,
                        LegacyVmOwnerKind::Target,
                        saved.script_members.as_ref(),
                        location_prefix,
                        &mut computed_locations,
                    )?;
                    PlannedLeaf::Target {
                        entity: entity_id,
                        animation: order_type(
                            saved.animation,
                            creation_order,
                            "RHElementTarget::manimation",
                        )?,
                        progression: saved.progression,
                        linked_fxs,
                        fx: preflight_fx(engine, &saved.fx, creation_order)?,
                        vm_heap,
                    }
                }
                LegacyElementPayload::Fx(saved) => {
                    require_kind(
                        runtime,
                        entity_id,
                        creation_order,
                        "FX",
                        |entity| matches!(entity, Entity::Fx(fx) if fx.fx.mobile_index.is_none()),
                    )?;
                    PlannedLeaf::Fx {
                        entity: entity_id,
                        state: preflight_fx(engine, &saved.fx, creation_order)?,
                    }
                }
                LegacyElementPayload::FxMasked(saved) => {
                    require_kind(
                        runtime,
                        entity_id,
                        creation_order,
                        "masked FX",
                        |entity| matches!(entity, Entity::Fx(fx) if fx.fx.mobile_index.is_some()),
                    )?;
                    finite(
                        saved.animation_speed,
                        creation_order,
                        "RHElementFXMasked::mfAnimationSpeed",
                    )?;
                    PlannedLeaf::FxMasked {
                        entity: entity_id,
                        animation_speed: saved.animation_speed,
                    }
                }
                LegacyElementPayload::ObjectItem(saved) => {
                    preflight_object_item(saved, runtime, entity_id, creation_order, entities)?
                }
            };
            records.push(planned);
        }
        Ok(Self { records })
    }

    /// Apply a completely preflighted plan to the detached candidate.
    pub fn apply(self, engine: &mut EngineInner) {
        for record in self.records {
            match record {
                PlannedLeaf::Object { entity, state } => {
                    apply_object(
                        engine
                            .world
                            .entities
                            .get_mut(entity)
                            .and_then(Entity::object_data_mut)
                            .expect("preflighted object leaf changed kind"),
                        state,
                    );
                }
                PlannedLeaf::Scroll {
                    entity,
                    state,
                    status,
                    hourglass_timeout,
                    vm_heap,
                } => {
                    let scroll = engine
                        .world
                        .entities
                        .get_mut(entity)
                        .and_then(Entity::as_scroll_mut)
                        .expect("preflighted scroll leaf changed kind");
                    apply_object(&mut scroll.object, state);
                    scroll.script_hourglass_timeout = hourglass_timeout;
                    engine.restore_legacy_scroll_status_raw(entity, status);
                    if let Some(heap) = vm_heap {
                        engine
                            .scripts
                            .mission
                            .as_mut()
                            .expect("preflighted scroll VM mission disappeared")
                            .replace_scroll_vm_heap(ScriptHandleCodec::actor_handle(entity), heap);
                    }
                }
                PlannedLeaf::Target {
                    entity,
                    animation,
                    progression,
                    linked_fxs,
                    fx,
                    vm_heap,
                } => {
                    let target = engine
                        .world
                        .entities
                        .get_mut(entity)
                        .and_then(Entity::as_target_mut)
                        .expect("preflighted target leaf changed kind");
                    target.target.animation = animation;
                    target.target.progression = progression;
                    target.target.linked_fx = linked_fxs;
                    apply_fx(&mut target.fx, fx);
                    if let Some(heap) = vm_heap {
                        engine
                            .scripts
                            .mission
                            .as_mut()
                            .expect("preflighted target VM mission disappeared")
                            .replace_target_vm_heap(ScriptHandleCodec::actor_handle(entity), heap);
                    }
                }
                PlannedLeaf::Fx { entity, state } => {
                    let fx = engine
                        .world
                        .entities
                        .get_mut(entity)
                        .and_then(Entity::as_fx_mut)
                        .expect("preflighted FX leaf changed kind");
                    apply_fx(&mut fx.fx, state);
                }
                PlannedLeaf::FxMasked {
                    entity,
                    animation_speed,
                } => {
                    engine
                        .world
                        .entities
                        .get_mut(entity)
                        .and_then(Entity::as_fx_mut)
                        .expect("preflighted masked-FX leaf changed kind")
                        .fx
                        .animation_speed = animation_speed;
                }
            }
        }
    }
}

fn preflight_object_item(
    saved: &LegacyObjectItemPayload,
    runtime: &Entity,
    entity_id: EntityId,
    creation_order: u32,
    entities: &LegacyEntityFixups,
) -> Result<PlannedLeaf, LegacyObjectLeafAdoptError> {
    let (object, saved_kind, predicate): (&LegacyObjectPayload, &'static str, fn(&Entity) -> bool) =
        match saved {
            // Rust stores plain RHElementObject-derived leaves in the
            // `Entity::Bonus` representation too. Their ElementKind remains
            // ObjectOther, so the semantic `Entity::is_bonus` predicate
            // intentionally returns false for them.
            LegacyObjectItemPayload::Object(object) => (object, "object", |entity| {
                matches!(entity, Entity::Bonus(_))
            }),
            LegacyObjectItemPayload::Ale(payload) => (&payload.object, "ale", |entity| {
                matches!(entity, Entity::Bonus(_))
            }),
            LegacyObjectItemPayload::SpyCape(payload) => (&payload.object, "spy cape", |entity| {
                matches!(entity, Entity::Bonus(_))
            }),
            LegacyObjectItemPayload::Arrow(payload) => {
                (&payload.projectile.object, "arrow", Entity::is_projectile)
            }
            LegacyObjectItemPayload::Apple(payload) => {
                (&payload.projectile.object, "apple", Entity::is_projectile)
            }
            LegacyObjectItemPayload::Purse(payload) => {
                (&payload.projectile.object, "purse", Entity::is_projectile)
            }
            LegacyObjectItemPayload::Stone(payload) => {
                (&payload.projectile.object, "stone", Entity::is_projectile)
            }
            LegacyObjectItemPayload::WaspNest(payload) => (
                &payload.projectile.object,
                "wasp nest",
                Entity::is_projectile,
            ),
            LegacyObjectItemPayload::Wasp(payload) => {
                (&payload.object, "wasp", Entity::is_projectile)
            }
            LegacyObjectItemPayload::Coin(payload) => {
                (&payload.projectile.object, "coin", Entity::is_projectile)
            }
            LegacyObjectItemPayload::Net(payload) => {
                (&payload.projectile.object, "net", |entity| {
                    matches!(entity, Entity::Net(_))
                })
            }
            LegacyObjectItemPayload::Mobile(_) => {
                return Err(LegacyObjectLeafAdoptError::MobileMaster { creation_order });
            }
        };
    require_kind(runtime, entity_id, creation_order, saved_kind, predicate)?;
    Ok(PlannedLeaf::Object {
        entity: entity_id,
        state: preflight_object(object, creation_order, entities)?,
    })
}

fn preflight_object(
    saved: &LegacyObjectPayload,
    creation_order: u32,
    entities: &LegacyEntityFixups,
) -> Result<PlannedObject, LegacyObjectLeafAdoptError> {
    // Object reference/back-pointer fields are not serialized by
    // RHElementObject. Projectile-family leaf plans own those references.
    let _ = entities;
    Ok(PlannedObject {
        terminate: saved.terminate,
        quantity: saved.quantity,
        animation: order_type(
            saved.animation,
            creation_order,
            "RHElementObject::manimation",
        )?,
        object_type: object_type(saved.object_type, creation_order)?,
        associated_action: Action::try_from(saved.associated_action).map_err(|_| {
            LegacyObjectLeafAdoptError::UnknownEnum {
                creation_order,
                field: "RHElementObject::mAssociatedAction",
                value: saved.associated_action,
            }
        })?,
        belongs_to_beggar: saved.belongs_to_beggar,
        taken: saved.taken,
        legacy_v48_repulsive_point: retain_dormant_object_repulsive_point(&saved.repulsive_point),
    })
}

fn apply_object(runtime: &mut ObjectData, saved: PlannedObject) {
    runtime.terminate = saved.terminate;
    runtime.quantity = saved.quantity;
    runtime.animation = saved.animation;
    runtime.object_type = saved.object_type;
    runtime.associated_action = saved.associated_action;
    runtime.belongs_to_beggar = saved.belongs_to_beggar;
    runtime.taken = saved.taken;
    runtime.legacy_v48_repulsive_point = Some(saved.legacy_v48_repulsive_point);
}

fn preflight_fx(
    engine: &EngineInner,
    saved: &LegacyFxPayload,
    creation_order: u32,
) -> Result<PlannedFx, LegacyObjectLeafAdoptError> {
    let patch_index = saved
        .patch
        .0
        .map(|raw| {
            let raw =
                u16::try_from(raw).map_err(|_| LegacyObjectLeafAdoptError::NegativePatch {
                    creation_order,
                    patch_index: raw,
                })?;
            let index = usize::from(raw);
            let patch_count = engine.script_domains.interactables.patches.len();
            if index >= patch_count {
                return Err(LegacyObjectLeafAdoptError::MissingPatch {
                    creation_order,
                    patch_index: index,
                    patch_count,
                });
            }
            PatchIndex::new(u32::from(raw)).ok_or(LegacyObjectLeafAdoptError::MissingPatch {
                creation_order,
                patch_index: index,
                patch_count,
            })
        })
        .transpose()?;
    Ok(PlannedFx {
        patch_index,
        force_display: saved.force_display,
        restore_background: saved.restore_background,
    })
}

fn apply_fx(runtime: &mut crate::element::FxData, saved: PlannedFx) {
    runtime.patch_index = saved.patch_index;
    runtime.force_display = saved.force_display;
    runtime.restore_background = saved.restore_background;
}

pub(crate) fn preflight_vm(
    engine: &EngineInner,
    assets: &LevelAssets,
    entities: &LegacyEntityFixups,
    owner: EntityId,
    creation_order: u32,
    owner_kind: LegacyVmOwnerKind,
    saved: Option<&LegacyVmMemberSection>,
    location_prefix: usize,
    computed_locations: &mut Vec<Option<ComputedScriptLocation>>,
) -> Result<Option<Vec<u8>>, LegacyObjectLeafAdoptError> {
    let handle = ScriptHandleCodec::actor_handle(owner);
    let runtime = engine
        .scripts
        .mission
        .as_ref()
        .and_then(|mission| match owner_kind {
            LegacyVmOwnerKind::Actor => mission.actor_vm_class_and_heap(handle),
            LegacyVmOwnerKind::Target => mission.target_vm_class_and_heap(handle),
            LegacyVmOwnerKind::Scroll => mission.scroll_vm_class_and_heap(handle),
        });
    if saved.is_some() != runtime.is_some() {
        return Err(LegacyObjectLeafAdoptError::VmPresenceMismatch {
            owner_kind: owner_kind.name(),
            creation_order,
            saved: saved.is_some(),
            runtime: runtime.is_some(),
        });
    }
    let (Some(saved), Some((class, current_heap))) = (saved, runtime) else {
        return Ok(None);
    };
    if saved.class_name != class.class_name {
        return Err(LegacyObjectLeafAdoptError::VmClassMismatch {
            owner_kind: owner_kind.name(),
            creation_order,
            saved: saved.class_name.clone(),
            runtime: class.class_name.clone(),
        });
    }
    if saved.members.len() != class.member_variables.len() {
        return Err(LegacyObjectLeafAdoptError::VmMemberCountMismatch {
            owner_kind: owner_kind.name(),
            creation_order,
            class_name: class.class_name.clone(),
            saved: saved.members.len(),
            runtime: class.member_variables.len(),
        });
    }
    let mut heap = current_heap.to_vec();
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
                    return Err(LegacyObjectLeafAdoptError::VmSchemaMismatch {
                        owner_kind: owner_kind.name(),
                        creation_order,
                        index,
                        detail: format!("initialized class uses unsupported native type {other:?}"),
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
            return Err(LegacyObjectLeafAdoptError::VmSchemaMismatch {
                owner_kind: owner_kind.name(),
                creation_order,
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
            return Err(LegacyObjectLeafAdoptError::VmHeapRange {
                owner_kind: owner_kind.name(),
                creation_order,
                member: saved_member.schema.name.clone(),
                heap_len: heap.len(),
                address,
                end,
            });
        }
        let bits = match (&saved_member.schema.kind, &saved_member.value) {
            (LegacyVmMemberKind::Raw32 { .. }, LegacyVmMemberValue::Raw32 { bits }) => *bits,
            (LegacyVmMemberKind::ActorRef, LegacyVmMemberValue::ActorRef(reference)) => {
                vm_entity_handle(
                    engine,
                    entities,
                    owner_kind,
                    creation_order,
                    &saved_member.schema.name,
                    "Actor",
                    *reference,
                    Entity::is_actor,
                )?
            }
            (LegacyVmMemberKind::ScrollRef, LegacyVmMemberValue::ScrollRef(reference)) => {
                vm_entity_handle(
                    engine,
                    entities,
                    owner_kind,
                    creation_order,
                    &saved_member.schema.name,
                    "Scroll",
                    *reference,
                    |entity| matches!(entity, Entity::Scroll(_)),
                )?
            }
            (LegacyVmMemberKind::Location, LegacyVmMemberValue::Location(location)) => {
                let storage_index = location_prefix
                    .checked_add(computed_locations.len())
                    .ok_or_else(|| LegacyObjectLeafAdoptError::VmHandleOverflow {
                        owner_kind: owner_kind.name(),
                        creation_order,
                        member: saved_member.schema.name.clone(),
                        index: usize::MAX,
                    })?;
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
                        return Err(LegacyObjectLeafAdoptError::VmMissingSector {
                            owner_kind: owner_kind.name(),
                            creation_order,
                            member: saved_member.schema.name.clone(),
                            sector,
                            count: sector_count,
                        });
                    }
                    let layer_count = engine.world.fast_grid.level.layers.len();
                    if usize::from(location.layer) >= layer_count {
                        return Err(LegacyObjectLeafAdoptError::VmMissingLayer {
                            owner_kind: owner_kind.name(),
                            creation_order,
                            member: saved_member.schema.name.clone(),
                            layer: location.layer,
                            count: layer_count,
                        });
                    }
                    let handle_index = assets
                        .scripts
                        .location_count
                        .checked_add(storage_index)
                        .ok_or_else(|| LegacyObjectLeafAdoptError::VmHandleOverflow {
                            owner_kind: owner_kind.name(),
                            creation_order,
                            member: saved_member.schema.name.clone(),
                            index: usize::MAX,
                        })?;
                    if handle_index > HANDLE_INDEX_MAX {
                        return Err(LegacyObjectLeafAdoptError::VmHandleOverflow {
                            owner_kind: owner_kind.name(),
                            creation_order,
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
                    computed_locations.push(None);
                    0
                };
                bits
            }
            _ => {
                return Err(LegacyObjectLeafAdoptError::VmSchemaMismatch {
                    owner_kind: owner_kind.name(),
                    creation_order,
                    index,
                    detail: "decoded value variant does not match decoded member kind".to_owned(),
                });
            }
        };
        heap[address..end].copy_from_slice(&bits.to_le_bytes());
    }
    Ok(Some(heap))
}

#[allow(clippy::too_many_arguments)]
fn vm_entity_handle(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    owner_kind: LegacyVmOwnerKind,
    creation_order: u32,
    member: &str,
    member_kind: &'static str,
    reference: LegacyElementRef,
    predicate: impl FnOnce(&Entity) -> bool,
) -> Result<u32, LegacyObjectLeafAdoptError> {
    let Some(entity_id) = entities.resolve_element(reference)? else {
        return Ok(0);
    };
    let runtime = engine.world.entities.get(entity_id);
    if !runtime.is_some_and(predicate) {
        return Err(LegacyObjectLeafAdoptError::VmWrongEntity {
            owner_kind: owner_kind.name(),
            creation_order,
            member_kind,
            member: member.to_owned(),
            entity_id,
        });
    }
    let index = entity_id.index() as usize;
    if index > HANDLE_INDEX_MAX {
        return Err(LegacyObjectLeafAdoptError::VmHandleOverflow {
            owner_kind: owner_kind.name(),
            creation_order,
            member: member.to_owned(),
            index,
        });
    }
    Ok(ScriptHandleCodec::actor_handle(entity_id) as u32)
}

fn require_kind(
    runtime: &Entity,
    entity_id: EntityId,
    creation_order: u32,
    saved_kind: &'static str,
    predicate: impl FnOnce(&Entity) -> bool,
) -> Result<(), LegacyObjectLeafAdoptError> {
    if predicate(runtime) {
        return Ok(());
    }
    Err(LegacyObjectLeafAdoptError::WrongEntityKind {
        creation_order,
        saved_kind,
        entity_id,
        runtime_kind: entity_kind(runtime),
    })
}

fn entity_kind(entity: &Entity) -> &'static str {
    match entity {
        Entity::Pc(_) => "PC",
        Entity::Soldier(_) => "soldier",
        Entity::Civilian(_) => "civilian",
        Entity::Fx(_) => "FX",
        Entity::Target(_) => "target",
        Entity::Bonus(_) => "bonus",
        Entity::Scroll(_) => "scroll",
        Entity::Projectile(_) => "projectile",
        Entity::Net(_) => "net",
    }
}

fn retain_dormant_object_repulsive_point(
    saved: &LegacyRepulsivePointPayload,
) -> LegacyV48ObjectRepulsivePointState {
    LegacyV48ObjectRepulsivePointState {
        position_bits: [saved.position.x.to_bits(), saved.position.y.to_bits()],
        concave: saved.concave,
        limit_left_bits: [saved.limit_left.x.to_bits(), saved.limit_left.y.to_bits()],
        limit_right_bits: [saved.limit_right.x.to_bits(), saved.limit_right.y.to_bits()],
        action_radius_bits: saved.action_radius.to_bits(),
        force_a_bits: saved.force_a.to_bits(),
        force_b_bits: saved.force_b.to_bits(),
        radius_bits: saved.radius.to_bits(),
        id: saved.id,
        affects_pcs: saved.affects_pcs,
        affects_soldiers: saved.affects_soldiers,
        affects_civilians: saved.affects_civilians,
        affects_animals: saved.affects_animals,
    }
}

fn finite(
    value: f32,
    creation_order: u32,
    field: &'static str,
) -> Result<(), LegacyObjectLeafAdoptError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(LegacyObjectLeafAdoptError::NonFinite {
            creation_order,
            field,
            value,
        })
    }
}

fn order_type(
    value: u32,
    creation_order: u32,
    field: &'static str,
) -> Result<OrderType, LegacyObjectLeafAdoptError> {
    OrderType::try_from(value).map_err(|_| LegacyObjectLeafAdoptError::UnknownEnum {
        creation_order,
        field,
        value,
    })
}

fn object_type(value: u32, creation_order: u32) -> Result<ObjectType, LegacyObjectLeafAdoptError> {
    use ObjectType::*;
    let result = match value {
        0 => None,
        1 => VirtualJumper,
        2 => VirtualListen,
        3 => Ale,
        4 => Apple,
        5 => Arrow,
        6 => Stone,
        7 => Purse,
        8 => Coin,
        9 => Net,
        10 => Wasp,
        11 => WaspNest,
        12 => Scroll,
        13 => Cape,
        14 => BonusAmulet,
        15 => BonusAle,
        16 => BonusApple,
        17 => BonusArrow,
        18 => BonusBlazon,
        19 => BonusLambLeg,
        20 => BonusNet,
        21 => BonusPlants,
        22 => BonusPurse,
        23 => BonusRansom,
        24 => BonusStone,
        25 => BonusWaspNest,
        26 => BonusAmpulla,
        27 => BonusCoronationSpoon,
        28 => BonusRichardsCrown,
        29 => BonusRoyalSeal,
        30 => BonusRoyalSceptre,
        31 => BonusDomesdayBook,
        32 => BonusSwordOfTheState,
        _ => {
            return Err(LegacyObjectLeafAdoptError::UnknownEnum {
                creation_order,
                field: "RHElementObject::mobjectType",
                value,
            });
        }
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_save::payload_base::LegacyPoint2;

    #[test]
    fn original_object_type_ordinals_map_exactly_and_strictly() {
        assert_eq!(object_type(0, 17).unwrap(), ObjectType::None);
        assert_eq!(object_type(12, 17).unwrap(), ObjectType::Scroll);
        assert_eq!(object_type(25, 17).unwrap(), ObjectType::BonusWaspNest);
        assert_eq!(
            object_type(32, 17).unwrap(),
            ObjectType::BonusSwordOfTheState
        );
        assert!(matches!(
            object_type(33, 17),
            Err(LegacyObjectLeafAdoptError::UnknownEnum {
                creation_order: 17,
                field: "RHElementObject::mobjectType",
                value: 33,
            })
        ));
    }

    #[test]
    fn object_apply_replaces_every_authoritative_shared_member() {
        let mut runtime = ObjectData::default();
        apply_object(
            &mut runtime,
            PlannedObject {
                terminate: true,
                quantity: 9,
                animation: OrderType::WalkingUpright,
                object_type: ObjectType::BonusApple,
                associated_action: Action::Apple,
                belongs_to_beggar: true,
                taken: true,
                legacy_v48_repulsive_point: LegacyV48ObjectRepulsivePointState {
                    position_bits: [1.0f32.to_bits(), 2.0f32.to_bits()],
                    concave: false,
                    limit_left_bits: [0; 2],
                    limit_right_bits: [0; 2],
                    action_radius_bits: 10.0f32.to_bits(),
                    force_a_bits: 0.1f32.to_bits(),
                    force_b_bits: (-1.0f32).to_bits(),
                    radius_bits: 1.0f32.to_bits(),
                    id: 7,
                    affects_pcs: true,
                    affects_soldiers: true,
                    affects_civilians: true,
                    affects_animals: true,
                },
            },
        );
        assert!(runtime.terminate);
        assert_eq!(runtime.quantity, 9);
        assert_eq!(runtime.animation, OrderType::WalkingUpright);
        assert_eq!(runtime.object_type, ObjectType::BonusApple);
        assert_eq!(runtime.associated_action, Action::Apple);
        assert!(runtime.belongs_to_beggar);
        assert!(runtime.taken);
        assert_eq!(
            runtime
                .legacy_v48_repulsive_point
                .expect("dormant point storage")
                .id,
            7
        );
    }

    #[test]
    fn dormant_object_repulsive_payload_retains_non_finite_bits_exactly() {
        let point = LegacyRepulsivePointPayload {
            position: LegacyPoint2 {
                x: f32::from_bits(0x7fc0_0001),
                y: -0.0,
            },
            concave: true,
            limit_left: LegacyPoint2 {
                x: f32::INFINITY,
                y: f32::NEG_INFINITY,
            },
            limit_right: LegacyPoint2 {
                x: 1.0,
                y: f32::from_bits(0xffc0_1234),
            },
            action_radius: f32::from_bits(0x7fc0_0101),
            force_a: f32::from_bits(0x7fc0_0202),
            force_b: f32::from_bits(0xffc0_0303),
            radius: f32::from_bits(0x7fc0_0404),
            id: 8,
            affects_pcs: true,
            affects_soldiers: false,
            affects_civilians: true,
            affects_animals: true,
        };
        let retained = retain_dormant_object_repulsive_point(&point);
        assert_eq!(retained.position_bits, [0x7fc0_0001, (-0.0f32).to_bits()]);
        assert!(retained.concave);
        assert_eq!(
            retained.limit_left_bits,
            [f32::INFINITY.to_bits(), f32::NEG_INFINITY.to_bits()]
        );
        assert_eq!(retained.limit_right_bits, [1.0f32.to_bits(), 0xffc0_1234]);
        assert_eq!(retained.action_radius_bits, 0x7fc0_0101);
        assert_eq!(retained.force_a_bits, 0x7fc0_0202);
        assert_eq!(retained.force_b_bits, 0xffc0_0303);
        assert_eq!(retained.radius_bits, 0x7fc0_0404);
        assert_eq!(retained.id, 8);
        assert!(retained.affects_pcs);
        assert!(!retained.affects_soldiers);
        assert!(retained.affects_civilians);
        assert!(retained.affects_animals);

        let json = serde_json::to_string(&retained).expect("raw-bit sidecar must be JSON-safe");
        assert!(!json.contains("NaN"));
        assert!(!json.contains("Infinity"));
        let round_trip: LegacyV48ObjectRepulsivePointState =
            serde_json::from_str(&json).expect("raw-bit sidecar must round-trip through JSON");
        assert_eq!(round_trip, retained);
    }
}
