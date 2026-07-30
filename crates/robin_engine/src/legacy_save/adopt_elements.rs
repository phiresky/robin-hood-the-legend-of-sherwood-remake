//! Atomic adoption of the common static-element state in Original v48 saves.
//!
//! This module deliberately builds an owned, fully validated plan before
//! touching the initialized mission.  It is not wired into the replay runner
//! yet: later stages must first adopt the remaining leaf, sequence, AI, and
//! manager state into the same candidate engine.

use std::num::NonZeroU32;

use thiserror::Error;

use crate::{
    coordinates::MapPoint,
    element::{ActionState, Detectable, DetectableType, EntityId, NpcData, OutlineColorName},
    engine::EngineInner,
    order::OrderType,
    position_interface::PositionInterfaceV48State,
};

use super::{
    adopt::{
        preflight_v48_position, LegacyEntityFixups, LegacyPositionTopology, LegacySaveAdoptError,
    },
    payload_base::{
        LegacyActorPayload, LegacyElementPayloadBase, LegacyNpcPayload, LegacySpritePayload,
    },
    payload_dispatch::{LegacyElementPayload, LegacyElementPayloadStream},
    payload_objects::LegacyObjectItemPayload,
};

/// Authoritative serialized members which have no equivalent in the current
/// Rust entity model and therefore remain for later adoption stages.
///
/// Keeping the list next to the conversion prevents a partially adopted save
/// from looking complete.  In particular, `RHSprite::muwFrameCountDown` drives
/// projectile flight and must land before this plan is wired into replay.
pub const REMAINING_COMMON_ELEMENT_FIELDS: &[&str] = &[
    "RHElement::mbPositionMapDelayed / mptPositionMapDelayed",
    "RHElement::mbPositionDelayed / mptPositionDelayed",
    "RHSprite::muwFrameCountDown",
    "RHElementActor::mbIsAboutToSurrender",
    "RHElementActor::mbIsSurrendering",
    "RHElementActor::mfDistanceToBoundary",
    "RHElementActor::mmotionState",
    "RHElementActor bypass / railroad / seek-sector state",
    "RHElementActor sequence/order pointers and inline post-seek sequence",
    "RHElementActor script member variables",
    "RHElementActorNPC::mpAttachedScroll identity (Rust currently retains only attached/not attached)",
    "RHElementActorNPC::muwBodyVisitors",
    "RHElementActorNPC view, initial-position, and local-AI state",
    "RHElementActorNPC leaf Soldier/Civilian state",
];

/// Serialized bytes which are intentionally not simulation state in Rust.
///
/// Original recomputes display order and the sprite bounding box during its
/// normal render/position refreshes; the dummy is an uninitialized legacy
/// compatibility slot. `mbAlreadyDecompressed` describes a host-side sprite
/// asset cache. None may influence authoritative replay comparison.
pub const NON_AUTHORITATIVE_COMMON_ELEMENT_FIELDS: &[&str] = &[
    "RHSprite::mbAlreadyDecompressed",
    "RHSprite::mfDisplayOrder",
    "RHSprite display-order dummy",
    "RHSprite::mboundingBox",
];

#[derive(Debug, Error)]
pub enum LegacyElementAdoptError {
    #[error(transparent)]
    Common(#[from] LegacySaveAdoptError),
    #[error(
        "saved element creation order {creation_order} has delayed position state ({field}); Rust has no equivalent queue yet"
    )]
    UnsupportedDelayedPosition {
        creation_order: u32,
        field: &'static str,
    },
    #[error(
        "saved element creation order {creation_order} field {field} has unknown enum value {value}"
    )]
    UnknownEnum {
        creation_order: u32,
        field: &'static str,
        value: u32,
    },
    #[error(
        "saved element creation order {creation_order} resolves to missing Rust entity {entity_id}"
    )]
    MissingEntity {
        creation_order: u32,
        entity_id: EntityId,
    },
    #[error(
        "saved NPC creation order {creation_order} resolves to non-NPC Rust entity {entity_id}"
    )]
    ExpectedNpc {
        creation_order: u32,
        entity_id: EntityId,
    },
    #[error(
        "saved actor creation order {creation_order} resolves to non-actor Rust entity {entity_id}"
    )]
    ExpectedActor {
        creation_order: u32,
        entity_id: EntityId,
    },
    #[error(
        "saved actor creation order {creation_order} has last order ID zero, which Rust cannot represent distinctly from no order"
    )]
    ZeroLastOrderId { creation_order: u32 },
}

#[derive(Clone, Debug)]
pub struct LegacyStaticElementAdoption {
    records: Vec<ConvertedElement>,
}

#[derive(Clone, Debug)]
struct ConvertedElement {
    entity_id: EntityId,
    creation_order: u32,
    element: ConvertedElementBase,
    actor: Option<ConvertedActor>,
    npc: Option<ConvertedNpc>,
}

#[derive(Clone, Debug)]
struct ConvertedElementBase {
    outline_colors: [u16; 5],
    current_outline: OutlineColorName,
    outline_width: u16,
    custom_minimap_dot: u16,
    active: bool,
    in_honolulu: bool,
    index_in_elements_list: u16,
    blipped: bool,
    unreachable: bool,
    sprite: ConvertedSprite,
}

#[derive(Clone, Debug)]
struct ConvertedSprite {
    current_row: u16,
    current_frame: u16,
    frame_count: u16,
    current_height: u16,
    current_width: u16,
    last_action: OrderType,
    alternate_profile: bool,
    masked: bool,
    behind_display_order_reference: bool,
    display_order_reference: Option<EntityId>,
    action_done_frame: u16,
    action_done_counter: u16,
    last_sound_id: u16,
    last_processed_order_id: u32,
    animation_replacements: Vec<(OrderType, OrderType)>,
    position: PositionInterfaceV48State,
}

#[derive(Clone, Debug)]
struct ConvertedActor {
    last_order_id: Option<NonZeroU32>,
    old_action: OrderType,
    action_state: ActionState,
    execution_frozen: bool,
    ignored_for_anti_collision: bool,
    new_order: bool,
    wait_time: u32,
    seek_target: Option<EntityId>,
    last_seek_target_position: MapPoint,
    seek_distance: f32,
    passing_door_directly: bool,
    sequence_element_started: bool,
    script_class: String,
}

#[derive(Clone, Debug)]
struct ConvertedNpc {
    life: i16,
    arrows: u16,
    old_direction: i16,
    register: u16,
    attached_scroll: Option<EntityId>,
    inform: bool,
    money: u32,
    wasp: bool,
    old_deafness: u16,
    old_frame: u32,
    detectable_lists: Vec<Vec<Detectable>>,
    detection_suspects: [u16; DetectableType::COUNT],
    maximum_suspect: u16,
    worst_detectable_type: DetectableType,
    custom_values: [i32; 10],
    gave_money: bool,
}

impl LegacyStaticElementAdoption {
    /// Validate and convert every element record without mutating `engine`.
    pub fn preflight(
        engine: &EngineInner,
        payloads: &LegacyElementPayloadStream,
        entities: &LegacyEntityFixups,
        position_topology: &LegacyPositionTopology,
    ) -> Result<Self, LegacyElementAdoptError> {
        let mut records = Vec::with_capacity(payloads.records.len());
        for record in &payloads.records {
            let creation_order = record.header.creation_order;
            let entity_id = entities
                .by_creation_order
                .get(&creation_order)
                .copied()
                .ok_or(LegacySaveAdoptError::MissingCreationOrderReference { creation_order })?;
            let runtime = engine.world.entities.get(entity_id).ok_or(
                LegacyElementAdoptError::MissingEntity {
                    creation_order,
                    entity_id,
                },
            )?;
            let (base, actor, npc) = payload_parts(&record.payload);
            if actor.is_some() && runtime.actor_data().is_none() {
                return Err(LegacyElementAdoptError::ExpectedActor {
                    creation_order,
                    entity_id,
                });
            }
            if npc.is_some() && runtime.npc_data().is_none() {
                return Err(LegacyElementAdoptError::ExpectedNpc {
                    creation_order,
                    entity_id,
                });
            }
            records.push(ConvertedElement {
                entity_id,
                creation_order,
                element: convert_element(base, creation_order, entities, position_topology)?,
                actor: actor
                    .map(|actor| convert_actor(actor, creation_order, entities))
                    .transpose()?,
                npc: npc
                    .map(|npc| convert_npc(npc, creation_order, entities))
                    .transpose()?,
            });
        }
        Ok(Self { records })
    }

    /// Install a preflighted plan into a candidate engine.
    ///
    /// The lookups cannot fail when called on the candidate used for
    /// [`Self::preflight`].  Callers should clone the initialized engine,
    /// preflight all save sections against it, apply all plans, then swap the
    /// complete candidate into service.
    pub fn apply(self, engine: &mut EngineInner) {
        for converted in self.records {
            let entity = engine
                .world
                .entities
                .get_mut(converted.entity_id)
                .expect("preflighted v48 entity disappeared from candidate engine");
            let element = entity.element_data_mut();
            element.outline_colors = converted.element.outline_colors;
            element.current_outline = converted.element.current_outline;
            element.outline_width = converted.element.outline_width;
            element.custom_minimap_dot = converted.element.custom_minimap_dot;
            element.active = converted.element.active;
            element.in_honolulu = converted.element.in_honolulu;
            element.index_in_elements_list = converted.element.index_in_elements_list;
            element.blipped = converted.element.blipped;
            element.unreachable = converted.element.unreachable;
            let sprite = &mut element.sprite;
            sprite.current_row = converted.element.sprite.current_row;
            sprite.current_frame = converted.element.sprite.current_frame;
            sprite.frame_count = converted.element.sprite.frame_count;
            sprite.current_height = converted.element.sprite.current_height;
            sprite.current_width = converted.element.sprite.current_width;
            sprite.last_action = converted.element.sprite.last_action;
            sprite.use_alternate_profile = converted.element.sprite.alternate_profile;
            sprite.masked = converted.element.sprite.masked;
            sprite.behind_display_order_ref =
                converted.element.sprite.behind_display_order_reference;
            sprite.display_order_ref = converted.element.sprite.display_order_reference;
            sprite.action_done_frame = converted.element.sprite.action_done_frame;
            sprite.action_done_counter = converted.element.sprite.action_done_counter;
            sprite.last_sound_id = converted.element.sprite.last_sound_id;
            sprite.last_processed_order_id = converted.element.sprite.last_processed_order_id;
            (sprite.anims_to_be_replaced, sprite.replacing_anims) = converted
                .element
                .sprite
                .animation_replacements
                .into_iter()
                .unzip();
            sprite
                .position_iface
                .restore_v48_serialized_state(converted.element.sprite.position);

            if let Some(saved) = converted.actor {
                let actor = entity
                    .actor_data_mut()
                    .expect("preflighted v48 actor changed kind in candidate engine");
                actor.last_execute_order_id = saved.last_order_id;
                actor.old_action = saved.old_action;
                actor.action_state = saved.action_state;
                actor.execution_frozen = saved.execution_frozen;
                actor.is_ignored_for_anti_collision = saved.ignored_for_anti_collision;
                actor.execute_order_initialising = saved.new_order;
                actor.wait_time = saved.wait_time;
                actor.seek_target = saved.seek_target;
                actor.last_seek_target_position = saved.last_seek_target_position;
                actor.seek_distance = saved.seek_distance;
                actor.passing_door_directly = saved.passing_door_directly;
                actor.sequence_element_started = saved.sequence_element_started;
                actor.script_class = saved.script_class;
            }
            if let Some(saved) = converted.npc {
                apply_npc(
                    entity
                        .npc_data_mut()
                        .expect("preflighted v48 NPC changed kind in candidate engine"),
                    saved,
                );
            }
            debug_assert_eq!(
                engine
                    .world
                    .original_creation_order_by_entity
                    .get(&converted.entity_id),
                Some(&converted.creation_order)
            );
        }
    }
}

fn convert_element(
    saved: &LegacyElementPayloadBase,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
) -> Result<ConvertedElementBase, LegacyElementAdoptError> {
    if saved.position_map_delayed {
        return Err(LegacyElementAdoptError::UnsupportedDelayedPosition {
            creation_order,
            field: "mbPositionMapDelayed",
        });
    }
    if saved.position_delayed {
        return Err(LegacyElementAdoptError::UnsupportedDelayedPosition {
            creation_order,
            field: "mbPositionDelayed",
        });
    }
    Ok(ConvertedElementBase {
        outline_colors: saved.outline_colors,
        current_outline: outline(saved.current_outline, creation_order)?,
        outline_width: saved.outline_width,
        custom_minimap_dot: saved.custom_minimap_dot,
        active: saved.active,
        in_honolulu: saved.in_honolulu,
        index_in_elements_list: saved.index_in_elements_list,
        blipped: saved.blipped,
        unreachable: saved.unreachable,
        sprite: convert_sprite(&saved.sprite, creation_order, entities, topology)?,
    })
}

fn convert_sprite(
    saved: &LegacySpritePayload,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
) -> Result<ConvertedSprite, LegacyElementAdoptError> {
    let animation_replacements = saved
        .animation_replacements
        .iter()
        .map(|&(from, to)| {
            Ok((
                order_type(from, creation_order, "animation_replacements.from")?,
                order_type(to, creation_order, "animation_replacements.to")?,
            ))
        })
        .collect::<Result<Vec<_>, LegacyElementAdoptError>>()?;
    Ok(ConvertedSprite {
        current_row: saved.current_row,
        current_frame: saved.current_frame,
        frame_count: saved.frame_count,
        current_height: saved.current_height,
        current_width: saved.current_width,
        last_action: order_type(saved.last_action, creation_order, "last_action")?,
        alternate_profile: saved.alternate_profile,
        masked: saved.masked,
        behind_display_order_reference: saved.behind_display_order_reference,
        display_order_reference: entities.resolve_element(saved.display_order_reference)?,
        action_done_frame: saved.action_done_frame,
        action_done_counter: saved.action_done_counter,
        last_sound_id: saved.last_sound_id,
        last_processed_order_id: saved.last_processed_order_id,
        animation_replacements,
        position: preflight_v48_position(&saved.position, entities, topology)?,
    })
}

fn convert_actor(
    saved: &LegacyActorPayload,
    creation_order: u32,
    entities: &LegacyEntityFixups,
) -> Result<ConvertedActor, LegacyElementAdoptError> {
    let last_order_id = match saved.last_order_id {
        u32::MAX => None,
        0 => return Err(LegacyElementAdoptError::ZeroLastOrderId { creation_order }),
        value => NonZeroU32::new(value),
    };
    Ok(ConvertedActor {
        last_order_id,
        old_action: order_type(saved.old_action, creation_order, "old_action")?,
        action_state: ActionState::try_from(saved.action_state).map_err(|_| {
            LegacyElementAdoptError::UnknownEnum {
                creation_order,
                field: "action_state",
                value: saved.action_state,
            }
        })?,
        execution_frozen: saved.execution_frozen,
        ignored_for_anti_collision: saved.ignored_for_anti_collision,
        new_order: saved.new_order,
        wait_time: saved.wait_time,
        seek_target: entities.resolve_element(saved.seek_target)?,
        last_seek_target_position: MapPoint::new(
            saved.last_seek_target_position.x,
            saved.last_seek_target_position.y,
        ),
        seek_distance: saved.seek_distance,
        passing_door_directly: saved.passing_door_directly,
        sequence_element_started: saved.sequence_element_started,
        script_class: saved.script_class.clone(),
    })
}

fn convert_npc(
    saved: &LegacyNpcPayload,
    creation_order: u32,
    entities: &LegacyEntityFixups,
) -> Result<ConvertedNpc, LegacyElementAdoptError> {
    let mut detectable_lists = Vec::with_capacity(DetectableType::COUNT);
    let mut detection_suspects = [0; DetectableType::COUNT];
    for (index, bucket) in saved.detectable_buckets.iter().enumerate() {
        detection_suspects[index] = bucket.suspect;
        detectable_lists.push(
            bucket
                .entries
                .iter()
                .map(|detectable| {
                    Ok(Detectable {
                        element: entities.resolve_ai_element(detectable.element)?,
                        detectable_type: detectable_type(
                            detectable.detectable_type,
                            creation_order,
                            "detectable.type",
                        )?,
                        seen_last_frame: detectable.seen_last,
                        heard_last_frame: detectable.heard_last,
                        seen_now: detectable.seen_now,
                        // RHDetectable::Serialize omits this member. On load
                        // the just-constructed RHDetectable contributes false.
                        shadow_seen_now: false,
                        shadow_seen_last_frame: detectable.shadow_seen_last,
                        last_visibility: detectable.visibility,
                    })
                })
                .collect::<Result<Vec<_>, LegacyElementAdoptError>>()?,
        );
    }
    Ok(ConvertedNpc {
        life: saved.life,
        arrows: saved.arrows,
        old_direction: saved.old_direction,
        register: saved.register,
        attached_scroll: entities.resolve_element(saved.attached_scroll)?,
        inform: saved.inform,
        money: saved.money,
        wasp: saved.wasp,
        old_deafness: saved.old_deafness,
        old_frame: saved.old_frame,
        detectable_lists,
        detection_suspects,
        maximum_suspect: saved.maximum_suspect,
        worst_detectable_type: detectable_type(
            saved.worst_detectable_type,
            creation_order,
            "worst_detectable_type",
        )?,
        custom_values: saved.custom_values,
        gave_money: saved.gave_money,
    })
}

fn apply_npc(npc: &mut NpcData, saved: ConvertedNpc) {
    npc.life_points = saved.life;
    npc.number_of_arrows = saved.arrows;
    npc.direction_old = saved.old_direction;
    npc.register_number = saved.register;
    npc.scroll_attached = saved.attached_scroll.is_some();
    npc.inform_my_friends = saved.inform;
    npc.money = saved.money;
    npc.wasp_victim = saved.wasp;
    npc.old_cover_noise_deafness = saved.old_deafness;
    npc.old_cover_noise_deafness_frame_counter = saved.old_frame;
    npc.detectable_lists = saved.detectable_lists;
    npc.detection_suspects = saved.detection_suspects;
    npc.maximal_detection_suspect = saved.maximum_suspect;
    npc.worst_detected_type = saved.worst_detectable_type;
    npc.custom_values = saved.custom_values;
    npc.has_given_money_to_beggar = saved.gave_money;
}

fn payload_parts(
    payload: &LegacyElementPayload,
) -> (
    &LegacyElementPayloadBase,
    Option<&LegacyActorPayload>,
    Option<&LegacyNpcPayload>,
) {
    match payload {
        LegacyElementPayload::ActorPc(pc) => {
            let human = &pc.human;
            (&human.actor.element, Some(&human.actor), None)
        }
        LegacyElementPayload::ActorNpcSoldier(soldier) => {
            let npc = &soldier.npc;
            (&npc.human.actor.element, Some(&npc.human.actor), Some(npc))
        }
        LegacyElementPayload::ActorNpcCivilian(civilian) => {
            let npc = &civilian.npc;
            (&npc.human.actor.element, Some(&npc.human.actor), Some(npc))
        }
        LegacyElementPayload::ObjectItem(item) => (object_item_base(item), None, None),
        LegacyElementPayload::Bonus(bonus) => (&bonus.object.element, None, None),
        LegacyElementPayload::Scroll(scroll) => (&scroll.object.element, None, None),
        LegacyElementPayload::Target(target) => (&target.fx.element, None, None),
        LegacyElementPayload::Fx(fx) => (&fx.fx.element, None, None),
        LegacyElementPayload::FxMasked(fx) => (&fx.element, None, None),
    }
}

fn object_item_base(item: &LegacyObjectItemPayload) -> &LegacyElementPayloadBase {
    match item {
        LegacyObjectItemPayload::Object(value) => &value.element,
        LegacyObjectItemPayload::Arrow(value) => &value.projectile.object.element,
        LegacyObjectItemPayload::Apple(value) => &value.projectile.object.element,
        LegacyObjectItemPayload::Purse(value) => &value.projectile.object.element,
        LegacyObjectItemPayload::Stone(value) => &value.projectile.object.element,
        LegacyObjectItemPayload::WaspNest(value) => &value.projectile.object.element,
        LegacyObjectItemPayload::Wasp(value) => &value.object.element,
        LegacyObjectItemPayload::Net(value) => &value.projectile.object.element,
        LegacyObjectItemPayload::Coin(value) => &value.projectile.object.element,
        LegacyObjectItemPayload::Ale(value) => &value.object.element,
        LegacyObjectItemPayload::SpyCape(value) => &value.object.element,
        LegacyObjectItemPayload::Mobile(value) => &value.element,
    }
}

fn outline(value: u32, creation_order: u32) -> Result<OutlineColorName, LegacyElementAdoptError> {
    match value {
        0 => Ok(OutlineColorName::Default),
        1 => Ok(OutlineColorName::Target),
        2 => Ok(OutlineColorName::Hidden),
        3 => Ok(OutlineColorName::Striking),
        4 => Ok(OutlineColorName::Parrying),
        value => Err(LegacyElementAdoptError::UnknownEnum {
            creation_order,
            field: "current_outline",
            value,
        }),
    }
}

fn order_type(
    value: u32,
    creation_order: u32,
    field: &'static str,
) -> Result<OrderType, LegacyElementAdoptError> {
    OrderType::try_from(value).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value,
    })
}

fn detectable_type(
    value: u32,
    creation_order: u32,
    field: &'static str,
) -> Result<DetectableType, LegacyElementAdoptError> {
    // Original RHelementactornpc.h: NUMBER_OF_DETECTABLE_TYPES is the
    // non-value sentinel 6; DETECTABLE_NONE is 7.
    match value {
        0 => Ok(DetectableType::Enemy),
        1 => Ok(DetectableType::Body),
        2 => Ok(DetectableType::Object),
        3 => Ok(DetectableType::Friend),
        4 => Ok(DetectableType::MissedFriend),
        5 => Ok(DetectableType::Beggar),
        7 => Ok(DetectableType::None),
        value => Err(LegacyElementAdoptError::UnknownEnum {
            creation_order,
            field,
            value,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_detectable_none_skips_count_sentinel() {
        assert_eq!(
            detectable_type(7, 44, "worst_detectable_type").unwrap(),
            DetectableType::None
        );
        assert!(matches!(
            detectable_type(6, 44, "worst_detectable_type"),
            Err(LegacyElementAdoptError::UnknownEnum { value: 6, .. })
        ));
    }

    #[test]
    fn original_outline_values_map_without_reordering() {
        assert_eq!(outline(0, 31).unwrap(), OutlineColorName::Default);
        assert_eq!(outline(1, 31).unwrap(), OutlineColorName::Target);
        assert_eq!(outline(2, 31).unwrap(), OutlineColorName::Hidden);
        assert_eq!(outline(3, 31).unwrap(), OutlineColorName::Striking);
        assert_eq!(outline(4, 31).unwrap(), OutlineColorName::Parrying);
        assert!(outline(5, 31).is_err());
    }
}
