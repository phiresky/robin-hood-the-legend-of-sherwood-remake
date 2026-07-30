//! Atomic adoption of the common static-element state in Original v48 saves.
//!
//! This module deliberately builds an owned, fully validated plan before
//! touching the initialized mission.  It is not wired into the replay runner
//! yet: later stages must first adopt the remaining leaf, sequence, AI, and
//! manager state into the same candidate engine.

use std::num::NonZeroU32;

use thiserror::Error;

use crate::{
    ai::{
        AiController, AiState, AlertLevel, Attitude, CombatInfo, DoorCombatInfo, GotoFlags, Hint,
        Noise, NoiseType, PathHistoryEntry, PathId, PatrolPath, Position, ReconnaissanceReport,
        Remark, ReportType, Stimulus, StimulusInfo, StimulusType, StolenObject, Substate,
    },
    coordinates::{GroundPoint, MapPoint, MapVec},
    element::{
        ActionState, AiBrain, Detectable, DetectableType, EntityId, EyeStatus, NpcData,
        OutlineColorName,
    },
    engine::{EngineInner, LevelAssets},
    level_data::WaypointCommand,
    order::OrderType,
    position_interface::{PositionInterfaceV48State, SectorHandle},
};

use super::{
    adopt::{
        LegacyEntityFixups, LegacyPositionTopology, LegacySaveAdoptError, preflight_v48_position,
    },
    payload_ai::{
        LegacyAiPathStatus, LegacyAiPosition, LegacyLocalAiCommon, LegacyLocalAiTail,
        LegacyStimulus, LegacyStimulusInfo, LegacyStimulusPosition,
    },
    payload_base::{
        LegacyActorPayload, LegacyAiElementRef, LegacyElementPayloadBase, LegacyNpcPayload,
        LegacyNpcView, LegacySpritePayload,
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
    "RHElementActor seek layer, bypass/railroad flags, seek-to-point/check-jump, bypass exit/reference, and material/seek sectors",
    "RHElementActor sequence/order pointers and inline post-seek sequence",
    "RHElementActor script member variables",
    "RHElementActorNPC::mpAttachedScroll identity (Rust currently retains only attached/not attached)",
    "RHElementActorNPC::muwBodyVisitors",
    "RHElementActorNPC view iterator/crazy-cone/future-aperture/radius-reduction/sniper fields absent from Rust",
    "RHElementActorNPC local-AI most subclass state",
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
    "RHArtificialIntelligence::mlistAILog info/frame (Linux v48 serializes only the debug line type)",
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
    #[error(
        "saved element creation order {creation_order} field {field} references sector {index}, but initialized topology has {count} sector slots"
    )]
    MissingSector {
        creation_order: u32,
        field: &'static str,
        index: u16,
        count: usize,
    },
    #[error(
        "saved NPC creation order {creation_order} has {saved_kind} local AI but its initialized Rust entity has {runtime_kind}"
    )]
    AiKindMismatch {
        creation_order: u32,
        saved_kind: &'static str,
        runtime_kind: &'static str,
    },
    #[error(
        "saved NPC creation order {creation_order} local-AI owner resolves to {actual:?}; expected itself ({expected})"
    )]
    AiOwnerMismatch {
        creation_order: u32,
        expected: EntityId,
        actual: Option<EntityId>,
    },
    #[error(
        "saved element creation order {creation_order} field {field} has invalid bit flags 0x{value:x}"
    )]
    InvalidFlags {
        creation_order: u32,
        field: &'static str,
        value: u16,
    },
    #[error(
        "saved NPC creation order {creation_order} field {field} resolves to {entity_id} ({actual:?}); expected {expected}"
    )]
    WrongReferenceKind {
        creation_order: u32,
        field: &'static str,
        entity_id: EntityId,
        actual: crate::entity_id::EntityIdKind,
        expected: &'static str,
    },
    #[error(
        "saved NPC creation order {creation_order} field {field} has negative enum value {value}"
    )]
    NegativeEnum {
        creation_order: u32,
        field: &'static str,
        value: i32,
    },
    #[error(
        "saved NPC creation order {creation_order} has in-progress remark {remark}; Original completes it and dispatches AI event flags 0x{flags:x} while loading, which is not yet part of atomic adoption"
    )]
    UnsupportedRemarkCompletion {
        creation_order: u32,
        remark: u32,
        flags: u16,
    },
    #[error(
        "saved NPC creation order {creation_order} stimulus declares info type {declared}, but decoded payload is {actual}"
    )]
    StimulusInfoMismatch {
        creation_order: u32,
        declared: i32,
        actual: &'static str,
    },
    #[error(
        "saved NPC creation order {creation_order} path references hiking path {index}, but initialized assets have {count} paths"
    )]
    MissingHikingPath {
        creation_order: u32,
        index: u16,
        count: usize,
    },
    #[error(
        "saved NPC creation order {creation_order} hiking path {path} has {count} waypoints, which does not fit Original's u8 path state"
    )]
    TooManyWaypoints {
        creation_order: u32,
        path: u16,
        count: usize,
    },
    #[error(
        "saved NPC creation order {creation_order} path {path} field {field} references waypoint {waypoint}, but the path has {count} waypoints"
    )]
    MissingWaypoint {
        creation_order: u32,
        path: u16,
        field: &'static str,
        waypoint: u8,
        count: usize,
    },
    #[error(
        "saved NPC creation order {creation_order} has a patrol path but no initialized hiking path"
    )]
    PatrolPathWithoutHikingPath { creation_order: u32 },
    #[error(
        "saved NPC creation order {creation_order} macro cursor {offset} with {remaining} remaining bytes exceeds current waypoint macro length {length}"
    )]
    InvalidMacroCursor {
        creation_order: u32,
        offset: usize,
        remaining: u16,
        length: usize,
    },
    #[error(
        "saved NPC creation order {creation_order} has active macro state on a current waypoint whose command is {command}"
    )]
    MacroCommandKind {
        creation_order: u32,
        command: &'static str,
    },
    #[error(
        "saved NPC creation order {creation_order} has macro progress without a patrol-path-relative cursor"
    )]
    MacroWithoutPatrolPath { creation_order: u32 },
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
    view: ConvertedNpcView,
    initial_position_x: f32,
    initial_position_y: f32,
    initial_position_sector: Option<SectorHandle>,
    initial_position_level: u16,
    initial_view_direction: MapVec,
    local_ai: ConvertedLocalAi,
}

#[derive(Clone, Debug)]
struct ConvertedNpcView {
    eye_status: EyeStatus,
    transition: bool,
    alpha: u16,
    half_angle: f32,
    angle_step: f32,
    angle: f32,
    half_aperture: f32,
    real_half_aperture: f32,
    direction: [f32; 2],
    stare: GroundPoint,
    follow_target: Option<EntityId>,
    radius_goal: u16,
    radius: u16,
    radius_step: u16,
    long_range: f32,
    real_radius: u16,
    drunkenness: [f32; 4],
    leaning: bool,
}

#[derive(Clone, Debug)]
enum ConvertedLocalAi {
    Friendly {
        common: ConvertedLocalAiCommon,
        fleeing_seen_enemy_counter: u16,
        beggar_dont_talk_counter: u16,
    },
    Enemy {
        common: ConvertedLocalAiCommon,
        frame_when_enemy_detected: u32,
        fleeing_seen_enemy_counter: u16,
        pc_missed: bool,
        current_task_priority: u16,
        minimal_task_priority: u16,
        new_task_priority: u16,
        seen_dead_body: bool,
        seeking_charly: bool,
        reported_to_officer: bool,
        missed_soldier_timer: u16,
        old_money: u16,
        shield_bearer_direction: u16,
        phalanx_aborted: bool,
        changed_to_alert_path: bool,
        enemy_seen_below: bool,
        enemy_had_this_elevation: u16,
    },
}

#[derive(Clone, Copy)]
enum ReferenceKind {
    Human,
    Npc,
    Object,
}

impl ReferenceKind {
    fn accepts(self, kind: crate::entity_id::EntityIdKind) -> bool {
        use crate::entity_id::EntityIdKind;
        match self {
            Self::Human => matches!(
                kind,
                EntityIdKind::Pc | EntityIdKind::Soldier | EntityIdKind::Civilian
            ),
            Self::Npc => matches!(kind, EntityIdKind::Soldier | EntityIdKind::Civilian),
            Self::Object => matches!(
                kind,
                EntityIdKind::Bonus
                    | EntityIdKind::Scroll
                    | EntityIdKind::Projectile
                    | EntityIdKind::Net
            ),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Human => "PC, Soldier, or Civilian",
            Self::Npc => "Soldier or Civilian",
            Self::Object => "Bonus, Scroll, Projectile, or Net",
        }
    }
}

#[derive(Clone, Debug)]
struct ConvertedLocalAiCommon {
    last_goto_destination: Position,
    last_goto_flags: GotoFlags,
    stuck_counter: u16,
    forbidden_remark_ids: Vec<u32>,
    current_remark_flags: u16,
    current_state: AiState,
    old_state: AiState,
    current_substate: Substate,
    current_music_alert_status: AlertLevel,
    view_alert_status: AlertLevel,
    substate_at_last_timer_launch: Substate,
    attitude: Attitude,
    blood_alcohol: u8,
    initial_action: u32,
    number_of_looks: u8,
    can_move: bool,
    has_patrol_path: bool,
    patrol_path: Option<PatrolPath>,
    macro_command: Vec<u8>,
    macro_command_offset: usize,
    primary_target: u32,
    friend_in_trouble: u32,
    detected_body: u32,
    interesting_object: u32,
    antagonist: u32,
    last_stimulus_actor: Option<u32>,
    macro_in_progress: bool,
    number_of_remaining_macro_bytes: u16,
    timer_is_running: bool,
    when_does_timer_ring: u32,
    macro_timer_is_running: bool,
    when_does_macro_timer_ring: u32,
    standing_around_timer: u16,
    sorrow_level: u16,
    last_stimulus: [StimulusType; 5],
    last_stimulus_multiplicity: [u16; 5],
    is_master: bool,
    master: u32,
    first_try: bool,
    panic_center_x: f32,
    panic_center_y: f32,
    lasting_panic_runs: u8,
    directed_panic: bool,
    list_us: Vec<u32>,
    list_alerted_us: Vec<u32>,
    list_staying_us: Vec<u32>,
    couldnt_reachpoint: bool,
    already_on_point: bool,
    already_turned: bool,
    likes_to_sit_around: bool,
    special_action: bool,
    friends_are_alerted: bool,
    is_stay_at_home: bool,
    was_busy: bool,
    script_locked: bool,
    remember_events: bool,
    leave_house_number: u16,
    forgotten_objects: Vec<u32>,
    object_of_desire: u32,
    checkpoint_charly: u32,
    synchronize_charly: u32,
    inside_halt_method: bool,
    macro_started_in_this_frame: bool,
    synchronizing_actors: Vec<u32>,
    default_path_walking_flags: GotoFlags,
    current_remark: Remark,
    next_macro_rand: u8,
    next_macro_rand_forecasted: bool,
    knocked_out_in_money_fight: bool,
    got_beggar_trick: bool,
    reconnaissance: ReconnaissanceReport,
    patrol_chief: Option<EntityId>,
    patrol: Vec<EntityId>,
    missed_patrol_members: Vec<EntityId>,
    theoretical_patrol: Vec<EntityId>,
    patrol_stopped: bool,
    patrol_direction: u16,
    stimulus_queue: Vec<Stimulus>,
}

impl LegacyStaticElementAdoption {
    /// Validate and convert every element record without mutating `engine`.
    pub fn preflight(
        engine: &EngineInner,
        assets: &LevelAssets,
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
                    .map(|npc| {
                        convert_npc(
                            npc,
                            runtime
                                .npc_data()
                                .expect("NPC kind was validated immediately above"),
                            entity_id,
                            creation_order,
                            entities,
                            position_topology,
                            assets,
                        )
                    })
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
    runtime: &NpcData,
    entity_id: EntityId,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
    assets: &LevelAssets,
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
        view: convert_npc_view(
            &saved.view,
            entities.resolve_element(saved.mobile_target)?,
            creation_order,
        )?,
        initial_position_x: saved.initial_position.x,
        initial_position_y: saved.initial_position.y,
        initial_position_sector: sector(
            saved.initial_position.sector.0,
            topology,
            creation_order,
            "initial_position.sector",
        )?,
        initial_position_level: saved.initial_position.level,
        initial_view_direction: MapVec::new(saved.initial_view.x, saved.initial_view.y),
        local_ai: convert_local_ai(
            &saved.local_ai,
            runtime,
            entity_id,
            creation_order,
            entities,
            topology,
            assets,
            alert_level(saved.view.alert_status, creation_order, "view.alert_status")?,
        )?,
    })
}

fn apply_npc(npc: &mut NpcData, saved: ConvertedNpc) {
    let ai_initial_position = Position {
        x: saved.initial_position_x,
        y: saved.initial_position_y,
        sector: saved.initial_position_sector,
        level: saved.initial_position_level,
    };
    let ai_initial_view_direction = crate::position_interface::vector_to_sector_0_to_15(
        saved.initial_view_direction.x * crate::position_interface::ASPECT_RATIO,
        saved.initial_view_direction.y,
    ) as u16;
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
    npc.initial_position_x = saved.initial_position_x;
    npc.initial_position_y = saved.initial_position_y;
    npc.initial_position_sector = saved.initial_position_sector;
    npc.initial_position_level = saved.initial_position_level;
    npc.initial_view_direction = saved.initial_view_direction;
    npc.eye_status = saved.view.eye_status;
    npc.view_transition = saved.view.transition;
    npc.view_alpha_start = saved.view.alpha;
    npc.view_half_angle_range = saved.view.half_angle;
    npc.view_angle_step = saved.view.angle_step;
    npc.view_angle = saved.view.angle;
    npc.half_aperture = saved.view.half_aperture;
    npc.real_half_aperture = saved.view.real_half_aperture;
    npc.view_direction = saved.view.direction;
    npc.stare_point = saved.view.stare;
    npc.follow_target = saved.view.follow_target;
    npc.view_radius_goal = saved.view.radius_goal;
    npc.view_radius_base = saved.view.radius;
    npc.view_radius_step = saved.view.radius_step;
    npc.view_longrange_radius_factor = saved.view.long_range;
    npc.view_radius = saved.view.real_radius;
    npc.drunken_cone_iterators = saved.view.drunkenness;
    npc.view_lean_out = saved.view.leaning;
    apply_local_ai(&mut npc.ai_brain, saved.local_ai);
    let ai = ai_base_mut(&mut npc.ai_brain)
        .expect("preflighted local-AI kind cannot become None in candidate engine");
    ai.initial_position = ai_initial_position;
    ai.initial_view_direction = ai_initial_view_direction;
}

fn convert_npc_view(
    saved: &LegacyNpcView,
    follow_target: Option<EntityId>,
    creation_order: u32,
) -> Result<ConvertedNpcView, LegacyElementAdoptError> {
    Ok(ConvertedNpcView {
        eye_status: eye_status(saved.status, creation_order)?,
        transition: saved.transitioning,
        alpha: saved.alpha,
        half_angle: saved.half_angle,
        angle_step: saved.angle_step,
        angle: saved.angle,
        half_aperture: saved.half_aperture,
        real_half_aperture: saved.real_half_aperture,
        direction: [saved.direction.x, saved.direction.y],
        stare: GroundPoint::new(saved.stare.x, saved.stare.y),
        // The raw pointer echo inside LegacyNpcView is ABI residue;
        // LegacyNpcPayload::mobile_target is the authoritative pointer fixup.
        follow_target,
        radius_goal: saved.radius_goal,
        radius: saved.radius,
        radius_step: saved.radius_step,
        long_range: saved.long_range,
        real_radius: saved.real_radius,
        drunkenness: saved.drunkenness,
        leaning: saved.leaning,
    })
}

fn convert_local_ai(
    saved: &super::payload_ai::LegacyLocalAiPayload,
    runtime: &NpcData,
    entity_id: EntityId,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
    assets: &LevelAssets,
    view_alert_status: AlertLevel,
) -> Result<ConvertedLocalAi, LegacyElementAdoptError> {
    let owner = entities.resolve_ai_element(saved.common.owner)?;
    if owner != Some(entity_id) {
        return Err(LegacyElementAdoptError::AiOwnerMismatch {
            creation_order,
            expected: entity_id,
            actual: owner,
        });
    }
    let common = convert_local_ai_common(
        &saved.common,
        creation_order,
        entities,
        topology,
        view_alert_status,
        assets,
    )?;
    match (&saved.tail, &runtime.ai_brain) {
        (LegacyLocalAiTail::Friendly(tail), AiBrain::Friendly(_)) => {
            Ok(ConvertedLocalAi::Friendly {
                common,
                fleeing_seen_enemy_counter: tail.fleeing_seen_enemy_counter,
                beggar_dont_talk_counter: tail.beggar_dont_talk_counter,
            })
        }
        (LegacyLocalAiTail::Enemy(tail), AiBrain::Enemy(_)) => Ok(ConvertedLocalAi::Enemy {
            common,
            frame_when_enemy_detected: tail.frame_when_enemy_detected,
            fleeing_seen_enemy_counter: tail.fleeing_seen_enemy_counter,
            pc_missed: tail.pc_missed,
            current_task_priority: tail.current_task_priority,
            minimal_task_priority: tail.minimal_task_priority,
            new_task_priority: tail.new_task_priority,
            seen_dead_body: tail.seen_dead_body,
            seeking_charly: tail.seeking_charly,
            reported_to_officer: tail.reported_to_officer,
            missed_soldier_timer: tail.missed_soldier_timer,
            old_money: tail.old_money,
            shield_bearer_direction: tail.shield_bearer_direction,
            phalanx_aborted: tail.phalanx_aborted,
            changed_to_alert_path: tail.changed_to_alert_path,
            enemy_seen_below: tail.enemy_seen_below,
            enemy_had_this_elevation: tail.enemy_had_this_elevation,
        }),
        (LegacyLocalAiTail::Friendly(_), brain) => Err(LegacyElementAdoptError::AiKindMismatch {
            creation_order,
            saved_kind: "Friendly",
            runtime_kind: ai_brain_kind(brain),
        }),
        (LegacyLocalAiTail::Enemy(_), brain) => Err(LegacyElementAdoptError::AiKindMismatch {
            creation_order,
            saved_kind: "Enemy",
            runtime_kind: ai_brain_kind(brain),
        }),
    }
}

fn convert_local_ai_common(
    saved: &LegacyLocalAiCommon,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
    view_alert_status: AlertLevel,
    assets: &LevelAssets,
) -> Result<ConvertedLocalAiCommon, LegacyElementAdoptError> {
    let (macro_command, macro_command_offset) =
        convert_macro_command(saved, creation_order, &assets.hiking_paths)?;
    Ok(ConvertedLocalAiCommon {
        last_goto_destination: ai_position(
            saved.last_goto_destination,
            topology,
            creation_order,
            "local_ai.last_goto_destination.sector",
        )?,
        last_goto_flags: goto_flags(
            saved.last_goto_flags,
            creation_order,
            "local_ai.last_goto_flags",
        )?,
        stuck_counter: saved.stuck_counter,
        forbidden_remark_ids: saved
            .forbidden_remarks
            .iter()
            .map(|&value| {
                remark(value, creation_order, "local_ai.forbidden_remarks")
                    .map(|value| value as u32)
            })
            .collect::<Result<Vec<_>, _>>()?,
        current_remark_flags: saved.current_remark_flags,
        current_state: ai_state(
            saved.current_state,
            creation_order,
            "local_ai.current_state",
        )?,
        old_state: ai_state(saved.old_state, creation_order, "local_ai.old_state")?,
        current_substate: substate(
            saved.current_substate,
            creation_order,
            "local_ai.current_substate",
        )?,
        current_music_alert_status: alert_level(
            saved.current_music_alert_status as u32,
            creation_order,
            "local_ai.current_music_alert_status",
        )?,
        view_alert_status,
        substate_at_last_timer_launch: substate(
            saved.substate_at_last_timer_launch,
            creation_order,
            "local_ai.substate_at_last_timer_launch",
        )?,
        attitude: attitude(saved.attitude, creation_order, "local_ai.attitude")?,
        blood_alcohol: saved.blood_alcohol,
        initial_action: u32::try_from(saved.initial_action).map_err(|_| {
            LegacyElementAdoptError::UnknownEnum {
                creation_order,
                field: "local_ai.initial_action",
                value: saved.initial_action as u32,
            }
        })?,
        number_of_looks: saved.number_of_looks,
        can_move: saved.can_move,
        has_patrol_path: saved.has_patrol_path,
        patrol_path: convert_patrol_path(
            &saved.path,
            creation_order,
            topology,
            &assets.hiking_paths,
        )?,
        macro_command,
        macro_command_offset,
        primary_target: ai_handle(
            entities.resolve_ai_element(saved.primary_target)?,
            ReferenceKind::Human,
            creation_order,
            "local_ai.primary_target",
        )?,
        friend_in_trouble: ai_handle(
            entities.resolve_ai_element(saved.friend_in_trouble)?,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.friend_in_trouble",
        )?,
        detected_body: ai_handle(
            entities.resolve_ai_element(saved.detected_body)?,
            ReferenceKind::Human,
            creation_order,
            "local_ai.detected_body",
        )?,
        interesting_object: ai_handle(
            entities.resolve_ai_element(saved.interesting_object)?,
            ReferenceKind::Object,
            creation_order,
            "local_ai.interesting_object",
        )?,
        antagonist: ai_handle(
            entities.resolve_ai_element(saved.antagonist)?,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.antagonist",
        )?,
        last_stimulus_actor: ai_optional_handle(
            entities.resolve_ai_element(saved.last_stimulus_actor)?,
            ReferenceKind::Human,
            creation_order,
            "local_ai.last_stimulus_actor",
        )?,
        macro_in_progress: saved.macro_in_progress,
        number_of_remaining_macro_bytes: saved.remaining_macro_bytes,
        timer_is_running: saved.timer_is_running,
        when_does_timer_ring: saved.timer_ring_frame,
        macro_timer_is_running: saved.macro_timer_is_running,
        when_does_macro_timer_ring: saved.macro_timer_ring_frame,
        standing_around_timer: saved.standing_around_timer,
        sorrow_level: saved.sorrow_level,
        last_stimulus: saved
            .last_stimuli
            .map(|value| stimulus_type(value, creation_order, "local_ai.last_stimulus"))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .expect("fixed-size stimulus conversion preserves length"),
        last_stimulus_multiplicity: saved.last_stimulus_multiplicities,
        is_master: saved.is_master,
        master: ai_handle(
            entities.resolve_ai_element(saved.master)?,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.master",
        )?,
        first_try: saved.first_try,
        panic_center_x: saved.panic_center_x,
        panic_center_y: saved.panic_center_y,
        lasting_panic_runs: saved.lasting_panic_runs,
        directed_panic: saved.directed_panic,
        list_us: ai_handle_list(
            &saved.us,
            ReferenceKind::Human,
            creation_order,
            "local_ai.us",
            entities,
        )?,
        list_alerted_us: ai_handle_list(
            &saved.alerted_us,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.alerted_us",
            entities,
        )?,
        list_staying_us: ai_handle_list(
            &saved.staying_us,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.staying_us",
            entities,
        )?,
        couldnt_reachpoint: saved.could_not_reach_point,
        already_on_point: saved.already_on_point,
        already_turned: saved.already_turned,
        likes_to_sit_around: saved.likes_to_sit_around,
        special_action: saved.special_action,
        friends_are_alerted: saved.friends_are_alerted,
        is_stay_at_home: saved.stay_at_home,
        was_busy: saved.was_busy,
        script_locked: saved.script_locked,
        remember_events: saved.remember_events,
        leave_house_number: saved.leave_house_number,
        forgotten_objects: ai_handle_list(
            &saved.forgotten_objects,
            ReferenceKind::Object,
            creation_order,
            "local_ai.forgotten_objects",
            entities,
        )?,
        object_of_desire: element_handle(
            entities.resolve_element(saved.object_of_desire)?,
            ReferenceKind::Object,
            creation_order,
            "local_ai.object_of_desire",
        )?,
        checkpoint_charly: element_handle(
            entities.resolve_element(saved.checkpoint_charly)?,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.checkpoint_charly",
        )?,
        synchronize_charly: element_handle(
            entities.resolve_element(saved.synchronize_charly)?,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.synchronize_charly",
        )?,
        inside_halt_method: saved.inside_halt_method,
        macro_started_in_this_frame: saved.macro_started_this_frame,
        synchronizing_actors: ai_handle_list(
            &saved.synchronizing_actors,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.synchronizing_actors",
            entities,
        )?,
        default_path_walking_flags: goto_flags(
            saved.default_path_walking_flags,
            creation_order,
            "local_ai.default_path_walking_flags",
        )?,
        current_remark: {
            let current = remark(
                saved.current_remark,
                creation_order,
                "local_ai.current_remark",
            )?;
            if current != Remark::TheSoundOfSilence {
                return Err(LegacyElementAdoptError::UnsupportedRemarkCompletion {
                    creation_order,
                    remark: current as u32,
                    flags: saved.current_remark_flags,
                });
            }
            current
        },
        next_macro_rand: saved.next_macro_rand,
        next_macro_rand_forecasted: saved.next_macro_rand_forecasted,
        knocked_out_in_money_fight: saved.knocked_out_in_money_fight,
        got_beggar_trick: saved.got_beggar_trick,
        reconnaissance: reconnaissance(&saved.reconnaissance, creation_order, entities, topology)?,
        patrol_chief: optional_element_entity(
            entities.resolve_element(saved.patrol_chief)?,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.patrol_chief",
        )?,
        patrol: ai_entity_list(
            &saved.patrol,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.patrol",
            entities,
        )?,
        missed_patrol_members: ai_entity_list(
            &saved.missed_patrol_members,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.missed_patrol_members",
            entities,
        )?,
        theoretical_patrol: ai_entity_list(
            &saved.theoretical_patrol,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.theoretical_patrol",
            entities,
        )?,
        patrol_stopped: saved.patrol_stopped,
        patrol_direction: saved.patrol_direction,
        stimulus_queue: saved
            .stimulus_queue
            .iter()
            .map(|stimulus| convert_stimulus(stimulus, creation_order, entities, topology))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn apply_local_ai(brain: &mut AiBrain, saved: ConvertedLocalAi) {
    match (brain, saved) {
        (
            AiBrain::Friendly(ai),
            ConvertedLocalAi::Friendly {
                common,
                fleeing_seen_enemy_counter,
                beggar_dont_talk_counter,
            },
        ) => {
            apply_local_ai_common(&mut ai.base, common);
            ai.fleeing_seen_enemy_counter = fleeing_seen_enemy_counter;
            ai.beggar_dont_talk_counter = beggar_dont_talk_counter;
        }
        (
            AiBrain::Enemy(ai),
            ConvertedLocalAi::Enemy {
                common,
                frame_when_enemy_detected,
                fleeing_seen_enemy_counter,
                pc_missed,
                current_task_priority,
                minimal_task_priority,
                new_task_priority,
                seen_dead_body,
                seeking_charly,
                reported_to_officer,
                missed_soldier_timer,
                old_money,
                shield_bearer_direction,
                phalanx_aborted,
                changed_to_alert_path,
                enemy_seen_below,
                enemy_had_this_elevation,
            },
        ) => {
            apply_local_ai_common(&mut ai.base, common);
            ai.base.frame_when_enemy_detected = frame_when_enemy_detected;
            ai.fleeing_seen_enemy_counter = fleeing_seen_enemy_counter;
            ai.pc_missed = pc_missed;
            ai.current_task_priority = current_task_priority;
            ai.minimal_task_priority = minimal_task_priority;
            ai.new_task_priority = new_task_priority;
            ai.seen_dead_body = seen_dead_body;
            ai.seeking_charly = seeking_charly;
            ai.reported_to_officer = reported_to_officer;
            ai.missed_soldier_timer = missed_soldier_timer;
            ai.old_money = old_money;
            ai.shield_bearer_direction = shield_bearer_direction;
            ai.phalanx_aborted = phalanx_aborted;
            ai.changed_to_alert_path = changed_to_alert_path;
            ai.enemy_seen_below = enemy_seen_below;
            ai.enemy_had_this_elevation = enemy_had_this_elevation;
        }
        _ => unreachable!("preflighted local-AI kind changed in candidate engine"),
    }
}

fn apply_local_ai_common(ai: &mut AiController, saved: ConvertedLocalAiCommon) {
    ai.last_goto_destination = saved.last_goto_destination;
    ai.last_goto_flags = saved.last_goto_flags;
    ai.stuck_counter = saved.stuck_counter;
    ai.forbidden_remark_ids = saved.forbidden_remark_ids;
    ai.current_remark_flags = saved.current_remark_flags;
    ai.current_state = saved.current_state;
    ai.old_state = saved.old_state;
    ai.current_substate = saved.current_substate;
    ai.current_music_alert_status = saved.current_music_alert_status;
    ai.view_alert_status = saved.view_alert_status;
    ai.substate_at_last_timer_launch = saved.substate_at_last_timer_launch;
    ai.attitude = saved.attitude;
    ai.blood_alcohol = saved.blood_alcohol;
    ai.initial_action = saved.initial_action;
    ai.number_of_looks = saved.number_of_looks;
    ai.can_move = saved.can_move;
    ai.has_patrol_path = saved.has_patrol_path;
    ai.patrol_path = saved.patrol_path;
    ai.macro_command = saved.macro_command;
    ai.macro_command_offset = saved.macro_command_offset;
    ai.primary_target = saved.primary_target;
    ai.friend_in_trouble = saved.friend_in_trouble;
    ai.detected_body = saved.detected_body;
    ai.interesting_object = saved.interesting_object;
    ai.antagonist = saved.antagonist;
    ai.last_stimulus_actor = saved.last_stimulus_actor;
    ai.macro_in_progress = saved.macro_in_progress;
    ai.number_of_remaining_macro_bytes = saved.number_of_remaining_macro_bytes;
    ai.timer_is_running = saved.timer_is_running;
    ai.when_does_timer_ring = saved.when_does_timer_ring;
    ai.macro_timer_is_running = saved.macro_timer_is_running;
    ai.when_does_macro_timer_ring = saved.when_does_macro_timer_ring;
    ai.standing_around_timer = saved.standing_around_timer;
    ai.sorrow_level = saved.sorrow_level;
    ai.last_stimulus = saved.last_stimulus;
    ai.last_stimulus_multiplicity = saved.last_stimulus_multiplicity;
    ai.is_master = saved.is_master;
    ai.master = saved.master;
    ai.first_try = saved.first_try;
    ai.panic_center_x = saved.panic_center_x;
    ai.panic_center_y = saved.panic_center_y;
    ai.lasting_panic_runs = saved.lasting_panic_runs;
    ai.directed_panic = saved.directed_panic;
    ai.list_us = saved.list_us;
    ai.list_alerted_us = saved.list_alerted_us;
    ai.list_staying_us = saved.list_staying_us;
    ai.couldnt_reachpoint = saved.couldnt_reachpoint;
    ai.already_on_point = saved.already_on_point;
    ai.already_turned = saved.already_turned;
    ai.likes_to_sit_around = saved.likes_to_sit_around;
    ai.special_action = saved.special_action;
    ai.friends_are_alerted = saved.friends_are_alerted;
    ai.is_stay_at_home = saved.is_stay_at_home;
    ai.was_busy = saved.was_busy;
    ai.script_locked = saved.script_locked;
    ai.remember_events = saved.remember_events;
    ai.leave_house_number = saved.leave_house_number;
    ai.forgotten_objects = saved.forgotten_objects;
    ai.object_of_desire = saved.object_of_desire;
    ai.checkpoint_charly = saved.checkpoint_charly;
    ai.synchronize_charly = saved.synchronize_charly;
    ai.inside_halt_method = saved.inside_halt_method;
    ai.macro_started_in_this_frame = saved.macro_started_in_this_frame;
    ai.synchronizing_actors = saved.synchronizing_actors;
    ai.default_path_walking_flags = saved.default_path_walking_flags;
    ai.current_remark = saved.current_remark;
    ai.next_macro_rand = saved.next_macro_rand;
    ai.next_macro_rand_forecasted = saved.next_macro_rand_forecasted;
    ai.knocked_out_in_money_fight = saved.knocked_out_in_money_fight;
    ai.got_the_beggar_trick = saved.got_beggar_trick;
    ai.my_reconnaissance_report = saved.reconnaissance;
    ai.patrol_chief = saved.patrol_chief;
    ai.patrol = saved.patrol;
    ai.missed_patrol_members = saved.missed_patrol_members;
    ai.theoretical_patrol = saved.theoretical_patrol;
    ai.patrol_stopped = saved.patrol_stopped;
    ai.patrol_direction = saved.patrol_direction;
    ai.stimulus_queue = saved.stimulus_queue;
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

fn eye_status(value: u8, creation_order: u32) -> Result<EyeStatus, LegacyElementAdoptError> {
    match value {
        0 => Ok(EyeStatus::Closed),
        1 => Ok(EyeStatus::LookForward),
        2 => Ok(EyeStatus::LookToTheLeft),
        3 => Ok(EyeStatus::LookToTheRight),
        4 => Ok(EyeStatus::LookDownwards),
        5 => Ok(EyeStatus::DieOrGetUnconscious),
        6 => Ok(EyeStatus::Follow),
        7 => Ok(EyeStatus::Stare),
        8 => Ok(EyeStatus::ViewconeGrow),
        value => Err(LegacyElementAdoptError::UnknownEnum {
            creation_order,
            field: "view.status",
            value: u32::from(value),
        }),
    }
}

fn alert_level(
    value: u32,
    creation_order: u32,
    field: &'static str,
) -> Result<AlertLevel, LegacyElementAdoptError> {
    AlertLevel::try_from(value).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value,
    })
}

fn ai_state(
    value: i32,
    creation_order: u32,
    field: &'static str,
) -> Result<AiState, LegacyElementAdoptError> {
    let raw = u32::try_from(value).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value: value as u32,
    })?;
    AiState::try_from(raw).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value: raw,
    })
}

fn substate(
    value: i32,
    creation_order: u32,
    field: &'static str,
) -> Result<Substate, LegacyElementAdoptError> {
    let raw = u32::try_from(value).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value: value as u32,
    })?;
    Substate::try_from(raw).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value: raw,
    })
}

fn attitude(
    value: i32,
    creation_order: u32,
    field: &'static str,
) -> Result<Attitude, LegacyElementAdoptError> {
    let raw = u32::try_from(value).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value: value as u32,
    })?;
    Attitude::try_from(raw).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value: raw,
    })
}

fn goto_flags(
    value: u16,
    creation_order: u32,
    field: &'static str,
) -> Result<GotoFlags, LegacyElementAdoptError> {
    GotoFlags::from_bits(value).ok_or(LegacyElementAdoptError::InvalidFlags {
        creation_order,
        field,
        value,
    })
}

fn sector(
    value: Option<u16>,
    topology: &LegacyPositionTopology,
    creation_order: u32,
    field: &'static str,
) -> Result<Option<SectorHandle>, LegacyElementAdoptError> {
    value
        .map(|index| {
            if usize::from(index) >= topology.sector_count {
                return Err(LegacyElementAdoptError::MissingSector {
                    creation_order,
                    field,
                    index,
                    count: topology.sector_count,
                });
            }
            SectorHandle::new(index).ok_or(LegacyElementAdoptError::MissingSector {
                creation_order,
                field,
                index,
                count: topology.sector_count,
            })
        })
        .transpose()
}

fn ai_position(
    saved: LegacyAiPosition,
    topology: &LegacyPositionTopology,
    creation_order: u32,
    field: &'static str,
) -> Result<Position, LegacyElementAdoptError> {
    Ok(Position {
        x: saved.x,
        y: saved.y,
        sector: sector(saved.sector.0, topology, creation_order, field)?,
        level: saved.level,
    })
}

fn convert_patrol_path(
    saved: &LegacyAiPathStatus,
    creation_order: u32,
    topology: &LegacyPositionTopology,
    hiking_paths: &[crate::level_data::RawHikingPath],
) -> Result<Option<PatrolPath>, LegacyElementAdoptError> {
    let Some(raw_path_id) = saved.hiking_path_index else {
        return Ok(None);
    };
    let path_id = PathId::new(raw_path_id)
        .expect("Legacy decoder already maps the 0xffff no-path sentinel to None");
    let authored = hiking_paths.get(usize::from(path_id)).ok_or(
        LegacyElementAdoptError::MissingHikingPath {
            creation_order,
            index: raw_path_id,
            count: hiking_paths.len(),
        },
    )?;
    let size = u8::try_from(authored.waypoints.len()).map_err(|_| {
        LegacyElementAdoptError::TooManyWaypoints {
            creation_order,
            path: raw_path_id,
            count: authored.waypoints.len(),
        }
    })?;
    for (field, waypoint) in [
        ("current_waypoint_index", saved.current_waypoint_index),
        ("last_waypoint_index", saved.last_waypoint_index),
    ] {
        if usize::from(waypoint) >= authored.waypoints.len() {
            return Err(LegacyElementAdoptError::MissingWaypoint {
                creation_order,
                path: raw_path_id,
                field,
                waypoint,
                count: authored.waypoints.len(),
            });
        }
    }
    let history = saved
        .history
        .iter()
        .map(|entry| {
            Ok(PathHistoryEntry {
                position: Position {
                    x: entry.position_x,
                    y: entry.position_y,
                    sector: sector(
                        entry.sector.0,
                        topology,
                        creation_order,
                        "local_ai.path.history.sector",
                    )?,
                    level: entry.level,
                },
                direction: entry.direction,
                distance: entry.distance,
            })
        })
        .collect::<Result<_, LegacyElementAdoptError>>()?;
    Ok(Some(PatrolPath {
        hiking_path_index: path_id,
        current_waypoint_index: saved.current_waypoint_index,
        last_waypoint_index: saved.last_waypoint_index,
        forward: saved.forward_movement,
        size,
        history,
    }))
}

fn convert_macro_command(
    saved: &LegacyLocalAiCommon,
    creation_order: u32,
    hiking_paths: &[crate::level_data::RawHikingPath],
) -> Result<(Vec<u8>, usize), LegacyElementAdoptError> {
    if !saved.has_patrol_path {
        if saved.macro_in_progress || saved.remaining_macro_bytes != 0 {
            return Err(LegacyElementAdoptError::MacroWithoutPatrolPath { creation_order });
        }
        return Ok((Vec::new(), 0));
    }
    let raw_path_id = saved
        .path
        .hiking_path_index
        .ok_or(LegacyElementAdoptError::PatrolPathWithoutHikingPath { creation_order })?;
    let authored = hiking_paths.get(usize::from(raw_path_id)).ok_or(
        LegacyElementAdoptError::MissingHikingPath {
            creation_order,
            index: raw_path_id,
            count: hiking_paths.len(),
        },
    )?;
    let waypoint = authored
        .waypoints
        .get(usize::from(saved.path.current_waypoint_index))
        .ok_or(LegacyElementAdoptError::MissingWaypoint {
            creation_order,
            path: raw_path_id,
            field: "current_waypoint_index",
            waypoint: saved.path.current_waypoint_index,
            count: authored.waypoints.len(),
        })?;
    let offset = usize::from(
        saved
            .macro_command_offset
            .expect("decoder supplies the conditional cursor when has_patrol_path is true"),
    );
    let macro_data = match &waypoint.command {
        WaypointCommand::Macro(data) => data,
        WaypointCommand::None
            if offset == 0 && saved.remaining_macro_bytes == 0 && !saved.macro_in_progress =>
        {
            return Ok((Vec::new(), 0));
        }
        WaypointCommand::Script(_)
            if offset == 0 && saved.remaining_macro_bytes == 0 && !saved.macro_in_progress =>
        {
            return Ok((Vec::new(), 0));
        }
        WaypointCommand::None => {
            return Err(LegacyElementAdoptError::MacroCommandKind {
                creation_order,
                command: "none",
            });
        }
        WaypointCommand::Script(_) => {
            return Err(LegacyElementAdoptError::MacroCommandKind {
                creation_order,
                command: "script",
            });
        }
    };
    let end = offset
        .checked_add(usize::from(saved.remaining_macro_bytes))
        .filter(|&end| offset <= macro_data.len() && end <= macro_data.len())
        .ok_or(LegacyElementAdoptError::InvalidMacroCursor {
            creation_order,
            offset,
            remaining: saved.remaining_macro_bytes,
            length: macro_data.len(),
        })?;
    let _ = end;
    Ok((macro_data.clone(), offset))
}

fn stimulus_position(
    saved: LegacyStimulusPosition,
    topology: &LegacyPositionTopology,
    creation_order: u32,
    field: &'static str,
) -> Result<Position, LegacyElementAdoptError> {
    Ok(Position {
        x: saved.x,
        y: saved.y,
        sector: sector(saved.sector.0, topology, creation_order, field)?,
        level: saved.level,
    })
}

fn raw_i32(
    value: i32,
    creation_order: u32,
    field: &'static str,
) -> Result<u32, LegacyElementAdoptError> {
    u32::try_from(value).map_err(|_| LegacyElementAdoptError::NegativeEnum {
        creation_order,
        field,
        value,
    })
}

fn stimulus_type(
    value: i32,
    creation_order: u32,
    field: &'static str,
) -> Result<StimulusType, LegacyElementAdoptError> {
    let value = raw_i32(value, creation_order, field)?;
    StimulusType::try_from(value).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value,
    })
}

fn remark(
    value: i32,
    creation_order: u32,
    field: &'static str,
) -> Result<Remark, LegacyElementAdoptError> {
    let value = raw_i32(value, creation_order, field)?;
    Remark::try_from(value).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value,
    })
}

fn noise_type(
    value: i32,
    creation_order: u32,
    field: &'static str,
) -> Result<NoiseType, LegacyElementAdoptError> {
    let value = raw_i32(value, creation_order, field)?;
    NoiseType::try_from(value).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value,
    })
}

fn checked_reference(
    entity_id: Option<EntityId>,
    expected: ReferenceKind,
    creation_order: u32,
    field: &'static str,
) -> Result<Option<EntityId>, LegacyElementAdoptError> {
    if let Some(entity_id) = entity_id
        && !expected.accepts(entity_id.kind())
    {
        return Err(LegacyElementAdoptError::WrongReferenceKind {
            creation_order,
            field,
            entity_id,
            actual: entity_id.kind(),
            expected: expected.name(),
        });
    }
    Ok(entity_id)
}

fn ai_handle(
    entity_id: Option<EntityId>,
    expected: ReferenceKind,
    creation_order: u32,
    field: &'static str,
) -> Result<u32, LegacyElementAdoptError> {
    Ok(checked_reference(entity_id, expected, creation_order, field)?.map_or(0, EntityId::index))
}

fn ai_optional_handle(
    entity_id: Option<EntityId>,
    expected: ReferenceKind,
    creation_order: u32,
    field: &'static str,
) -> Result<Option<u32>, LegacyElementAdoptError> {
    Ok(checked_reference(entity_id, expected, creation_order, field)?.map(EntityId::index))
}

fn element_handle(
    entity_id: Option<EntityId>,
    expected: ReferenceKind,
    creation_order: u32,
    field: &'static str,
) -> Result<u32, LegacyElementAdoptError> {
    ai_handle(entity_id, expected, creation_order, field)
}

fn optional_element_entity(
    entity_id: Option<EntityId>,
    expected: ReferenceKind,
    creation_order: u32,
    field: &'static str,
) -> Result<Option<EntityId>, LegacyElementAdoptError> {
    checked_reference(entity_id, expected, creation_order, field)
}

fn ai_handle_list(
    references: &[LegacyAiElementRef],
    expected: ReferenceKind,
    creation_order: u32,
    field: &'static str,
    entities: &LegacyEntityFixups,
) -> Result<Vec<u32>, LegacyElementAdoptError> {
    references
        .iter()
        .copied()
        .map(|reference| {
            ai_handle(
                entities.resolve_ai_element(reference)?,
                expected,
                creation_order,
                field,
            )
        })
        .collect()
}

fn ai_entity_list(
    references: &[LegacyAiElementRef],
    expected: ReferenceKind,
    creation_order: u32,
    field: &'static str,
    entities: &LegacyEntityFixups,
) -> Result<Vec<EntityId>, LegacyElementAdoptError> {
    references
        .iter()
        .copied()
        .map(|reference| {
            checked_reference(
                entities.resolve_ai_element(reference)?,
                expected,
                creation_order,
                field,
            )?
            .ok_or(LegacySaveAdoptError::MissingCreationOrderReference { creation_order }.into())
        })
        .collect()
}

fn reconnaissance(
    saved: &super::payload_ai::LegacyReconnaissanceReport,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
) -> Result<ReconnaissanceReport, LegacyElementAdoptError> {
    let report_type = match raw_i32(
        saved.report_type,
        creation_order,
        "local_ai.reconnaissance.report_type",
    )? {
        0 => ReportType::Nothing,
        1 => ReportType::Noise,
        2 => ReportType::Body,
        3 => ReportType::MissedCharly,
        4 => ReportType::DeadBody,
        5 => ReportType::Enemy,
        value => {
            return Err(LegacyElementAdoptError::UnknownEnum {
                creation_order,
                field: "local_ai.reconnaissance.report_type",
                value,
            });
        }
    };
    Ok(ReconnaissanceReport {
        seek_position: Position {
            x: saved.seek_position_x,
            y: saved.seek_position_y,
            sector: sector(
                saved.seek_position_sector.0,
                topology,
                creation_order,
                "local_ai.reconnaissance.seek_position.sector",
            )?,
            level: saved.seek_position_level,
        },
        report_type,
        seen_bodies: saved
            .seen_bodies
            .iter()
            .copied()
            .map(|reference| {
                element_handle(
                    entities.resolve_element(reference)?,
                    ReferenceKind::Human,
                    creation_order,
                    "local_ai.reconnaissance.seen_bodies",
                )
            })
            .collect::<Result<_, _>>()?,
        charly: element_handle(
            entities.resolve_element(saved.charly)?,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.reconnaissance.charly",
        )?,
        charly_seen: saved.charly_seen,
    })
}

fn convert_stimulus(
    saved: &LegacyStimulus,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
) -> Result<Stimulus, LegacyElementAdoptError> {
    let (actual_info_type, actual_name, info) = match &saved.info {
        LegacyStimulusInfo::None => (0, "None", StimulusInfo::None),
        LegacyStimulusInfo::Noise {
            origin,
            noise_type: saved_noise_type,
            volume,
            elevation,
            ..
        } => (
            1,
            "Noise",
            StimulusInfo::Noise(Noise {
                origin: stimulus_position(
                    *origin,
                    topology,
                    creation_order,
                    "local_ai.stimulus_queue.noise.origin.sector",
                )?,
                noise_type: noise_type(
                    *saved_noise_type,
                    creation_order,
                    "local_ai.stimulus_queue.noise.type",
                )?,
                volume: *volume,
                elevation: *elevation,
                // Original does not serialize this member in INFO_NOISE.
                element_id: 0,
            }),
        ),
        LegacyStimulusInfo::Position(position) => (
            2,
            "Position",
            StimulusInfo::Position(stimulus_position(
                *position,
                topology,
                creation_order,
                "local_ai.stimulus_queue.position.sector",
            )?),
        ),
        LegacyStimulusInfo::Human(reference) => (
            3,
            "Human",
            StimulusInfo::Human(ai_handle(
                entities.resolve_ai_element(*reference)?,
                ReferenceKind::Human,
                creation_order,
                "local_ai.stimulus_queue.human",
            )?),
        ),
        LegacyStimulusInfo::Hint {
            position,
            teller,
            seek_flags,
        } => (
            4,
            "Hint",
            StimulusInfo::Hint(Hint {
                seek_point: stimulus_position(
                    *position,
                    topology,
                    creation_order,
                    "local_ai.stimulus_queue.hint.position.sector",
                )?,
                seek_flags: *seek_flags,
                who_tells_me: ai_handle(
                    entities.resolve_ai_element(*teller)?,
                    ReferenceKind::Npc,
                    creation_order,
                    "local_ai.stimulus_queue.hint.teller",
                )?,
            }),
        ),
        LegacyStimulusInfo::Object(reference) => (
            5,
            "Object",
            StimulusInfo::Object(ai_handle(
                entities.resolve_ai_element(*reference)?,
                ReferenceKind::Object,
                creation_order,
                "local_ai.stimulus_queue.object",
            )?),
        ),
        LegacyStimulusInfo::Stolen { object, thief } => (
            6,
            "Stolen",
            StimulusInfo::Stolen(StolenObject {
                object: ai_handle(
                    entities.resolve_ai_element(*object)?,
                    ReferenceKind::Object,
                    creation_order,
                    "local_ai.stimulus_queue.stolen.object",
                )?,
                thief: ai_handle(
                    entities.resolve_ai_element(*thief)?,
                    ReferenceKind::Npc,
                    creation_order,
                    "local_ai.stimulus_queue.stolen.thief",
                )?,
            }),
        ),
        LegacyStimulusInfo::Combat {
            enemy_position,
            actor,
        } => (
            7,
            "Combat",
            StimulusInfo::Combat(CombatInfo {
                actor_npc: ai_handle(
                    entities.resolve_ai_element(*actor)?,
                    ReferenceKind::Npc,
                    creation_order,
                    "local_ai.stimulus_queue.combat.actor",
                )?,
                enemy_position: stimulus_position(
                    *enemy_position,
                    topology,
                    creation_order,
                    "local_ai.stimulus_queue.combat.enemy_position.sector",
                )?,
            }),
        ),
        LegacyStimulusInfo::DoorCombat {
            delay,
            direction,
            goal,
            adversary,
        } => (
            8,
            "DoorCombat",
            StimulusInfo::DoorCombat(DoorCombatInfo {
                delay: *delay,
                goal: stimulus_position(
                    *goal,
                    topology,
                    creation_order,
                    "local_ai.stimulus_queue.door_combat.goal.sector",
                )?,
                direction: *direction,
                adversary: ai_handle(
                    entities.resolve_ai_element(*adversary)?,
                    ReferenceKind::Human,
                    creation_order,
                    "local_ai.stimulus_queue.door_combat.adversary",
                )?,
            }),
        ),
        LegacyStimulusInfo::Index(value) => (9, "Index", StimulusInfo::Index(*value)),
    };
    if saved.info_type != actual_info_type {
        return Err(LegacyElementAdoptError::StimulusInfoMismatch {
            creation_order,
            declared: saved.info_type,
            actual: actual_name,
        });
    }
    Ok(Stimulus {
        stimulus_type: stimulus_type(
            saved.stimulus_type,
            creation_order,
            "local_ai.stimulus_queue.stimulus_type",
        )?,
        info,
        owner: element_handle(
            entities.resolve_element(saved.owner)?,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.stimulus_queue.owner",
        )?,
        to_whole_patrol: saved.to_whole_patrol,
    })
}

fn ai_brain_kind(brain: &AiBrain) -> &'static str {
    match brain {
        AiBrain::None => "None",
        AiBrain::Enemy(_) => "Enemy",
        AiBrain::Friendly(_) => "Friendly",
    }
}

fn ai_base_mut(brain: &mut AiBrain) -> Option<&mut AiController> {
    match brain {
        AiBrain::None => None,
        AiBrain::Enemy(ai) => Some(&mut ai.base),
        AiBrain::Friendly(ai) => Some(&mut ai.base),
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

    #[test]
    fn original_eye_status_values_match_rhviewparameters_order() {
        assert_eq!(eye_status(0, 31).unwrap(), EyeStatus::Closed);
        assert_eq!(eye_status(1, 31).unwrap(), EyeStatus::LookForward);
        assert_eq!(eye_status(6, 31).unwrap(), EyeStatus::Follow);
        assert_eq!(eye_status(8, 31).unwrap(), EyeStatus::ViewconeGrow);
        assert!(eye_status(9, 31).is_err());
    }

    #[test]
    fn local_ai_flags_preserve_the_complete_original_word() {
        assert_eq!(
            goto_flags(GotoFlags::RUN.bits(), 31, "flags").unwrap(),
            GotoFlags::RUN
        );
        assert_eq!(
            goto_flags(u16::MAX, 31, "flags").unwrap().bits(),
            u16::MAX,
            "Original defines all sixteen walking-flag bits"
        );
    }

    #[test]
    fn local_ai_reference_kind_is_validated_before_apply() {
        let target = EntityId::new(7, crate::entity_id::EntityIdKind::Target);
        assert!(matches!(
            checked_reference(Some(target), ReferenceKind::Human, 31, "target"),
            Err(LegacyElementAdoptError::WrongReferenceKind {
                entity_id,
                actual: crate::entity_id::EntityIdKind::Target,
                ..
            }) if entity_id == target
        ));
    }

    #[test]
    fn local_ai_signed_enums_reject_negative_and_unknown_values() {
        assert!(matches!(
            stimulus_type(-1, 31, "stimulus"),
            Err(LegacyElementAdoptError::NegativeEnum { value: -1, .. })
        ));
        assert!(matches!(
            noise_type(15, 31, "noise"),
            Err(LegacyElementAdoptError::UnknownEnum { value: 15, .. })
        ));
        assert_eq!(
            remark(Remark::TheSoundOfSilence as i32, 31, "current_remark").unwrap(),
            Remark::TheSoundOfSilence
        );
    }

    #[test]
    fn local_ai_path_restores_saved_cursor_and_history() {
        let saved = LegacyAiPathStatus {
            current_waypoint_index: 1,
            last_waypoint_index: 0,
            forward_movement: false,
            hiking_path_index: Some(0),
            history: vec![super::super::payload_ai::LegacyAiPathHistoryEntry {
                position_x: 12.0,
                position_y: 34.0,
                sector: super::super::payload_base::LegacySectorRef(Some(1)),
                level: 2,
                direction: 3,
                distance: 4,
            }],
        };
        let paths = vec![crate::level_data::RawHikingPath {
            waypoints: vec![
                crate::level_data::RawWaypoint {
                    x: 0,
                    y: 0,
                    sector: 0,
                    level: 0,
                    command: WaypointCommand::None,
                },
                crate::level_data::RawWaypoint {
                    x: 1,
                    y: 1,
                    sector: 1,
                    level: 2,
                    command: WaypointCommand::Macro(vec![0, 1, 2]),
                },
            ],
        }];
        let topology = LegacyPositionTopology {
            sector_count: 2,
            doors: Vec::new(),
            projection_areas: Vec::new(),
            sight_obstacles: Vec::new(),
        };

        let restored = convert_patrol_path(&saved, 31, &topology, &paths)
            .unwrap()
            .unwrap();
        assert_eq!(restored.hiking_path_index.get(), 0);
        assert_eq!(restored.current_waypoint_index, 1);
        assert!(!restored.forward);
        assert_eq!(restored.history[0].position.sector.unwrap().get(), 1);
        assert_eq!(restored.history[0].distance, 4);
    }
}
