//! Mission-to-v48 topology adapters.
//!
//! Original v48 saves omit the sizes of several mission-created arrays.  The
//! decoder must recover those sizes from the already initialized mission,
//! following the same construction order as the Original.  This module only
//! exposes mappings for facts the Rust engine currently retains exactly.
//! Facts discarded during level loading fail with a named, typed error rather
//! than being reconstructed heuristically.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::{
    element::{AiBrain, Entity, EntityId, ObjectType},
    engine::{EngineInner, LevelAssets},
    level_data::{MissionElementChunk, MissionElementGroup, ProtoElementChunk, WaypointCommand},
};

use super::{
    elements::LegacyElementClass,
    payload_ai::LegacyLocalAiKind,
    payload_context::{LegacyElementPayloadMetadata, LegacyMissionPayloadMetadata},
    post_grid::{
        LegacyGateTopology, LegacyGridTopology, LegacyPatchFxTopology, LegacyPatchTopology,
        LegacyScriptObjectTopology, LegacySectorTopology,
    },
    post_hiking::{LegacyHikingGuideTopology, LegacyHikingPathTopology, LegacyWaypointTopology},
    post_tail::LegacyPostTailTopology,
};

/// A non-self-describing v48 fact which the current Rust mission model does
/// not retain with enough identity/order information to reproduce exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyMissingTopologyFact {
    /// `RHElement::mulCreationOrder` for each live static element.
    ///
    /// Entity slots preserve element order, but not hidden constructor draws
    /// from `gulCreationCounter` (notably each engine-owned mobile master).
    ElementCreationOrders,
    /// Full `RHFastFindGrid::marrayGates`, including byte-less jump gates.
    ///
    /// Rust keeps doors and jump-line geometry in separate collections and
    /// discards their interleaving in the Original gate array.
    GridGateOrder,
    /// Sparse `RHFastFindGrid::marraySectors`, including null holes and the
    /// separately appended out-of-map sector.
    GridSparseSectorOrder,
}

/// Strict failure returned while deriving omitted v48 save topology.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LegacyTopologyAdapterError {
    MissingRetainedFact {
        fact: LegacyMissingTopologyFact,
        original_owner: &'static str,
        detail: &'static str,
    },
    MissionAttachmentMismatch {
        fact: &'static str,
        engine_value: String,
        asset_value: String,
    },
    ElementTopologyMismatch {
        detail: String,
    },
}

impl fmt::Display for LegacyTopologyAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRetainedFact {
                fact,
                original_owner,
                detail,
            } => write!(
                formatter,
                "cannot derive {fact:?}: Original owns it in {original_owner}; {detail}"
            ),
            Self::MissionAttachmentMismatch {
                fact,
                engine_value,
                asset_value,
            } => write!(
                formatter,
                "cannot derive {fact}: initialized engine value {engine_value} does not match attached level asset value {asset_value}"
            ),
            Self::ElementTopologyMismatch { detail } => {
                write!(
                    formatter,
                    "cannot derive Original element topology: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for LegacyTopologyAdapterError {}

/// Original's pre-mission constructor prefix.
///
/// `RHEngine::CreateMasters` constructs thirty `RHElementObjectMaster`
/// instances and the engine constructor creates one standalone projectile
/// trajectory. None enters `marrayElements`, but all increment
/// `RHElement::gulCreationCounter`.
const PRE_LEVEL_ELEMENT_COUNT: u32 = 31;

/// Exact static element identity derived from one initialized mission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyStaticElementTopology {
    pub payload_metadata: LegacyMissionPayloadMetadata,
    /// Rust entity IDs omit mobile masters, so keep this explicit mapping
    /// instead of inviting callers to use `EntityId::index() + 31`.
    pub creation_order_by_entity: BTreeMap<EntityId, u32>,
    /// Original `RHEngine::mulNumberOfCreatedStaticElements`.
    pub static_creation_order_boundary: u32,
}

/// Derive exact Original static element order and all phase-two shape facts.
///
/// Original provenance:
///
/// - `RHElement::RHElement` assigns `mulCreationOrder = gulCreationCounter++`.
/// - `RHEngine::CreateMasters` plus the projectile helper consume 31 orders.
/// - Every later mission-created `RHElement` is inserted into
///   `marrayElements` in construction order.
/// - `RHEngine::AddElement(RHElementMobile*)` inserts the already-constructed
///   mobile master followed immediately by all of its masked child FX.
/// - `PopulateBeamMes` constructs PCs only after every mission chunk.
///
/// Rust deliberately stores mobile masters outside `Entities` and currently
/// constructs category batches independently of source chunk order. The
/// retained `LevelEntityAssets` source-order descriptors are therefore
/// authoritative; this function refuses to infer a missing descriptor.
pub fn derive_static_element_topology(
    engine: &EngineInner,
    assets: &LevelAssets,
) -> Result<LegacyStaticElementTopology, LegacyTopologyAdapterError> {
    let sequence = build_original_static_element_sequence(engine, assets)?;
    let mut payload_metadata = LegacyMissionPayloadMetadata::default();
    let mut creation_order_by_entity = BTreeMap::new();

    for (original_slot, element) in sequence.iter().enumerate() {
        let slot = u32::try_from(original_slot).map_err(|_| {
            element_mismatch(format!("static element slot {original_slot} exceeds u32"))
        })?;
        let creation_order = PRE_LEVEL_ELEMENT_COUNT
            .checked_add(slot)
            .ok_or_else(|| element_mismatch("static creation order overflow"))?;
        let (entity_id, metadata) = match *element {
            StaticElementSource::Entity(entity_id) => {
                let entity = engine.world.entities.get(entity_id).ok_or_else(|| {
                    element_mismatch(format!(
                        "retained static entity {entity_id} is absent from initialized engine"
                    ))
                })?;
                (
                    Some(entity_id),
                    metadata_for_entity(engine, entity_id, entity)?,
                )
            }
            StaticElementSource::MobileMaster(mobile_index) => {
                let mobile = engine
                    .world
                    .mobile_elements
                    .get(mobile_index)
                    .ok_or_else(|| {
                        element_mismatch(format!(
                        "retained mobile master {mobile_index} is absent from initialized engine"
                    ))
                    })?;
                (
                    None,
                    LegacyElementPayloadMetadata {
                        class: LegacyElementClass::Mobile,
                        script_class: None,
                        local_ai_kind: None,
                        mobile_sprite_count: Some(mobile.sprite_ids.len()),
                    },
                )
            }
        };
        if let Some(entity_id) = entity_id {
            if creation_order_by_entity
                .insert(entity_id, creation_order)
                .is_some()
            {
                return Err(element_mismatch(format!(
                    "static entity {entity_id} occurs more than once in retained construction order"
                )));
            }
        }
        if payload_metadata
            .by_creation_order
            .insert(creation_order, metadata)
            .is_some()
        {
            return Err(element_mismatch(format!(
                "duplicate static creation order {creation_order}"
            )));
        }
    }

    let sequence_len = u32::try_from(sequence.len())
        .map_err(|_| element_mismatch("static element count exceeds u32"))?;
    let static_creation_order_boundary = PRE_LEVEL_ELEMENT_COUNT
        .checked_add(sequence_len)
        .ok_or_else(|| element_mismatch("static creation-order boundary overflow"))?;
    payload_metadata.static_creation_order_boundary = static_creation_order_boundary;

    Ok(LegacyStaticElementTopology {
        payload_metadata,
        creation_order_by_entity,
        static_creation_order_boundary,
    })
}

/// Derive phase-two element metadata.
pub fn derive_element_payload_metadata(
    engine: &EngineInner,
    assets: &LevelAssets,
) -> Result<LegacyMissionPayloadMetadata, LegacyTopologyAdapterError> {
    Ok(derive_static_element_topology(engine, assets)?.payload_metadata)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StaticElementSource {
    Entity(EntityId),
    MobileMaster(usize),
}

fn build_original_static_element_sequence(
    engine: &EngineInner,
    assets: &LevelAssets,
) -> Result<Vec<StaticElementSource>, LegacyTopologyAdapterError> {
    let entity_assets = &assets.entities;
    if engine.world.mobile_elements.len() != entity_assets.mobile_element_count {
        return Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
            fact: "mobile element count",
            engine_value: engine.world.mobile_elements.len().to_string(),
            asset_value: entity_assets.mobile_element_count.to_string(),
        });
    }

    let patch_ids = entity_assets
        .patch_animation_entities
        .iter()
        .enumerate()
        .map(|(patch_index, handle)| {
            let handle = handle.ok_or_else(|| {
                element_mismatch(format!(
                    "patch {patch_index} has no retained FX entity handle"
                ))
            })?;
            let slot =
                crate::natives::ScriptHandleCodec::actor_handle_index(handle).ok_or_else(|| {
                    element_mismatch(format!(
                        "patch {patch_index} FX handle {handle} is not an actor handle"
                    ))
                })?;
            let slot = u32::try_from(slot).map_err(|_| {
                element_mismatch(format!("patch {patch_index} FX entity slot exceeds u32"))
            })?;
            engine
                .world
                .entities
                .id_at_legacy_slot(slot)
                .ok_or_else(|| {
                    element_mismatch(format!(
                        "patch {patch_index} FX handle {handle} is absent from initialized engine"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if entity_assets.legacy_proto_patch_count > patch_ids.len() {
        return Err(element_mismatch(format!(
            "retained proto patch count {} exceeds total patch FX count {}",
            entity_assets.legacy_proto_patch_count,
            patch_ids.len()
        )));
    }
    let (proto_patch_ids, mission_patch_ids) =
        patch_ids.split_at(entity_assets.legacy_proto_patch_count);
    let patch_id_set = patch_ids.iter().copied().collect::<BTreeSet<_>>();

    let proto_animation_ids = engine
        .world
        .entities
        .occupied()
        .filter_map(|(id, entity)| match entity {
            Entity::Fx(fx)
                if fx.fx.mobile_index.is_none()
                    && !patch_id_set.contains(&id)
                    && fx.fx.patch_index.is_none() =>
            {
                Some(id)
            }
            _ => None,
        })
        .take(entity_assets.legacy_proto_animation_count)
        .collect::<Vec<_>>();
    if proto_animation_ids.len() != entity_assets.legacy_proto_animation_count {
        return Err(element_mismatch(format!(
            "retained proto animation count {} has only {} matching initialized FX",
            entity_assets.legacy_proto_animation_count,
            proto_animation_ids.len()
        )));
    }

    let civilians = entities_matching(engine, |entity| matches!(entity, Entity::Civilian(_)));
    let targets = entities_matching(engine, |entity| matches!(entity, Entity::Target(_)));
    let bonuses = entities_matching(engine, |entity| matches!(entity, Entity::Bonus(_)));
    let rescue_pcs = entities_matching(
        engine,
        |entity| matches!(entity, Entity::Pc(pc) if pc.pc.beam_me_index < 0),
    );
    let beam_pcs = entities_matching(
        engine,
        |entity| matches!(entity, Entity::Pc(pc) if pc.pc.beam_me_index >= 0),
    );
    let soldiers = validate_typed_id_list(
        engine,
        "soldier",
        &entity_assets.soldier_entity_ids,
        |entity| matches!(entity, Entity::Soldier(_)),
    )?;
    let scrolls = validate_typed_id_list(
        engine,
        "scroll",
        &entity_assets.scroll_entity_ids,
        |entity| matches!(entity, Entity::Scroll(_)),
    )?;

    let mut sequence = Vec::new();
    let mut used = BTreeSet::new();
    let mut append_entities = |ids: &[EntityId],
                               sequence: &mut Vec<StaticElementSource>|
     -> Result<(), LegacyTopologyAdapterError> {
        for &id in ids {
            if !used.insert(id) {
                return Err(element_mismatch(format!(
                    "static entity {id} belongs to more than one retained construction group"
                )));
            }
            sequence.push(StaticElementSource::Entity(id));
        }
        Ok(())
    };

    for chunk in &entity_assets.legacy_proto_element_chunk_order {
        match chunk {
            ProtoElementChunk::Animation => append_entities(&proto_animation_ids, &mut sequence)?,
            ProtoElementChunk::Patch => append_entities(proto_patch_ids, &mut sequence)?,
        }
    }
    if (!proto_animation_ids.is_empty() || !proto_patch_ids.is_empty())
        && entity_assets.legacy_proto_element_chunk_order.is_empty()
    {
        return Err(missing_element_order(
            "proto RHElement constructors exist but proto element chunk order was not retained",
        ));
    }

    for chunk in &entity_assets.legacy_mission_element_chunk_order {
        match chunk {
            MissionElementChunk::Element => {
                if entity_assets.legacy_mission_element_group_order.is_empty()
                    && (!civilians.is_empty()
                        || !soldiers.is_empty()
                        || !targets.is_empty()
                        || !rescue_pcs.is_empty())
                {
                    return Err(missing_element_order(
                        "mission actor constructors exist but ELEMENT group order was not retained",
                    ));
                }
                for group in &entity_assets.legacy_mission_element_group_order {
                    match group {
                        MissionElementGroup::Civilian => {
                            append_entities(&civilians, &mut sequence)?
                        }
                        MissionElementGroup::Soldier => append_entities(&soldiers, &mut sequence)?,
                        MissionElementGroup::Target => append_entities(&targets, &mut sequence)?,
                        MissionElementGroup::PcToRescue => {
                            append_entities(&rescue_pcs, &mut sequence)?
                        }
                        MissionElementGroup::BeamMe | MissionElementGroup::Animal => {}
                    }
                }
            }
            MissionElementChunk::Scroll => append_entities(&scrolls, &mut sequence)?,
            MissionElementChunk::Bonus => append_entities(&bonuses, &mut sequence)?,
            MissionElementChunk::Patch => append_entities(mission_patch_ids, &mut sequence)?,
            MissionElementChunk::Mobile => {
                for (mobile_index, mobile) in engine.world.mobile_elements.iter().enumerate() {
                    sequence.push(StaticElementSource::MobileMaster(mobile_index));
                    append_entities(&mobile.sprite_ids, &mut sequence)?;
                }
            }
        }
    }
    if (!mission_patch_ids.is_empty()
        || !civilians.is_empty()
        || !soldiers.is_empty()
        || !targets.is_empty()
        || !rescue_pcs.is_empty()
        || !scrolls.is_empty()
        || !bonuses.is_empty()
        || !engine.world.mobile_elements.is_empty())
        && entity_assets.legacy_mission_element_chunk_order.is_empty()
    {
        return Err(missing_element_order(
            "mission RHElement constructors exist but mission element chunk order was not retained",
        ));
    }

    // `RHEngine::PopulateBeamMes` runs after the complete mission chunk loop.
    append_entities(&beam_pcs, &mut sequence)?;

    validate_mobile_children(engine, &used)?;
    Ok(sequence)
}

fn entities_matching(engine: &EngineInner, predicate: impl Fn(&Entity) -> bool) -> Vec<EntityId> {
    engine
        .world
        .entities
        .occupied()
        .filter_map(|(id, entity)| predicate(entity).then_some(id))
        .collect()
}

fn validate_typed_id_list(
    engine: &EngineInner,
    kind: &'static str,
    ids: &[EntityId],
    predicate: impl Fn(&Entity) -> bool,
) -> Result<Vec<EntityId>, LegacyTopologyAdapterError> {
    ids.iter()
        .copied()
        .map(|id| {
            let entity = engine.world.entities.get(id).ok_or_else(|| {
                element_mismatch(format!(
                    "retained {kind} entity {id} is absent from initialized engine"
                ))
            })?;
            if !predicate(entity) {
                return Err(element_mismatch(format!(
                    "retained {kind} entity {id} has incompatible Rust variant"
                )));
            }
            Ok(id)
        })
        .collect()
}

fn validate_mobile_children(
    engine: &EngineInner,
    used: &BTreeSet<EntityId>,
) -> Result<(), LegacyTopologyAdapterError> {
    for (mobile_index, mobile) in engine.world.mobile_elements.iter().enumerate() {
        if mobile.sprite_ids.is_empty() {
            return Err(element_mismatch(format!(
                "mobile {mobile_index} has no masked child sprites"
            )));
        }
        for &id in &mobile.sprite_ids {
            let Some(Entity::Fx(fx)) = engine.world.entities.get(id) else {
                return Err(element_mismatch(format!(
                    "mobile {mobile_index} child {id} is absent or not FX"
                )));
            };
            if fx.fx.mobile_index != u16::try_from(mobile_index).ok() {
                return Err(element_mismatch(format!(
                    "mobile {mobile_index} child {id} records owner {:?}",
                    fx.fx.mobile_index
                )));
            }
            if !used.contains(&id) {
                return Err(element_mismatch(format!(
                    "mobile {mobile_index} child {id} was not inserted into retained Original order"
                )));
            }
        }
    }
    Ok(())
}

fn metadata_for_entity(
    engine: &EngineInner,
    entity_id: EntityId,
    entity: &Entity,
) -> Result<LegacyElementPayloadMetadata, LegacyTopologyAdapterError> {
    let bound_actor_class = || {
        engine.scripts.mission.as_ref().and_then(|mission| {
            mission
                .actor_vm_class_and_heap(crate::natives::ScriptHandleCodec::actor_handle(entity_id))
                .map(|(class, _)| class.class_name.clone())
        })
    };
    let (class, script_class, local_ai_kind) = match entity {
        Entity::Pc(pc) => (
            LegacyElementClass::ActorPc,
            bound_actor_class().or_else(|| nonempty(&pc.actor.script_class)),
            None,
        ),
        Entity::Soldier(soldier) => {
            let kind = match soldier.npc.ai_brain {
                AiBrain::Enemy(_) => LegacyLocalAiKind::Enemy,
                ref actual => {
                    return Err(element_mismatch(format!(
                        "ActorNpcSoldier has incompatible local AI {actual:?}; expected Enemy"
                    )));
                }
            };
            (
                LegacyElementClass::ActorNpcSoldier,
                bound_actor_class().or_else(|| nonempty(&soldier.actor.script_class)),
                Some(kind),
            )
        }
        Entity::Civilian(civilian) => {
            let kind = match civilian.npc.ai_brain {
                AiBrain::Friendly(_) => LegacyLocalAiKind::Friendly,
                ref actual => {
                    return Err(element_mismatch(format!(
                        "ActorNpcCivilian has incompatible local AI {actual:?}; expected Friendly"
                    )));
                }
            };
            (
                LegacyElementClass::ActorNpcCivilian,
                bound_actor_class().or_else(|| nonempty(&civilian.actor.script_class)),
                Some(kind),
            )
        }
        Entity::Fx(fx) => (
            if fx.fx.mobile_index.is_some() {
                LegacyElementClass::FxMasked
            } else {
                LegacyElementClass::Fx
            },
            None,
            None,
        ),
        Entity::Target(target) => (
            LegacyElementClass::Target,
            nonempty(&target.target.script_class),
            None,
        ),
        Entity::Scroll(scroll) => (
            LegacyElementClass::Scroll,
            nonempty(&scroll.script_class),
            None,
        ),
        Entity::Bonus(bonus) => (legacy_object_class(bonus.object.object_type)?, None, None),
        Entity::Projectile(projectile) => (
            legacy_object_class(projectile.object.object_type)?,
            None,
            None,
        ),
        Entity::Net(_) => (LegacyElementClass::Net, None, None),
    };
    Ok(LegacyElementPayloadMetadata {
        class,
        script_class,
        local_ai_kind,
        mobile_sprite_count: None,
    })
}

fn legacy_object_class(
    object_type: ObjectType,
) -> Result<LegacyElementClass, LegacyTopologyAdapterError> {
    Ok(match object_type {
        ObjectType::Arrow => LegacyElementClass::Arrow,
        ObjectType::Apple => LegacyElementClass::Apple,
        ObjectType::Purse => LegacyElementClass::Purse,
        ObjectType::Stone => LegacyElementClass::Stone,
        ObjectType::WaspNest => LegacyElementClass::WaspNest,
        ObjectType::Wasp => LegacyElementClass::Wasp,
        ObjectType::Net => LegacyElementClass::Net,
        ObjectType::Coin => LegacyElementClass::Coin,
        ObjectType::Ale => LegacyElementClass::Ale,
        ObjectType::Cape => LegacyElementClass::SpyCape,
        ObjectType::Scroll => LegacyElementClass::Scroll,
        ObjectType::BonusAle => LegacyElementClass::BonusAle,
        ObjectType::BonusAmulet => LegacyElementClass::BonusAmulet,
        ObjectType::BonusArrow => LegacyElementClass::BonusArrow,
        ObjectType::BonusApple => LegacyElementClass::BonusApple,
        ObjectType::BonusBlazon => LegacyElementClass::BonusBlazon,
        ObjectType::BonusLambLeg => LegacyElementClass::BonusLambLeg,
        ObjectType::BonusNet => LegacyElementClass::BonusNet,
        ObjectType::BonusPlants => LegacyElementClass::BonusPlants,
        ObjectType::BonusPurse => LegacyElementClass::BonusPurse,
        ObjectType::BonusStone => LegacyElementClass::BonusStone,
        ObjectType::BonusWaspNest => LegacyElementClass::BonusWaspNest,
        ObjectType::BonusRansom => LegacyElementClass::BonusRansom,
        ObjectType::BonusAmpulla => LegacyElementClass::BonusAmpulla,
        ObjectType::BonusCoronationSpoon => LegacyElementClass::BonusCoronationSpoon,
        ObjectType::BonusRichardsCrown => LegacyElementClass::BonusRichardsCrown,
        ObjectType::BonusRoyalSeal => LegacyElementClass::BonusRoyalSeal,
        ObjectType::BonusRoyalSceptre => LegacyElementClass::BonusRoyalSceptre,
        ObjectType::BonusDomesdayBook => LegacyElementClass::BonusDomesdayBook,
        ObjectType::BonusSwordOfTheState => LegacyElementClass::BonusSwordOfTheState,
        ObjectType::None | ObjectType::VirtualJumper | ObjectType::VirtualListen => {
            return Err(element_mismatch(format!(
                "initialized marrayElements entity has non-concrete object type {object_type:?}"
            )));
        }
    })
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn element_mismatch(detail: impl Into<String>) -> LegacyTopologyAdapterError {
    LegacyTopologyAdapterError::ElementTopologyMismatch {
        detail: detail.into(),
    }
}

fn missing_element_order(detail: &'static str) -> LegacyTopologyAdapterError {
    LegacyTopologyAdapterError::MissingRetainedFact {
        fact: LegacyMissingTopologyFact::ElementCreationOrders,
        original_owner: "retained proto/mission RHElement construction order",
        detail,
    }
}

/// Derive the exact `RHFastFindGrid::Serialize` walk topology.
///
/// Patch order and serializing door order can be recovered, but the requested
/// result represents the *full* arrays. Returning a wire-equivalent topology
/// with jump gates omitted would hide a mission-identity mismatch, so the
/// adapter remains strict.
pub fn derive_grid_topology(
    engine: &EngineInner,
    assets: &LevelAssets,
) -> Result<LegacyGridTopology, LegacyTopologyAdapterError> {
    use crate::engine::{LegacyGridGateAsset, LegacyGridScriptObjectAsset, LegacyGridSectorAsset};

    let retained = assets.legacy_grid_topology.as_ref().ok_or(
        LegacyTopologyAdapterError::MissingRetainedFact {
            fact: LegacyMissingTopologyFact::GridSparseSectorOrder,
            original_owner: "RHFastFindGrid construction-time arrays",
            detail: "the attached level assets do not contain source-derived legacy grid topology",
        },
    )?;
    let static_elements = derive_static_element_topology(engine, assets)?;

    if retained.patches.len() != engine.script_domains.interactables.patches.len() {
        return Err(attachment_mismatch(
            "patch count",
            engine.script_domains.interactables.patches.len(),
            retained.patches.len(),
        ));
    }

    let runtime_door_count = engine
        .script_domains
        .interactables
        .doors
        .iter()
        .filter(|door| door.gate_type == crate::gate::GateType::Door)
        .count();
    let runtime_jump_count = engine
        .script_domains
        .interactables
        .doors
        .iter()
        .filter(|door| door.gate_type == crate::gate::GateType::Jump)
        .count();
    let retained_door_count = retained
        .gates
        .iter()
        .filter(|gate| matches!(gate, LegacyGridGateAsset::Door))
        .count();
    let retained_jump_count = retained.gates.len() - retained_door_count;
    if (runtime_door_count, runtime_jump_count) != (retained_door_count, retained_jump_count) {
        return Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
            fact: "door/jump gate counts",
            engine_value: format!("doors={runtime_door_count}, jumps={runtime_jump_count}"),
            asset_value: format!("doors={retained_door_count}, jumps={retained_jump_count}"),
        });
    }
    if retained.script_objects.len() != assets.scripts.location_count {
        return Err(attachment_mismatch(
            "script object count",
            assets.scripts.location_count,
            retained.script_objects.len(),
        ));
    }
    validate_special_sector_counts(engine, retained)?;

    let patches = retained
        .patches
        .iter()
        .map(|patch| {
            let handle = patch.fx_entity_handle.ok_or_else(|| {
                element_mismatch(format!(
                    "retained patch {} has no RHElementFX handle",
                    patch.patch_index
                ))
            })?;
            let slot =
                crate::natives::ScriptHandleCodec::actor_handle_index(handle).ok_or_else(|| {
                    element_mismatch(format!(
                        "retained patch {} FX handle {handle} is not an actor handle",
                        patch.patch_index
                    ))
                })?;
            let slot = u32::try_from(slot)
                .map_err(|_| element_mismatch("patch FX entity slot exceeds u32"))?;
            let entity_id = engine
                .world
                .entities
                .id_at_legacy_slot(slot)
                .ok_or_else(|| {
                    element_mismatch(format!(
                        "retained patch {} FX slot {slot} is absent",
                        patch.patch_index
                    ))
                })?;
            let creation_order = static_elements
                .creation_order_by_entity
                .get(&entity_id)
                .copied()
                .ok_or_else(|| {
                    element_mismatch(format!(
                        "retained patch {} FX entity {entity_id} has no Original creation order",
                        patch.patch_index
                    ))
                })?;
            Ok(LegacyPatchTopology {
                layer: patch.layer,
                index_in_layer: patch.index_in_layer,
                fx: Some(LegacyPatchFxTopology {
                    creation_order,
                    class: LegacyElementClass::Fx,
                }),
            })
        })
        .collect::<Result<Vec<_>, LegacyTopologyAdapterError>>()?;

    Ok(LegacyGridTopology {
        patches,
        gates: retained
            .gates
            .iter()
            .map(|gate| match gate {
                LegacyGridGateAsset::Door => LegacyGateTopology::Door,
                LegacyGridGateAsset::Stateless => LegacyGateTopology::Stateless,
            })
            .collect(),
        script_objects: retained
            .script_objects
            .iter()
            .map(|object| match object {
                LegacyGridScriptObjectAsset::NonSector => LegacyScriptObjectTopology::NonSector,
                LegacyGridScriptObjectAsset::Sector { associated_class } => {
                    LegacyScriptObjectTopology::Sector {
                        associated_class: associated_class.clone(),
                    }
                }
            })
            .collect(),
        sectors: retained
            .sectors
            .iter()
            .map(|sector| match sector {
                LegacyGridSectorAsset::NullOrOrdinary => LegacySectorTopology::NullOrOrdinary,
                LegacyGridSectorAsset::Door { .. } => LegacySectorTopology::Door,
                LegacyGridSectorAsset::Building => LegacySectorTopology::Building,
                LegacyGridSectorAsset::Lift => LegacySectorTopology::Lift,
            })
            .collect(),
    })
}

fn attachment_mismatch(
    fact: &'static str,
    engine_value: usize,
    asset_value: usize,
) -> LegacyTopologyAdapterError {
    LegacyTopologyAdapterError::MissionAttachmentMismatch {
        fact,
        engine_value: engine_value.to_string(),
        asset_value: asset_value.to_string(),
    }
}

fn validate_special_sector_counts(
    engine: &EngineInner,
    retained: &crate::engine::LegacyGridTopologyAssets,
) -> Result<(), LegacyTopologyAdapterError> {
    use crate::engine::LegacyGridSectorAsset;

    let count_runtime = |predicate: fn(crate::sector::SectorType) -> bool| {
        engine
            .world
            .fast_grid
            .level
            .sectors
            .iter()
            .filter(|sector| predicate(sector.sector_type))
            .count()
    };
    let runtime = (
        count_runtime(crate::sector::SectorType::is_door),
        count_runtime(crate::sector::SectorType::is_building),
        count_runtime(crate::sector::SectorType::is_lift),
    );
    let assets = (
        retained
            .sectors
            .iter()
            .filter(|sector| matches!(sector, LegacyGridSectorAsset::Door { .. }))
            .count(),
        retained
            .sectors
            .iter()
            .filter(|sector| matches!(sector, LegacyGridSectorAsset::Building))
            .count(),
        retained
            .sectors
            .iter()
            .filter(|sector| matches!(sector, LegacyGridSectorAsset::Lift))
            .count(),
    );
    if runtime != assets {
        return Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
            fact: "door/building/lift sector counts",
            engine_value: format!(
                "doors={}, buildings={}, lifts={}",
                runtime.0, runtime.1, runtime.2
            ),
            asset_value: format!(
                "doors={}, buildings={}, lifts={}",
                assets.0, assets.1, assets.2
            ),
        });
    }
    Ok(())
}

/// Derive `RHHikingGuide::marrayHikingPathes` in its exact stored order.
///
/// Original provenance:
/// `original-code/RHhikingguide.cpp::RHHikingGuide::Serialize` walks paths
/// then waypoints without serializing either count. `RHWaypoint::Serialize`
/// serializes members only when global scripting and `bCommandIsScript` are
/// both true.
pub fn derive_hiking_guide_topology(
    engine: &EngineInner,
    assets: &LevelAssets,
) -> Result<LegacyHikingGuideTopology, LegacyTopologyAdapterError> {
    let script_enabled = engine.scripts.mission.is_some();
    let asset_script_enabled = assets.scripts.mission_name.is_some();
    if script_enabled != asset_script_enabled {
        return Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
            fact: "global script enablement",
            engine_value: script_enabled.to_string(),
            asset_value: asset_script_enabled.to_string(),
        });
    }

    Ok(map_hiking_paths(&assets.hiking_paths, script_enabled))
}

fn map_hiking_paths(
    paths: &[crate::level_data::RawHikingPath],
    script_enabled: bool,
) -> LegacyHikingGuideTopology {
    LegacyHikingGuideTopology {
        paths: paths
            .iter()
            .map(|path| LegacyHikingPathTopology {
                waypoints: path
                    .waypoints
                    .iter()
                    .map(|waypoint| LegacyWaypointTopology {
                        script_class: match &waypoint.command {
                            WaypointCommand::Script(class) if script_enabled => Some(class.clone()),
                            WaypointCommand::None
                            | WaypointCommand::Macro(_)
                            | WaypointCommand::Script(_) => None,
                        },
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Derive the mission-sized arrays consumed after `RHTitbits::Serialize`.
///
/// The global engine VM is always the SCB `StartUp` class in the Rust port,
/// matching `MissionScript::from_manager`. Seek/archery arrays are read from
/// the initialized AI runtime because construction may merge authored seek
/// directions. Pathfinder counts come from the runtime state matrix that the
/// save bytes restore, and are checked against the attached static graph.
pub fn derive_post_tail_topology(
    engine: &EngineInner,
    assets: &LevelAssets,
    eof_offset: u64,
) -> Result<LegacyPostTailTopology, LegacyTopologyAdapterError> {
    let global_script_class = match (
        engine.scripts.mission.as_ref(),
        assets.scripts.mission_name.as_deref(),
    ) {
        (None, None) => None,
        (Some(script), Some(asset_name)) if script.script_name == asset_name => {
            Some("StartUp".to_owned())
        }
        (engine_script, asset_script) => {
            return Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
                fact: "mission script identity",
                engine_value: engine_script.map_or_else(
                    || "disabled".to_owned(),
                    |script| script.script_name.clone(),
                ),
                asset_value: asset_script.unwrap_or("disabled").to_owned(),
            });
        }
    };

    let runtime_area_counts: Vec<usize> = engine
        .world
        .pathfinder
        .states
        .iter()
        .map(Vec::len)
        .collect();
    let asset_area_counts: Vec<usize> = assets
        .pathfinder_graph
        .layers
        .iter()
        .map(Vec::len)
        .collect();
    if runtime_area_counts != asset_area_counts {
        return Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
            fact: "pathfinder layer/area topology",
            engine_value: format!("{runtime_area_counts:?}"),
            asset_value: format!("{asset_area_counts:?}"),
        });
    }

    Ok(LegacyPostTailTopology {
        global_script_class,
        seek_point_count: engine.ai.global.seek_points.len(),
        archery_sector_point_counts: engine
            .ai
            .global
            .archery_sectors
            .iter()
            .map(|sector| sector.points.len())
            .collect(),
        path_graph_area_counts: runtime_area_counts,
        eof_offset,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        ai::{PointArchery, Position, SectorArchery, SeekPoint},
        ai_enemy::EnemyAi,
        coordinates::{MapPoint, MapVec},
        element::{
            ActorData, ActorPc, ActorSoldier, ElementBonus, ElementData, ElementFx, ElementTarget,
            FxData, HumanData, NpcData, ObjectData, PcData, SoldierData, TargetData,
        },
        engine::EngineInner,
        level_data::{
            MissionElementChunk, MissionElementGroup, ProtoElementChunk, RawHikingPath, RawWaypoint,
        },
        mobile::MobileElement,
        pathfinder::PathGraph,
        sector::SectorNumber,
    };

    use super::*;

    #[test]
    fn hiking_mapping_preserves_path_waypoint_order_and_script_gate() {
        let engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        assets.hiking_paths = Arc::new(vec![
            RawHikingPath {
                waypoints: vec![
                    waypoint(WaypointCommand::Macro(vec![1, 2])),
                    waypoint(WaypointCommand::Script("PatrolTurn".to_owned())),
                ],
            },
            RawHikingPath {
                waypoints: vec![waypoint(WaypointCommand::None)],
            },
        ]);

        let topology = derive_hiking_guide_topology(&engine, &assets).unwrap();
        assert_eq!(topology.paths.len(), 2);
        assert_eq!(
            topology.paths[0]
                .waypoints
                .iter()
                .map(|waypoint| waypoint.script_class.as_deref())
                .collect::<Vec<_>>(),
            vec![None, None],
        );
        assert_eq!(topology.paths[1].waypoints[0].script_class, None);

        let scripted = map_hiking_paths(&assets.hiking_paths, true);
        assert_eq!(
            scripted.paths[0].waypoints[1].script_class.as_deref(),
            Some("PatrolTurn"),
        );
    }

    #[test]
    fn hiking_rejects_engine_asset_script_enablement_mismatch() {
        let engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        assets.scripts.mission_name = Some("mission".to_owned());

        assert!(matches!(
            derive_hiking_guide_topology(&engine, &assets),
            Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
                fact: "global script enablement",
                ..
            })
        ));
    }

    #[test]
    fn post_tail_uses_runtime_ai_order_and_checked_pathfinder_shape() {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();

        engine.ai.global.seek_points = (0..3).map(seek_point).collect();
        engine.ai.global.archery_sectors = vec![archery_sector(2), archery_sector(4)];
        engine.world.pathfinder.states = vec![vec![0; 2], vec![0; 1]];

        let mut graph = PathGraph::new();
        graph.layers = vec![vec![Vec::new(); 2], vec![Vec::new(); 1]];
        assets.pathfinder_graph = Arc::new(graph);

        let topology = derive_post_tail_topology(&engine, &assets, 9_999).unwrap();
        assert_eq!(topology.global_script_class, None);
        assert_eq!(topology.seek_point_count, 3);
        assert_eq!(topology.archery_sector_point_counts, vec![2, 4]);
        assert_eq!(topology.path_graph_area_counts, vec![2, 1]);
        assert_eq!(topology.eof_offset, 9_999);
    }

    #[test]
    fn post_tail_rejects_detached_pathfinder_shape() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        engine.world.pathfinder.states = vec![vec![0]];

        assert!(matches!(
            derive_post_tail_topology(&engine, &assets, 0),
            Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
                fact: "pathfinder layer/area topology",
                ..
            })
        ));
    }

    #[test]
    fn element_metadata_preserves_original_mixed_mobile_order_and_shape() {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();

        let proto_fx = engine.add_entity(fx(None));
        let soldier = engine.add_entity(soldier(
            "GuardScript",
            AiBrain::Enemy(Box::new(EnemyAi::new(0))),
        ));
        let target = engine.add_entity(target("BellTarget"));
        let bonus = engine.add_entity(bonus(ObjectType::BonusAle));
        let scroll = engine.add_entity(Entity::Scroll(crate::element::ElementScroll {
            script_class: "ClueScroll".to_owned(),
            ..Default::default()
        }));
        let beam_pc = engine.add_entity(pc("RobinScript", 0));
        let mobile_child = engine.add_entity(fx(Some(0)));
        engine
            .world
            .mobile_elements
            .push(mobile(vec![mobile_child]));

        assets.entities.mobile_element_count = 1;
        assets.entities.soldier_entity_ids = vec![soldier];
        assets.entities.scroll_entity_ids = vec![scroll];
        assets.entities.legacy_proto_animation_count = 1;
        assets.entities.legacy_proto_element_chunk_order = vec![ProtoElementChunk::Animation];
        assets.entities.legacy_mission_element_chunk_order = vec![
            MissionElementChunk::Element,
            MissionElementChunk::Bonus,
            MissionElementChunk::Scroll,
            MissionElementChunk::Mobile,
        ];
        assets.entities.legacy_mission_element_group_order =
            vec![MissionElementGroup::Soldier, MissionElementGroup::Target];

        let topology = derive_static_element_topology(&engine, &assets).unwrap();
        assert_eq!(topology.static_creation_order_boundary, 39);
        assert_eq!(topology.creation_order_by_entity[&proto_fx], 31);
        assert_eq!(topology.creation_order_by_entity[&soldier], 32);
        assert_eq!(topology.creation_order_by_entity[&target], 33);
        assert_eq!(topology.creation_order_by_entity[&bonus], 34);
        assert_eq!(topology.creation_order_by_entity[&scroll], 35);
        assert_eq!(topology.creation_order_by_entity[&mobile_child], 37);
        assert_eq!(topology.creation_order_by_entity[&beam_pc], 38);

        let mobile_metadata = &topology.payload_metadata.by_creation_order[&36];
        assert_eq!(mobile_metadata.class, LegacyElementClass::Mobile);
        assert_eq!(mobile_metadata.mobile_sprite_count, Some(1));
        assert_eq!(
            topology.payload_metadata.by_creation_order[&32],
            LegacyElementPayloadMetadata {
                class: LegacyElementClass::ActorNpcSoldier,
                script_class: Some("GuardScript".to_owned()),
                local_ai_kind: Some(LegacyLocalAiKind::Enemy),
                mobile_sprite_count: None,
            }
        );
        assert_eq!(
            topology.payload_metadata.by_creation_order[&33]
                .script_class
                .as_deref(),
            Some("BellTarget")
        );
        assert_eq!(
            topology.payload_metadata.by_creation_order[&35]
                .script_class
                .as_deref(),
            Some("ClueScroll")
        );
        assert_eq!(
            topology.payload_metadata.by_creation_order[&38]
                .script_class
                .as_deref(),
            Some("RobinScript")
        );
    }

    #[test]
    fn element_metadata_rejects_attachment_and_local_ai_mismatches() {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let soldier = engine.add_entity(soldier("Guard", AiBrain::None));
        assets.entities.soldier_entity_ids = vec![soldier];
        assets.entities.legacy_mission_element_chunk_order = vec![MissionElementChunk::Element];
        assets.entities.legacy_mission_element_group_order = vec![MissionElementGroup::Soldier];

        let error = derive_static_element_topology(&engine, &assets).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("ActorNpcSoldier has incompatible local AI")
        );

        let child = engine.add_entity(fx(Some(0)));
        engine.world.mobile_elements.push(mobile(vec![child]));
        let error = derive_static_element_topology(&engine, &assets).unwrap_err();
        assert!(matches!(
            error,
            LegacyTopologyAdapterError::MissionAttachmentMismatch {
                fact: "mobile element count",
                ..
            }
        ));
    }

    #[test]
    fn grid_topology_requires_retained_sparse_construction_order() {
        let engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        assert!(matches!(
            derive_grid_topology(&engine, &assets),
            Err(LegacyTopologyAdapterError::MissingRetainedFact {
                fact: LegacyMissingTopologyFact::GridSparseSectorOrder,
                ..
            })
        ));

        assets.legacy_grid_topology = Some(crate::engine::LegacyGridTopologyAssets::default());
        assert_eq!(
            derive_grid_topology(&engine, &assets).unwrap(),
            LegacyGridTopology {
                patches: Vec::new(),
                gates: Vec::new(),
                script_objects: Vec::new(),
                sectors: Vec::new(),
            }
        );
    }

    fn fx(mobile_index: Option<u16>) -> Entity {
        Entity::Fx(ElementFx {
            element: ElementData {
                kind: crate::element::ElementKind::Fx,
                ..Default::default()
            },
            fx: FxData {
                mobile_index,
                ..Default::default()
            },
        })
    }

    fn soldier(script_class: &str, ai_brain: AiBrain) -> Entity {
        Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                ..Default::default()
            },
            actor: ActorData {
                script_class: script_class.to_owned(),
                ..Default::default()
            },
            human: HumanData::default(),
            npc: NpcData {
                ai_brain,
                ..Default::default()
            },
            soldier: SoldierData::default(),
        })
    }

    fn target(script_class: &str) -> Entity {
        Entity::Target(ElementTarget {
            element: ElementData {
                kind: crate::element::ElementKind::Target,
                ..Default::default()
            },
            fx: FxData::default(),
            target: TargetData {
                script_class: script_class.to_owned(),
                ..Default::default()
            },
        })
    }

    fn bonus(object_type: ObjectType) -> Entity {
        Entity::Bonus(ElementBonus {
            element: ElementData {
                kind: crate::element::ElementKind::ObjectBonus,
                ..Default::default()
            },
            object: ObjectData {
                object_type,
                ..Default::default()
            },
        })
    }

    fn pc(script_class: &str, beam_me_index: i16) -> Entity {
        Entity::Pc(ActorPc {
            element: ElementData {
                kind: crate::element::ElementKind::ActorPc,
                ..Default::default()
            },
            actor: ActorData {
                script_class: script_class.to_owned(),
                ..Default::default()
            },
            human: HumanData::default(),
            pc: PcData {
                beam_me_index,
                ..Default::default()
            },
        })
    }

    fn mobile(sprite_ids: Vec<EntityId>) -> MobileElement {
        MobileElement {
            sprite_ids,
            motion_polygon: Vec::new(),
            position: MapPoint::default(),
            old_position: MapPoint::default(),
            path_index: 0,
            current_waypoint: 0,
            forward: true,
            layer: 0,
            sector: 0,
            obstacle: None,
            active: true,
            stopped: false,
            speed: 0.0,
            speed_goal: 0.0,
            acceleration: 0.0,
            increment: MapVec::ZERO,
            goal: MapPoint::default(),
        }
    }

    fn waypoint(command: WaypointCommand) -> RawWaypoint {
        RawWaypoint {
            x: 0,
            y: 0,
            sector: 0,
            level: 0,
            command,
        }
    }

    fn archery_sector(point_count: usize) -> SectorArchery {
        SectorArchery {
            points: (0..point_count)
                .map(|_| PointArchery {
                    position: Position::default(),
                    direction: 0,
                    is_shooting_point: false,
                    sector_index: SectorNumber::new(0),
                    owner: None,
                })
                .collect(),
            polygon: vec![(0.0, 0.0)],
            layer: 0,
            index_first_shooting_point: None,
            index_last_shooting_point: None,
            num_shooting_points: 0,
            num_owners: 0,
        }
    }

    fn seek_point(id: u16) -> SeekPoint {
        SeekPoint {
            position: Position::default(),
            frame_when_full_interest: 0,
            directions: Vec::new(),
            last_calculated_interest: 100,
            locked: false,
            id,
        }
    }
}
