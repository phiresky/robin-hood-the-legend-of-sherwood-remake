//! Atomic adoption of the common static-element state in Original v48 saves.
//!
//! This module deliberately builds an owned, fully validated plan before
//! touching the initialized mission.  It is not wired into the replay runner
//! yet: later stages must first adopt the remaining leaf, sequence, AI, and
//! manager state into the same candidate engine.

use std::{collections::BTreeMap, num::NonZeroU32};

use thiserror::Error;

use crate::{
    actor_state::{ActorContinuationState, ActorSeekSector},
    ai::{
        AiController, AiGlobalState, AiLockFlags, AiState, AlertLevel, Attitude, CombatInfo,
        DetachedPatrolPathStatus, DoorCombatInfo, GotoFlags, Hint, Noise, NoiseType,
        PathHistoryEntry, PathId, PatrolPath, Position, Question, ReconnaissanceReport, Remark,
        ReportType, Stimulus, StimulusInfo, StimulusType, StolenObject, Substate,
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
    sprite::MotionState,
};

use super::{
    adopt::{
        LegacyEntityFixups, LegacyLineTopology, LegacyLineTopologyError, LegacyPositionTopology,
        LegacySaveAdoptError, preflight_v48_position,
    },
    payload_ai::{
        LegacyAiPathStatus, LegacyAiPosition, LegacyLocalAiCommon, LegacyLocalAiTail,
        LegacyStimulus, LegacyStimulusInfo, LegacyStimulusPosition,
    },
    payload_base::{
        LegacyActorPayload, LegacyAiElementRef, LegacyElementPayloadBase, LegacyLineRef,
        LegacyNpcPayload, LegacyNpcView, LegacyPoint2, LegacySpritePayload,
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
    "RHElementActor sequence/order pointers and inline post-seek sequence",
    "RHElementActor script member variables",
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
    "RHArtificialMalignity duplicate pre-personal seek directions and first seek-flags/status copies (later serialized copies overwrite them during Original load)",
];

#[derive(Debug, Error)]
pub enum LegacyElementAdoptError {
    #[error(transparent)]
    Common(#[from] LegacySaveAdoptError),
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
        "saved element creation order {creation_order} field {field} references sector {index}, but initialized topology has {count} sector slots"
    )]
    MissingSector {
        creation_order: u32,
        field: &'static str,
        index: u16,
        count: usize,
    },
    #[error(
        "saved NPC creation order {creation_order} field {field} references Original gate slot {index}, but initialized topology has {count} gate slots"
    )]
    MissingGate {
        creation_order: u32,
        field: &'static str,
        index: i16,
        count: usize,
    },
    #[error(
        "saved element creation order {creation_order} field {field} references layer {index}, but initialized topology has {count} layers"
    )]
    MissingLayer {
        creation_order: u32,
        field: &'static str,
        index: u16,
        count: usize,
    },
    #[error(
        "saved element creation order {creation_order} field {field} contains non-finite value {value}"
    )]
    NonFinite {
        creation_order: u32,
        field: &'static str,
        value: f32,
    },
    #[error(
        "saved actor creation order {creation_order} writes inconsistent duplicate distance-to-boundary values {first} and {second}"
    )]
    DistanceToBoundaryMismatch {
        creation_order: u32,
        first: f32,
        second: f32,
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
    #[error(
        "saved NPC creation order {creation_order} field {field} has {saved} entries, but initialized AI topology has {initialized}"
    )]
    AiTopologyCount {
        creation_order: u32,
        field: &'static str,
        saved: usize,
        initialized: usize,
    },
    #[error(
        "saved NPC creation order {creation_order} field {field} index {index} is invalid for initialized count {count}"
    )]
    InvalidAiIndex {
        creation_order: u32,
        field: &'static str,
        index: u32,
        count: usize,
    },
    #[error(transparent)]
    LineTopology(#[from] LegacyLineTopologyError),
    #[error(
        "saved NPC creation order {creation_order} has {saved_kind} leaf state but initialized entity is {runtime_kind:?}"
    )]
    NpcLeafKindMismatch {
        creation_order: u32,
        saved_kind: &'static str,
        runtime_kind: crate::entity_id::EntityIdKind,
    },
}

#[derive(Clone, Debug)]
pub struct LegacyStaticElementAdoption {
    records: Vec<ConvertedElement>,
    clear_ambush_points: bool,
}

#[derive(Clone, Debug)]
struct ConvertedElement {
    entity_id: EntityId,
    creation_order: u32,
    element: ConvertedElementBase,
    actor: Option<ConvertedActor>,
    npc: Option<ConvertedNpc>,
    npc_leaf: Option<ConvertedNpcLeaf>,
}

#[derive(Clone, Copy, Debug)]
enum ConvertedNpcLeaf {
    Soldier { apple_smell: u32 },
    Civilian { current_scroll_set: u32 },
}

#[derive(Clone, Debug)]
struct ConvertedElementBase {
    outline_colors: [u16; 5],
    current_outline: OutlineColorName,
    outline_width: u16,
    custom_minimap_dot: u16,
    active: bool,
    position_map_delayed: bool,
    delayed_map_position: MapPoint,
    position_delayed: bool,
    delayed_position: crate::coordinates::WorldPoint3D,
    in_honolulu: bool,
    index_in_elements_list: u16,
    blipped: bool,
    unreachable: bool,
    sprite: ConvertedSprite,
}

/// Preflighted common `RHElement`/`RHSprite` state for an element embedded
/// outside the main element stream (currently `RHPatchFX`).
///
/// Original serializes the patch-owned FX base a second time while walking
/// the grid, so that later copy is authoritative and must compose with the
/// ordinary element adoption rather than being discarded.
#[derive(Clone, Debug)]
pub(crate) struct LegacyElementBaseAdoption {
    entity_id: EntityId,
    creation_order: u32,
    element: ConvertedElementBase,
}

impl LegacyElementBaseAdoption {
    pub(crate) fn preflight(
        engine: &EngineInner,
        saved: &LegacyElementPayloadBase,
        entities: &LegacyEntityFixups,
        position_topology: &LegacyPositionTopology,
    ) -> Result<Self, LegacyElementAdoptError> {
        let creation_order = saved.creation_order;
        let entity_id = entities
            .by_creation_order
            .get(&creation_order)
            .copied()
            .ok_or(LegacySaveAdoptError::MissingCreationOrderReference { creation_order })?;
        if engine.world.entities.get(entity_id).is_none() {
            return Err(LegacyElementAdoptError::MissingEntity {
                creation_order,
                entity_id,
            });
        }
        Ok(Self {
            entity_id,
            creation_order,
            element: convert_element(saved, creation_order, entities, position_topology)?,
        })
    }

    pub(crate) fn entity_id(&self) -> EntityId {
        self.entity_id
    }

    pub(crate) fn apply(self, engine: &mut EngineInner) {
        let entity = engine
            .world
            .entities
            .get_mut(self.entity_id)
            .expect("preflighted embedded v48 element disappeared from candidate engine");
        apply_element_base(entity.element_data_mut(), self.element);
        debug_assert_eq!(
            engine
                .world
                .original_creation_order_by_entity
                .get(&self.entity_id),
            Some(&self.creation_order)
        );
    }
}

#[derive(Clone, Debug)]
struct ConvertedSprite {
    current_row: u16,
    current_frame: u16,
    frame_count: u16,
    flight_frame_countdown: u16,
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
    continuation: ActorContinuationState,
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
    body_visitors: u16,
    fried_pikachu: bool,
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
    angle_iterator: f32,
    angle_iterator_step: f32,
    angle_step: f32,
    angle: f32,
    half_aperture: f32,
    real_half_aperture: f32,
    half_aperture_cosine: f32,
    future_half_aperture: f32,
    half_aperture_step: f32,
    half_aperture_changes: bool,
    crazy_angle_iterator: f32,
    crazy_angle_iterator_step: f32,
    crazy_color_iterator: u8,
    crazy_half_angle_range: f32,
    direction: [f32; 2],
    left_side: [f32; 2],
    right_side: [f32; 2],
    stare: GroundPoint,
    follow_target: Option<EntityId>,
    radius_goal: u16,
    radius: u16,
    radius_reduction_permil: u16,
    radius_step: u16,
    long_range: f32,
    real_radius: u16,
    drunkenness: [f32; 4],
    sniper: bool,
    leaning: bool,
}

#[derive(Clone, Debug)]
enum ConvertedLocalAi {
    Friendly {
        common: ConvertedLocalAiCommon,
        fleeing_seen_enemy_counter: u16,
        beggar_dont_talk_counter: u16,
        wants_to_talk: bool,
        last_talk_partner: u32,
        can_go_away: bool,
    },
    Enemy {
        common: ConvertedLocalAiCommon,
        last_stimulus_dispatched_to_patrol: Stimulus,
        frame_when_missed_charly: u32,
        heard_nets: Vec<u32>,
        frame_when_enemy_detected: u32,
        fleeing_seen_enemy_counter: u16,
        other_seen_ale: Vec<u32>,
        pc_gone_away_direction: u16,
        detected_something_there: Position,
        missed_pc: u32,
        last_seek_direction_index: u8,
        beggar_to_examine: u32,
        pc_missed: bool,
        search_charly_way: Vec<Position>,
        current_task_priority: u16,
        minimal_task_priority: u16,
        new_task_priority: u16,
        number_of_different_checkpoints: u8,
        delta_sorrow_level: u16,
        missed_in_action: Vec<u32>,
        other_bodies_to_examine: Vec<u32>,
        beggars_to_control: Vec<u32>,
        thirsty: bool,
        old_life_points: u8,
        initial_life_points: u8,
        list_them: Vec<u32>,
        old_odds: i16,
        position_change_locked_for_test: bool,
        ambush_point_array_reset: bool,
        ambush_point_status: Vec<crate::ai_enemy::AmbushPointStatus>,
        my_seek_points: Vec<u16>,
        personal_seek_point_1: Option<crate::ai::SeekPoint>,
        personal_seek_point_2: Option<crate::ai::SeekPoint>,
        seek_center: Position,
        actual_seek_point: Option<u16>,
        seek_point_view_directions: Vec<u16>,
        positions_of_beggars_to_control: Vec<Position>,
        seek_flags: crate::ai_enemy::SeekFlags,
        forced_next_battle_decision: crate::ai::Decision,
        reset_battle_decision: bool,
        synchronize_index: u16,
        seen_dead_body: bool,
        seeking_charly: bool,
        initial_view_cone: crate::ai::ViewCone,
        company_number: u16,
        left_combat_neighbour: u32,
        right_combat_neighbour: u32,
        attentive: bool,
        will_be_attentive: bool,
        forced_attentive: bool,
        guarded_pc: Option<crate::entity_id::PcId>,
        tower_guard: bool,
        combat_trainer: bool,
        gather_position: Position,
        gather_direction: u16,
        gather_position_instructed: bool,
        officers_position: Position,
        previous_state: i32,
        previous_substate: i32,
        reported_to_officer: bool,
        missed_soldier_timer: u16,
        old_money: u16,
        other_seen_money: Vec<u32>,
        money_fight_enemies: Vec<u32>,
        money_fight_victims: Vec<u32>,
        archer_behind_me: u32,
        shield_bearer_before_me: u32,
        already_seen_bodies: Vec<u32>,
        my_line_jump: Option<u32>,
        shield_bearer_direction: u16,
        phalanx_aborted: bool,
        changed_to_alert_path: bool,
        my_shooting_point: Option<(u16, u16)>,
        my_archery_sector: Option<u16>,
        my_archery_sector_index: u16,
        my_archery_point_index: crate::sector::ArcheryPointIdx,
        my_archery_point_increment: i8,
        enemy_seen_below: bool,
        enemy_had_this_elevation: u16,
        known_enemy_strikes: [Option<crate::weapons::SwordStrike>; 3],
    },
}

#[derive(Clone, Copy)]
enum ReferenceKind {
    Human,
    Npc,
    Object,
    Scroll,
    Net,
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
            Self::Scroll => matches!(kind, EntityIdKind::Scroll),
            Self::Net => matches!(kind, EntityIdKind::Net),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Human => "PC, Soldier, or Civilian",
            Self::Npc => "Soldier or Civilian",
            Self::Object => "Bonus, Scroll, Projectile, or Net",
            Self::Scroll => "Scroll",
            Self::Net => "Net",
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
    old_state: i32,
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
    detached_patrol_path_status: DetachedPatrolPathStatus,
    stop_before_end_of_path: bool,
    use_max_norm_to_stop_before_end_of_path: bool,
    stop_before_end_of_path_distance: u16,
    macro_cursor: Option<ConvertedMacroCursor>,
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
    seek_position: Position,
    alert_soldiers_point: Position,
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
    remaining_tequila_gulps: u8,
    friends_are_alerted: bool,
    is_stay_at_home: bool,
    locks_flag_field: AiLockFlags,
    was_busy: bool,
    script_locked: bool,
    remember_events: bool,
    leave_house_number: u16,
    last_hint_actuality: u32,
    last_hint_subject: Question,
    my_door_index: Option<u32>,
    looking_for_help_because_enemy_seen: bool,
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

#[derive(Clone, Copy, Debug)]
struct SavedHumanGeometry {
    map: MapPoint,
    sector: Option<SectorHandle>,
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
        let line_topology = LegacyLineTopology::derive(engine, assets)?;
        let saved_human_geometries = saved_human_geometries(payloads, entities, position_topology)?;
        let mut records = Vec::with_capacity(payloads.records.len());
        for record in &payloads.records {
            if matches!(
                &record.payload,
                LegacyElementPayload::ObjectItem(LegacyObjectItemPayload::Mobile(_))
            ) {
                continue;
            }
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
                    .map(|actor| {
                        convert_actor(
                            actor,
                            creation_order,
                            entities,
                            position_topology,
                            engine.world.fast_grid.level.layers.len(),
                        )
                    })
                    .transpose()?,
                npc: npc
                    .map(|npc| {
                        convert_npc(
                            npc,
                            engine,
                            runtime
                                .npc_data()
                                .expect("NPC kind was validated immediately above"),
                            entity_id,
                            creation_order,
                            entities,
                            position_topology,
                            assets,
                            &engine.ai.global,
                            &line_topology,
                            &saved_human_geometries,
                        )
                    })
                    .transpose()?,
                npc_leaf: convert_npc_leaf(&record.payload, entity_id, creation_order)?,
            });
        }
        let clear_ambush_points = records.iter().any(|record| {
            record
                .npc
                .as_ref()
                .is_some_and(|npc| matches!(&npc.local_ai, ConvertedLocalAi::Enemy { .. }))
        });
        Ok(Self {
            records,
            clear_ambush_points,
        })
    }

    /// Install a preflighted plan into a candidate engine.
    ///
    /// The lookups cannot fail when called on the candidate used for
    /// [`Self::preflight`].  Callers should clone the initialized engine,
    /// preflight all save sections against it, apply all plans, then swap the
    /// complete candidate into service.
    pub(crate) fn apply(self, engine: &mut EngineInner) {
        if self.clear_ambush_points {
            clear_ambush_points_on_enemy_load(&mut engine.ai.global);
        }
        for converted in self.records {
            let entity = engine
                .world
                .entities
                .get_mut(converted.entity_id)
                .expect("preflighted v48 entity disappeared from candidate engine");
            apply_element_base(entity.element_data_mut(), converted.element);

            if let Some(saved) = converted.actor {
                let actor = entity
                    .actor_data_mut()
                    .expect("preflighted v48 actor changed kind in candidate engine");
                actor.continuation = saved.continuation;
                actor.last_execute_order_id = saved.last_order_id;
                actor.old_action = saved.old_action;
                actor.action_state = saved.action_state;
                actor.execution_frozen = saved.execution_frozen;
                actor.is_ignored_for_anti_collision = saved.ignored_for_anti_collision;
                actor.execute_order_initialising = saved.new_order;
                actor.wait_time = saved.wait_time;
                // Original serializes one overloaded `mulWaitTime` for both
                // WAIT_TIMER and seek-refresh aging. Rust separates those
                // responsibilities, but the live sequence is adopted later
                // and therefore cannot select the owner of this scalar here.
                // Seed both candidates from the authoritative saved value;
                // starting a new wait or seek will overwrite its own field.
                actor.seek_refresh_wait = saved.wait_time;
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
            if let Some(saved) = converted.npc_leaf {
                match (entity, saved) {
                    (
                        crate::element::Entity::Soldier(soldier),
                        ConvertedNpcLeaf::Soldier { apple_smell },
                    ) => soldier.soldier.apple_smell = apple_smell,
                    (
                        crate::element::Entity::Civilian(civilian),
                        ConvertedNpcLeaf::Civilian { current_scroll_set },
                    ) => civilian.civilian.current_scroll_set = current_scroll_set,
                    _ => unreachable!("preflighted NPC leaf kind changed in candidate engine"),
                }
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

fn apply_element_base(element: &mut crate::element::ElementData, converted: ConvertedElementBase) {
    element.outline_colors = converted.outline_colors;
    element.current_outline = converted.current_outline;
    element.outline_width = converted.outline_width;
    element.custom_minimap_dot = converted.custom_minimap_dot;
    element.active = converted.active;
    element.position_map_delayed = converted.position_map_delayed;
    element.delayed_map_position = converted.delayed_map_position;
    element.position_delayed = converted.position_delayed;
    element.delayed_position = converted.delayed_position;
    element.in_honolulu = converted.in_honolulu;
    element.index_in_elements_list = converted.index_in_elements_list;
    element.blipped = converted.blipped;
    element.unreachable = converted.unreachable;
    let sprite = &mut element.sprite;
    // RHSprite serializes its own fields before delegating to
    // RHPositionInterface::Serialize. On read, a changed alternate-profile
    // flag immediately calls SwitchAlternateProfile(), so the row is
    // recomputed with the freshly constructed sprite's direction; the saved
    // position/direction is restored only afterward. Preserve that observable
    // load-order artifact instead of treating the payload as one atomic state.
    let pre_restore_alternate_profile = sprite.use_alternate_profile;
    let pre_restore_direction = sprite.position_iface.get_direction().as_u8() as u16;
    sprite.current_row = converted.sprite.current_row;
    sprite.current_frame = converted.sprite.current_frame;
    sprite.frame_count = converted.sprite.frame_count;
    sprite.flight_frame_countdown = converted.sprite.flight_frame_countdown;
    sprite.current_height = converted.sprite.current_height;
    sprite.current_width = converted.sprite.current_width;
    sprite.last_action = converted.sprite.last_action;
    if converted.sprite.alternate_profile != pre_restore_alternate_profile {
        sprite.use_alternate_profile = pre_restore_alternate_profile;
        sprite.switch_alternate_profile(pre_restore_direction);
    } else {
        sprite.use_alternate_profile = converted.sprite.alternate_profile;
    }
    sprite.masked = converted.sprite.masked;
    sprite.behind_display_order_ref = converted.sprite.behind_display_order_reference;
    sprite.display_order_ref = converted.sprite.display_order_reference;
    sprite.action_done_frame = converted.sprite.action_done_frame;
    sprite.action_done_counter = converted.sprite.action_done_counter;
    sprite.last_sound_id = converted.sprite.last_sound_id;
    sprite.last_processed_order_id = converted.sprite.last_processed_order_id;
    (sprite.anims_to_be_replaced, sprite.replacing_anims) =
        converted.sprite.animation_replacements.into_iter().unzip();
    restore_position_and_gameplay_posture(element, converted.sprite.position);
}

fn restore_position_and_gameplay_posture(
    element: &mut crate::element::ElementData,
    position: PositionInterfaceV48State,
) {
    // Original has only one posture source: RHElement::GetPosture forwards
    // directly to the RHPositionInterface embedded in its sprite, and that
    // position interface is what v48 saves serialize. Rust keeps a separate
    // gameplay-facing posture, so adoption must install both from the same
    // serialized value. Assign directly instead of calling set_posture:
    // restoration must not apply the runtime corpse-transition guard.
    element.posture = position.posture;
    element
        .sprite
        .position_iface
        .restore_v48_serialized_state(position);
}

fn convert_element(
    saved: &LegacyElementPayloadBase,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
) -> Result<ConvertedElementBase, LegacyElementAdoptError> {
    // RHElement consumes these coordinates only while the corresponding
    // delayed-position bit is set. Its setters overwrite the complete point
    // before setting that bit, so inactive bytes are dormant constructor
    // storage and may legitimately contain non-finite legacy values.
    if saved.position_map_delayed {
        finite_point(
            saved.delayed_map_position,
            creation_order,
            "delayed_map_position",
        )?;
    }
    if saved.position_delayed {
        finite(
            saved.delayed_position.x,
            creation_order,
            "delayed_position.x",
        )?;
        finite(
            saved.delayed_position.y,
            creation_order,
            "delayed_position.y",
        )?;
        finite(
            saved.delayed_position.z,
            creation_order,
            "delayed_position.z",
        )?;
    }
    Ok(ConvertedElementBase {
        outline_colors: saved.outline_colors,
        current_outline: outline(saved.current_outline, creation_order)?,
        outline_width: saved.outline_width,
        custom_minimap_dot: saved.custom_minimap_dot,
        active: saved.active,
        position_map_delayed: saved.position_map_delayed,
        delayed_map_position: MapPoint::new(
            saved.delayed_map_position.x,
            saved.delayed_map_position.y,
        ),
        position_delayed: saved.position_delayed,
        delayed_position: crate::coordinates::WorldPoint3D::new(
            saved.delayed_position.x,
            saved.delayed_position.y,
            saved.delayed_position.z,
        ),
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
        flight_frame_countdown: saved.frame_count_down,
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
        last_processed_order_id: map_legacy_order_id_or_sentinel(saved.last_processed_order_id),
        animation_replacements,
        position: preflight_v48_position(&saved.position, entities, topology)?,
    })
}

/// Map Original's zero-based order identity into Rust's non-zero domain while
/// preserving the all-ones "no processed order" sentinel.
fn map_legacy_order_id_or_sentinel(value: u32) -> u32 {
    if value == u32::MAX {
        u32::MAX
    } else {
        value + 1
    }
}

fn convert_actor(
    saved: &LegacyActorPayload,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
    layer_count: usize,
) -> Result<ConvertedActor, LegacyElementAdoptError> {
    let last_order_id = match saved.last_order_id {
        u32::MAX => None,
        value => NonZeroU32::new(
            value
                .checked_add(1)
                .expect("u32::MAX was handled as the Original null sentinel"),
        ),
    };
    // Original serializes this cache twice and verifies neither copy. Keep
    // the exact cached bits (including a legacy NaN) while still requiring
    // the redundant records to agree. GetDistanceToBoundary refreshes it
    // whenever the actor moves and ignores it when no material sector exists.
    if saved.distance_to_boundary_first.to_bits() != saved.distance_to_boundary_second.to_bits() {
        return Err(LegacyElementAdoptError::DistanceToBoundaryMismatch {
            creation_order,
            first: saved.distance_to_boundary_first,
            second: saved.distance_to_boundary_second,
        });
    }
    finite_point(saved.bypass_exit, creation_order, "bypass_exit")?;
    finite_point(
        saved.position_at_last_distance_request,
        creation_order,
        "position_at_last_distance_request",
    )?;
    for point in &saved.bypass_points {
        finite_point(*point, creation_order, "bypass_points")?;
    }
    if saved.seek_to_point && usize::from(saved.seek_layer) >= layer_count {
        return Err(LegacyElementAdoptError::MissingLayer {
            creation_order,
            field: "seek_layer",
            index: saved.seek_layer,
            count: layer_count,
        });
    }
    let menacer = checked_reference(
        entities.resolve_element(saved.menacer)?,
        ReferenceKind::Npc,
        creation_order,
        "menacer",
    )?;
    let motion_state = motion_state(saved.motion_state, creation_order)?;
    let continuation = ActorContinuationState {
        about_to_surrender: saved.about_to_surrender,
        surrendering: saved.surrendering,
        menacer,
        distance_to_boundary: saved.distance_to_boundary_second,
        position_at_last_distance_request: MapPoint::new(
            saved.position_at_last_distance_request.x,
            saved.position_at_last_distance_request.y,
        ),
        motion_state,
        seek_layer: saved.seek_layer,
        seek_to_point: saved.seek_to_point,
        seek_sector: seek_sector(saved.seek_sector.0, topology, creation_order, "seek_sector")?,
        check_for_jump: saved.check_for_jump,
        bypassing: saved.bypassing,
        on_railroad: saved.on_railroad,
        bypass_exit: MapPoint::new(saved.bypass_exit.x, saved.bypass_exit.y),
        bypass_reference: entities.resolve_element(saved.bypass_reference)?,
        bypass_points: saved
            .bypass_points
            .iter()
            .map(|point| MapPoint::new(point.x, point.y))
            .collect(),
        material_sector: sector(
            saved.material_sector.0,
            topology,
            creation_order,
            "material_sector",
        )?,
    };
    Ok(ConvertedActor {
        continuation,
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

fn saved_human_geometries(
    payloads: &LegacyElementPayloadStream,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
) -> Result<BTreeMap<EntityId, SavedHumanGeometry>, LegacyElementAdoptError> {
    let mut geometries = BTreeMap::new();
    for record in &payloads.records {
        let position = match &record.payload {
            LegacyElementPayload::ActorPc(pc) => &pc.human.actor.element.sprite.position,
            LegacyElementPayload::ActorNpcSoldier(soldier) => {
                &soldier.npc.human.actor.element.sprite.position
            }
            LegacyElementPayload::ActorNpcCivilian(civilian) => {
                &civilian.npc.human.actor.element.sprite.position
            }
            _ => continue,
        };
        let creation_order = record.header.creation_order;
        let entity_id = entities
            .by_creation_order
            .get(&creation_order)
            .copied()
            .ok_or(LegacySaveAdoptError::MissingCreationOrderReference { creation_order })?;
        geometries.insert(
            entity_id,
            SavedHumanGeometry {
                map: MapPoint::new(position.map.x, position.map.y),
                sector: sector(
                    position.sector.0,
                    topology,
                    creation_order,
                    "position.sector",
                )?,
            },
        );
    }
    Ok(geometries)
}

#[allow(clippy::too_many_arguments)]
fn resolve_saved_enemy_jump_line(
    reference: LegacyLineRef,
    primary_target: LegacyAiElementRef,
    owner_id: EntityId,
    owner_creation_order: u32,
    entities: &LegacyEntityFixups,
    geometries: &BTreeMap<EntityId, SavedHumanGeometry>,
    topology: &LegacyLineTopology,
    engine: &EngineInner,
    assets: &LevelAssets,
) -> Result<Option<crate::jump_line::JumpLineIndex>, LegacyElementAdoptError> {
    match topology.resolve("local_ai.enemy.jump_line", reference) {
        Ok(line) => return Ok(line),
        Err(LegacyLineTopologyError::Missing { .. }) => {}
        Err(error) => return Err(error.into()),
    }

    let target_id = entities
        .resolve_ai_element(primary_target)?
        .ok_or_else(|| shifted_enemy_line_error(reference, owner_creation_order, 0))?;
    let target_creation_order = entities
        .creation_order_by_entity
        .get(&target_id)
        .copied()
        .ok_or_else(|| {
            shifted_enemy_line_error(reference, owner_creation_order, target_id.index())
        })?;
    let owner_geometry = geometries.get(&owner_id).ok_or_else(|| {
        shifted_enemy_line_error(reference, owner_creation_order, target_creation_order)
    })?;
    let target_geometry = geometries.get(&target_id).ok_or_else(|| {
        shifted_enemy_line_error(reference, owner_creation_order, target_creation_order)
    })?;
    let owner_sector = owner_geometry.sector.ok_or_else(|| {
        shifted_enemy_line_error(reference, owner_creation_order, target_creation_order)
    })?;
    let target_sector = target_geometry.sector.ok_or_else(|| {
        shifted_enemy_line_error(reference, owner_creation_order, target_creation_order)
    })?;
    let owner =
        engine
            .world
            .entities
            .get(owner_id)
            .ok_or(LegacyElementAdoptError::MissingEntity {
                creation_order: owner_creation_order,
                entity_id: owner_id,
            })?;
    let maximal_sword_range =
        crate::engine::melee::get_hth_weapon_id_full(owner, &assets.profile_manager)
            .and_then(|weapon| assets.profile_manager.get_hth_weapon(weapon))
            .map(|weapon| weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] as f32)
            .ok_or_else(|| {
                shifted_enemy_line_error(reference, owner_creation_order, target_creation_order)
            })?;

    topology
        .resolve_enemy_jump_line(
            "local_ai.enemy.jump_line",
            reference,
            &engine.world.fast_grid,
            owner_creation_order,
            owner_sector,
            target_creation_order,
            target_sector,
            target_geometry.map,
            maximal_sword_range,
        )
        .map_err(Into::into)
}

fn shifted_enemy_line_error(
    reference: LegacyLineRef,
    owner: u32,
    target: u32,
) -> LegacyElementAdoptError {
    LegacyLineTopologyError::MissingGeometryIdentity {
        field: "local_ai.enemy.jump_line",
        layer: reference.layer.unwrap_or(u16::MAX),
        index: reference.index.unwrap_or(-1),
        owner,
        target,
    }
    .into()
}

fn convert_npc(
    saved: &LegacyNpcPayload,
    engine: &EngineInner,
    runtime: &NpcData,
    entity_id: EntityId,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
    assets: &LevelAssets,
    ai_global: &AiGlobalState,
    line_topology: &LegacyLineTopology,
    saved_human_geometries: &BTreeMap<EntityId, SavedHumanGeometry>,
) -> Result<ConvertedNpc, LegacyElementAdoptError> {
    finite(
        saved.initial_position.x,
        creation_order,
        "initial_position.x",
    )?;
    finite(
        saved.initial_position.y,
        creation_order,
        "initial_position.y",
    )?;
    finite_point(saved.initial_view, creation_order, "initial_view")?;

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
    crate::engine::debug_detectable_mutation_load_snapshot(
        entity_id,
        creation_order,
        &detectable_lists,
        |target_id| entities.creation_order_by_entity.get(&target_id).copied(),
    );
    Ok(ConvertedNpc {
        life: saved.life,
        arrows: saved.arrows,
        old_direction: saved.old_direction,
        register: saved.register,
        attached_scroll: checked_reference(
            entities.resolve_element(saved.attached_scroll)?,
            ReferenceKind::Scroll,
            creation_order,
            "attached_scroll",
        )?,
        body_visitors: saved.body_visitors,
        fried_pikachu: saved.fried,
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
            ai_global,
            line_topology,
            saved_human_geometries,
            engine,
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
    npc.attached_scroll = saved.attached_scroll;
    npc.body_visitors = saved.body_visitors;
    npc.fried_pikachu = saved.fried_pikachu;
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
    npc.view_angle_iterator = saved.view.angle_iterator;
    npc.view_angle_iterator_step = saved.view.angle_iterator_step;
    npc.view_angle_step = saved.view.angle_step;
    npc.view_angle = saved.view.angle;
    npc.half_aperture = saved.view.half_aperture;
    npc.real_half_aperture = saved.view.real_half_aperture;
    npc.view_half_aperture_cosine = saved.view.half_aperture_cosine;
    npc.view_future_half_aperture = saved.view.future_half_aperture;
    npc.view_half_aperture_step = saved.view.half_aperture_step;
    npc.view_half_aperture_changes = saved.view.half_aperture_changes;
    npc.view_crazy_angle_iterator = saved.view.crazy_angle_iterator;
    npc.view_crazy_angle_iterator_step = saved.view.crazy_angle_iterator_step;
    npc.view_crazy_color_iterator = saved.view.crazy_color_iterator;
    npc.view_crazy_half_angle_range = saved.view.crazy_half_angle_range;
    npc.view_direction = saved.view.direction;
    npc.view_left_side = saved.view.left_side;
    npc.view_right_side = saved.view.right_side;
    npc.stare_point = saved.view.stare;
    npc.follow_target = saved.view.follow_target;
    npc.view_radius_goal = saved.view.radius_goal;
    npc.view_radius_base = saved.view.radius;
    npc.view_radius_reduction_permil = saved.view.radius_reduction_permil;
    npc.view_radius_step = saved.view.radius_step;
    npc.view_longrange_radius_factor = saved.view.long_range;
    npc.view_radius = saved.view.real_radius;
    npc.drunken_cone_iterators = saved.view.drunkenness;
    npc.view_sniper = saved.view.sniper;
    npc.view_lean_out = saved.view.leaning;
    apply_local_ai(&mut npc.ai_brain, saved.local_ai);
    let ai = ai_base_mut(&mut npc.ai_brain)
        .expect("preflighted local-AI kind cannot become None in candidate engine");
    // The saved view status and follow target are already authoritative.
    // Rust's edge-triggered primary-target reconciliation is runtime-only
    // bookkeeping; leaving its marker at the constructor default would make
    // the first post-load RefreshView synthesize a Focus(primary_target) and
    // overwrite a saved LookForward/Stare state that Original preserves.
    ai.last_synced_focus_target = (ai.primary_target != 0).then_some(ai.primary_target);
    ai.initial_position = ai_initial_position;
    ai.initial_view_direction = ai_initial_view_direction;
}

fn convert_npc_view(
    saved: &LegacyNpcView,
    follow_target: Option<EntityId>,
    creation_order: u32,
) -> Result<ConvertedNpcView, LegacyElementAdoptError> {
    finite(saved.half_angle, creation_order, "view.half_angle")?;
    finite(saved.angle_iterator, creation_order, "view.angle_iterator")?;
    finite(
        saved.angle_iterator_step,
        creation_order,
        "view.angle_iterator_step",
    )?;
    finite(saved.angle_step, creation_order, "view.angle_step")?;
    finite(saved.angle, creation_order, "view.angle")?;
    finite(saved.half_aperture, creation_order, "view.half_aperture")?;
    finite(
        saved.real_half_aperture,
        creation_order,
        "view.real_half_aperture",
    )?;
    finite(
        saved.half_aperture_cosine,
        creation_order,
        "view.half_aperture_cosine",
    )?;
    finite(
        saved.future_half_aperture,
        creation_order,
        "view.future_half_aperture",
    )?;
    finite(
        saved.half_aperture_step,
        creation_order,
        "view.half_aperture_step",
    )?;
    finite(saved.crazy_iterator, creation_order, "view.crazy_iterator")?;
    finite(
        saved.crazy_iterator_step,
        creation_order,
        "view.crazy_iterator_step",
    )?;
    finite(
        saved.crazy_half_aperture,
        creation_order,
        "view.crazy_half_aperture",
    )?;
    finite_point(saved.direction, creation_order, "view.direction")?;
    finite_point(saved.left, creation_order, "view.left")?;
    finite_point(saved.right, creation_order, "view.right")?;
    finite_point(saved.stare, creation_order, "view.stare")?;
    finite(saved.long_range, creation_order, "view.long_range")?;
    for value in saved.drunkenness {
        finite(value, creation_order, "view.drunkenness")?;
    }

    Ok(ConvertedNpcView {
        eye_status: eye_status(saved.status, creation_order)?,
        transition: saved.transitioning,
        alpha: saved.alpha,
        half_angle: saved.half_angle,
        angle_iterator: saved.angle_iterator,
        angle_iterator_step: saved.angle_iterator_step,
        angle_step: saved.angle_step,
        angle: saved.angle,
        half_aperture: saved.half_aperture,
        real_half_aperture: saved.real_half_aperture,
        half_aperture_cosine: saved.half_aperture_cosine,
        future_half_aperture: saved.future_half_aperture,
        half_aperture_step: saved.half_aperture_step,
        half_aperture_changes: saved.half_aperture_changes,
        crazy_angle_iterator: saved.crazy_iterator,
        crazy_angle_iterator_step: saved.crazy_iterator_step,
        crazy_color_iterator: saved.color,
        crazy_half_angle_range: saved.crazy_half_aperture,
        direction: [saved.direction.x, saved.direction.y],
        left_side: [saved.left.x, saved.left.y],
        right_side: [saved.right.x, saved.right.y],
        stare: GroundPoint::new(saved.stare.x, saved.stare.y),
        // The raw pointer echo inside LegacyNpcView is ABI residue;
        // LegacyNpcPayload::mobile_target is the authoritative pointer fixup.
        follow_target,
        radius_goal: saved.radius_goal,
        radius: saved.radius,
        radius_reduction_permil: saved.radius_reduction,
        radius_step: saved.radius_step,
        long_range: saved.long_range,
        real_radius: saved.real_radius,
        drunkenness: saved.drunkenness,
        sniper: saved.sniper,
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
    ai_global: &AiGlobalState,
    line_topology: &LegacyLineTopology,
    saved_human_geometries: &BTreeMap<EntityId, SavedHumanGeometry>,
    engine: &EngineInner,
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
                wants_to_talk: tail.wants_to_talk,
                last_talk_partner: ai_handle(
                    entities.resolve_ai_element(tail.last_talk_partner)?,
                    ReferenceKind::Npc,
                    creation_order,
                    "local_ai.friendly.last_talk_partner",
                )?,
                can_go_away: tail.can_go_away,
            })
        }
        (LegacyLocalAiTail::Enemy(tail), AiBrain::Enemy(runtime_ai)) => {
            let actual_seek_point = convert_actual_seek_point(
                tail.actual_seek_point_id,
                tail.personal_seek_point_1.is_some(),
                tail.personal_seek_point_2.is_some(),
                ai_global.seek_points.len(),
                creation_order,
            )?;
            let ambush_point_status = convert_ambush_point_statuses(
                &runtime_ai.ambush_point_status,
                &tail.ambush_point_statuses,
                creation_order,
            )?;
            let (my_shooting_point, my_archery_sector) =
                convert_archery_refs(tail, ai_global, creation_order)?;
            Ok(ConvertedLocalAi::Enemy {
                common,
                last_stimulus_dispatched_to_patrol: convert_stimulus(
                    &tail.last_stimulus_dispatched_to_patrol,
                    creation_order,
                    entities,
                    topology,
                )?,
                frame_when_missed_charly: tail.frame_when_missed_charly,
                heard_nets: ai_handle_list(
                    &tail.heard_nets,
                    ReferenceKind::Net,
                    creation_order,
                    "local_ai.enemy.heard_nets",
                    entities,
                )?,
                frame_when_enemy_detected: tail.frame_when_enemy_detected,
                fleeing_seen_enemy_counter: tail.fleeing_seen_enemy_counter,
                other_seen_ale: ai_handle_list(
                    &tail.other_seen_ale,
                    ReferenceKind::Object,
                    creation_order,
                    "local_ai.enemy.other_seen_ale",
                    entities,
                )?,
                pc_gone_away_direction: tail.pc_gone_away_direction,
                detected_something_there: ai_position(
                    tail.detected_something_there,
                    topology,
                    creation_order,
                    "local_ai.enemy.detected_something_there.sector",
                )?,
                missed_pc: ai_handle(
                    entities.resolve_ai_element(tail.missed_pc)?,
                    ReferenceKind::Human,
                    creation_order,
                    "local_ai.enemy.missed_pc",
                )?,
                last_seek_direction_index: tail.last_seek_direction_index,
                beggar_to_examine: ai_handle(
                    entities.resolve_ai_element(tail.beggar_to_examine)?,
                    ReferenceKind::Human,
                    creation_order,
                    "local_ai.enemy.beggar_to_examine",
                )?,
                pc_missed: tail.pc_missed,
                search_charly_way: ai_position_list(
                    &tail.search_charly_way,
                    topology,
                    creation_order,
                    "local_ai.enemy.search_charly_way.sector",
                )?,
                current_task_priority: tail.current_task_priority,
                minimal_task_priority: tail.minimal_task_priority,
                new_task_priority: tail.new_task_priority,
                number_of_different_checkpoints: tail.number_of_different_checkpoints,
                delta_sorrow_level: tail.delta_sorrow_level,
                missed_in_action: ai_handle_list(
                    &tail.missed_in_action,
                    ReferenceKind::Npc,
                    creation_order,
                    "local_ai.enemy.missed_in_action",
                    entities,
                )?,
                other_bodies_to_examine: ai_handle_list(
                    &tail.other_bodies_to_examine,
                    ReferenceKind::Human,
                    creation_order,
                    "local_ai.enemy.other_bodies_to_examine",
                    entities,
                )?,
                beggars_to_control: ai_handle_list(
                    &tail.beggars_to_control,
                    ReferenceKind::Human,
                    creation_order,
                    "local_ai.enemy.beggars_to_control",
                    entities,
                )?,
                thirsty: tail.thirsty,
                old_life_points: tail.old_life_points,
                initial_life_points: tail.initial_life_points,
                list_them: ai_handle_list(
                    &tail.them,
                    ReferenceKind::Human,
                    creation_order,
                    "local_ai.enemy.them",
                    entities,
                )?,
                old_odds: tail.old_odds,
                position_change_locked_for_test: tail.position_change_locked_for_test,
                ambush_point_array_reset: tail.ambush_point_array_reset,
                ambush_point_status,
                my_seek_points: convert_seek_point_ids(
                    &tail.seek_point_ids,
                    tail.personal_seek_point_1.is_some(),
                    tail.personal_seek_point_2.is_some(),
                    ai_global.seek_points.len(),
                    creation_order,
                )?,
                personal_seek_point_1: tail
                    .personal_seek_point_1
                    .as_ref()
                    .map(|point| convert_seek_point(point, 1111, topology, creation_order))
                    .transpose()?,
                personal_seek_point_2: tail
                    .personal_seek_point_2
                    .as_ref()
                    .map(|point| convert_seek_point(point, 2222, topology, creation_order))
                    .transpose()?,
                seek_center: ai_position(
                    tail.seek_center,
                    topology,
                    creation_order,
                    "local_ai.enemy.seek_center.sector",
                )?,
                actual_seek_point,
                seek_point_view_directions: tail.seek_point_view_directions.clone(),
                positions_of_beggars_to_control: ai_position_list(
                    &tail.positions_of_beggars_to_control,
                    topology,
                    creation_order,
                    "local_ai.enemy.positions_of_beggars_to_control.sector",
                )?,
                seek_flags: seek_flags(
                    tail.repeated_seek_flags,
                    creation_order,
                    "local_ai.enemy.seek_flags",
                )?,
                forced_next_battle_decision: decision(
                    tail.forced_next_battle_decision,
                    creation_order,
                )?,
                reset_battle_decision: tail.reset_battle_decision,
                synchronize_index: tail.synchronize_index,
                seen_dead_body: tail.seen_dead_body,
                seeking_charly: tail.seeking_charly,
                initial_view_cone: view_cone(tail.initial_view_cone, creation_order)?,
                company_number: tail.company_number,
                left_combat_neighbour: ai_handle(
                    entities.resolve_ai_element(tail.left_combat_neighbour)?,
                    ReferenceKind::Human,
                    creation_order,
                    "local_ai.enemy.left_combat_neighbour",
                )?,
                right_combat_neighbour: ai_handle(
                    entities.resolve_ai_element(tail.right_combat_neighbour)?,
                    ReferenceKind::Human,
                    creation_order,
                    "local_ai.enemy.right_combat_neighbour",
                )?,
                attentive: tail.attentive,
                will_be_attentive: tail.will_be_attentive,
                forced_attentive: tail.forced_attentive,
                guarded_pc: optional_pc_id(
                    entities.resolve_ai_element(tail.guarded_pc)?,
                    creation_order,
                    "local_ai.enemy.guarded_pc",
                )?,
                tower_guard: tail.tower_guard,
                combat_trainer: tail.combat_trainer,
                gather_position: ai_position(
                    tail.gather_position,
                    topology,
                    creation_order,
                    "local_ai.enemy.gather_position.sector",
                )?,
                gather_direction: tail.gather_direction,
                gather_position_instructed: tail.gather_position_instructed,
                officers_position: ai_position(
                    tail.officers_position,
                    topology,
                    creation_order,
                    "local_ai.enemy.officers_position.sector",
                )?,
                // Both fields are uninitialized by
                // `RHArtificialMalignity::RHArtificialMalignity`, then
                // serialized as complete four-byte enum storage. Preserve
                // their raw words until the one runtime branch in which the
                // Original treats the pair as live.
                previous_state: preserve_previous_enemy_state_word(tail.previous_state),
                previous_substate: preserve_previous_enemy_state_word(tail.previous_substate),
                reported_to_officer: tail.reported_to_officer,
                missed_soldier_timer: tail.missed_soldier_timer,
                old_money: tail.old_money,
                other_seen_money: ai_handle_list(
                    &tail.other_seen_money,
                    ReferenceKind::Object,
                    creation_order,
                    "local_ai.enemy.other_seen_money",
                    entities,
                )?,
                money_fight_enemies: ai_handle_list(
                    &tail.money_fight_enemies,
                    ReferenceKind::Npc,
                    creation_order,
                    "local_ai.enemy.money_fight_enemies",
                    entities,
                )?,
                money_fight_victims: ai_handle_list(
                    &tail.money_fight_victims,
                    ReferenceKind::Npc,
                    creation_order,
                    "local_ai.enemy.money_fight_victims",
                    entities,
                )?,
                archer_behind_me: ai_handle(
                    entities.resolve_ai_element(tail.archer_behind_me)?,
                    ReferenceKind::Npc,
                    creation_order,
                    "local_ai.enemy.archer_behind_me",
                )?,
                shield_bearer_before_me: ai_handle(
                    entities.resolve_ai_element(tail.shield_bearer_before_me)?,
                    ReferenceKind::Npc,
                    creation_order,
                    "local_ai.enemy.shield_bearer_before_me",
                )?,
                already_seen_bodies: ai_handle_list(
                    &tail.already_seen_bodies,
                    ReferenceKind::Human,
                    creation_order,
                    "local_ai.enemy.already_seen_bodies",
                    entities,
                )?,
                my_line_jump: resolve_saved_enemy_jump_line(
                    tail.jump_line,
                    saved.common.primary_target,
                    entity_id,
                    creation_order,
                    entities,
                    saved_human_geometries,
                    line_topology,
                    engine,
                    assets,
                )?
                .map(crate::jump_line::JumpLineIndex::get),
                shield_bearer_direction: tail.shield_bearer_direction,
                phalanx_aborted: tail.phalanx_aborted,
                changed_to_alert_path: tail.changed_to_alert_path,
                my_shooting_point,
                my_archery_sector,
                my_archery_sector_index: tail.archery_sector_index,
                my_archery_point_index: crate::sector::ArcheryPointIdx(tail.archery_point_index),
                my_archery_point_increment: tail.archery_point_increment,
                enemy_seen_below: tail.enemy_seen_below,
                enemy_had_this_elevation: tail.enemy_had_this_elevation,
                known_enemy_strikes: tail
                    .known_enemy_strike_commands
                    .map(|command| known_enemy_strike(command, creation_order))
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()?
                    .try_into()
                    .expect("fixed-size strike conversion preserves length"),
            })
        }
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

/// The macro byte cursor as the save stream carries it.
///
/// The stream only stores the cursor for an NPC that owns a patrol path, and
/// the restore rebuilds it against that NPC's current waypoint. A save taken
/// while the NPC had no patrol path leaves the cursor out entirely, so the
/// value the mission's own initialization put there survives the load — hence
/// [`Option`] rather than an empty stream.
#[derive(Clone, Debug)]
struct ConvertedMacroCursor {
    command: Vec<u8>,
    offset: usize,
    waypoint: Option<(PathId, u8)>,
}

fn convert_local_ai_common(
    saved: &LegacyLocalAiCommon,
    creation_order: u32,
    entities: &LegacyEntityFixups,
    topology: &LegacyPositionTopology,
    view_alert_status: AlertLevel,
    assets: &LevelAssets,
) -> Result<ConvertedLocalAiCommon, LegacyElementAdoptError> {
    let macro_cursor = convert_macro_command(saved, creation_order, &assets.hiking_paths)?;
    let (patrol_path, detached_patrol_path_status) =
        convert_patrol_path(&saved.path, creation_order, topology, &assets.hiking_paths)?;
    let saved_current_remark = remark(
        saved.current_remark,
        creation_order,
        "local_ai.current_remark",
    )?;
    let completes_saved_remark = saved_current_remark != Remark::TheSoundOfSilence;
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
        // Original clears the latch immediately before its synchronous
        // InformAIOnFinishedRemark call. The dedicated post-load plan owns
        // that callback and retains the saved flags separately.
        current_remark_flags: if completes_saved_remark {
            0
        } else {
            saved.current_remark_flags
        },
        current_state: ai_state(
            saved.current_state,
            creation_order,
            "local_ai.current_state",
        )?,
        old_state: preserve_old_ai_state(saved.old_state),
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
        patrol_path,
        detached_patrol_path_status,
        stop_before_end_of_path: saved.stop_before_end_of_path,
        use_max_norm_to_stop_before_end_of_path: saved.use_max_norm_to_stop_before_end_of_path,
        stop_before_end_of_path_distance: saved.stop_before_end_of_path_distance,
        macro_cursor,
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
        seek_position: ai_position(
            saved.seek_position,
            topology,
            creation_order,
            "local_ai.seek_position.sector",
        )?,
        alert_soldiers_point: ai_position(
            saved.alert_soldiers_point,
            topology,
            creation_order,
            "local_ai.alert_soldiers_point.sector",
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
        remaining_tequila_gulps: saved.remaining_tequila_gulps,
        friends_are_alerted: saved.friends_are_alerted,
        is_stay_at_home: saved.stay_at_home,
        locks_flag_field: AiLockFlags::from_bits(saved.locks_flag_field).ok_or(
            LegacyElementAdoptError::InvalidFlags {
                creation_order,
                field: "local_ai.locks_flag_field",
                value: u16::from(saved.locks_flag_field),
            },
        )?,
        was_busy: saved.was_busy,
        script_locked: saved.script_locked,
        remember_events: saved.remember_events,
        leave_house_number: saved.leave_house_number,
        last_hint_actuality: saved.last_hint_actuality,
        last_hint_subject: question(
            saved.last_hint_subject,
            creation_order,
            "local_ai.last_hint_subject",
        )?,
        my_door_index: legacy_door_index(
            saved.door_index,
            topology,
            creation_order,
            "local_ai.my_door",
        )?,
        looking_for_help_because_enemy_seen: saved.looking_for_help_because_enemy_seen,
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
        current_remark: Remark::TheSoundOfSilence,
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
                wants_to_talk,
                last_talk_partner,
                can_go_away,
            },
        ) => {
            apply_local_ai_common(&mut ai.base, common);
            ai.fleeing_seen_enemy_counter = fleeing_seen_enemy_counter;
            ai.beggar_dont_talk_counter = beggar_dont_talk_counter;
            ai.wants_to_talk = wants_to_talk;
            ai.last_talk_partner = last_talk_partner;
            ai.can_go_away = can_go_away;
        }
        (
            AiBrain::Enemy(ai),
            ConvertedLocalAi::Enemy {
                common,
                last_stimulus_dispatched_to_patrol,
                frame_when_missed_charly,
                heard_nets,
                frame_when_enemy_detected,
                fleeing_seen_enemy_counter,
                other_seen_ale,
                pc_gone_away_direction,
                detected_something_there,
                missed_pc,
                last_seek_direction_index,
                beggar_to_examine,
                pc_missed,
                search_charly_way,
                current_task_priority,
                minimal_task_priority,
                new_task_priority,
                number_of_different_checkpoints,
                delta_sorrow_level,
                missed_in_action,
                other_bodies_to_examine,
                beggars_to_control,
                thirsty,
                old_life_points,
                initial_life_points,
                list_them,
                old_odds,
                position_change_locked_for_test,
                ambush_point_array_reset,
                ambush_point_status,
                my_seek_points,
                personal_seek_point_1,
                personal_seek_point_2,
                seek_center,
                actual_seek_point,
                seek_point_view_directions,
                positions_of_beggars_to_control,
                seek_flags,
                forced_next_battle_decision,
                reset_battle_decision,
                synchronize_index,
                seen_dead_body,
                seeking_charly,
                initial_view_cone,
                company_number,
                left_combat_neighbour,
                right_combat_neighbour,
                attentive,
                will_be_attentive,
                forced_attentive,
                guarded_pc,
                tower_guard,
                combat_trainer,
                gather_position,
                gather_direction,
                gather_position_instructed,
                officers_position,
                previous_state,
                previous_substate,
                reported_to_officer,
                missed_soldier_timer,
                old_money,
                other_seen_money,
                money_fight_enemies,
                money_fight_victims,
                archer_behind_me,
                shield_bearer_before_me,
                already_seen_bodies,
                my_line_jump,
                shield_bearer_direction,
                phalanx_aborted,
                changed_to_alert_path,
                my_shooting_point,
                my_archery_sector,
                my_archery_sector_index,
                my_archery_point_index,
                my_archery_point_increment,
                enemy_seen_below,
                enemy_had_this_elevation,
                known_enemy_strikes,
            },
        ) => {
            apply_local_ai_common(&mut ai.base, common);
            // Original has one `mlistAlertedUs` member on the common AI
            // base. Runtime EnemyAi keeps the actively coordinated officer
            // group in its typed mirror, so restore that mirror from the
            // authoritative serialized list as part of adoption.
            ai.alerted_us = ai.base.list_alerted_us.clone();
            ai.last_stimulus_dispatched_to_patrol = Some(last_stimulus_dispatched_to_patrol);
            ai.frame_when_missed_charly = frame_when_missed_charly;
            ai.heard_nets = heard_nets;
            ai.base.frame_when_enemy_detected = frame_when_enemy_detected;
            ai.fleeing_seen_enemy_counter = fleeing_seen_enemy_counter;
            ai.other_seen_ale = other_seen_ale;
            ai.pc_gone_away_in_this_direction = pc_gone_away_direction;
            ai.detected_something_there = detected_something_there;
            ai.missed_pc = missed_pc;
            ai.last_seek_direction_index = last_seek_direction_index;
            ai.beggar_to_examine = beggar_to_examine;
            ai.pc_missed = pc_missed;
            ai.search_charly_way = search_charly_way;
            ai.current_task_priority = current_task_priority;
            ai.minimal_task_priority = minimal_task_priority;
            ai.new_task_priority = new_task_priority;
            ai.number_of_different_checkpoints = number_of_different_checkpoints;
            ai.base.delta_sorrow_level = delta_sorrow_level;
            ai.base.missed_in_action = missed_in_action;
            ai.other_bodies_to_examine = other_bodies_to_examine;
            ai.beggars_to_control = beggars_to_control;
            ai.thirsty = thirsty;
            ai.old_life_points = old_life_points;
            ai.initial_life_points = initial_life_points;
            ai.list_them = list_them;
            ai.old_odds = old_odds;
            ai.position_change_locked_for_test = position_change_locked_for_test;
            ai.ambush_point_array_reset = ambush_point_array_reset;
            ai.ambush_point_status = ambush_point_status;
            ai.my_seek_points = my_seek_points;
            ai.personal_seek_point_1 = personal_seek_point_1;
            ai.personal_seek_point_2 = personal_seek_point_2;
            ai.seek_center = seek_center;
            ai.actual_seek_point = actual_seek_point;
            ai.seek_point_view_directions = seek_point_view_directions;
            ai.positions_of_beggars_to_control = positions_of_beggars_to_control;
            ai.seek_flags = seek_flags;
            ai.forced_next_battle_decision = forced_next_battle_decision;
            ai.reset_battle_decision = reset_battle_decision;
            ai.base.synchronize_index = synchronize_index;
            ai.seen_dead_body = seen_dead_body;
            ai.seeking_charly = seeking_charly;
            ai.base.initial_view_cone = initial_view_cone;
            ai.company_number = company_number;
            ai.left_combat_neighbour = left_combat_neighbour;
            ai.right_combat_neighbour = right_combat_neighbour;
            ai.attentive = attentive;
            ai.will_be_attentive = will_be_attentive;
            ai.forced_attentive = forced_attentive;
            ai.guarded_pc = guarded_pc;
            ai.tower_guard = tower_guard;
            ai.combat_trainer = combat_trainer;
            ai.gather_position = gather_position;
            ai.gather_direction = gather_direction;
            ai.gather_position_instructed = gather_position_instructed;
            ai.officers_position = officers_position;
            ai.previous_state = previous_state;
            ai.previous_substate = previous_substate;
            ai.reported_to_officer = reported_to_officer;
            ai.missed_soldier_timer = missed_soldier_timer;
            ai.old_money = old_money;
            ai.other_seen_money = other_seen_money;
            ai.money_fight_enemies = money_fight_enemies;
            ai.money_fight_victims = money_fight_victims;
            ai.archer_behind_me = archer_behind_me;
            ai.shield_bearer_before_me = shield_bearer_before_me;
            ai.already_seen_bodies = already_seen_bodies;
            ai.my_line_jump = my_line_jump;
            ai.shield_bearer_direction = shield_bearer_direction;
            ai.phalanx_aborted = phalanx_aborted;
            ai.changed_to_alert_path = changed_to_alert_path;
            ai.my_shooting_point = my_shooting_point;
            ai.my_archery_sector = my_archery_sector;
            ai.my_archery_sector_index = my_archery_sector_index;
            ai.my_archery_point_index = my_archery_point_index;
            ai.my_archery_point_increment = my_archery_point_increment;
            ai.enemy_seen_below = enemy_seen_below;
            ai.enemy_had_this_elevation = enemy_had_this_elevation;
            let [strike_1, strike_2, strike_3] = known_enemy_strikes;
            ai.known_enemy_strike_1 = strike_1;
            ai.known_enemy_strike_2 = strike_2;
            ai.known_enemy_strike_3 = strike_3;
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
    ai.detached_patrol_path_status = saved.detached_patrol_path_status;
    ai.stop_before_end_of_path = saved.stop_before_end_of_path;
    ai.use_max_norm_to_stop_before_end_of_path = saved.use_max_norm_to_stop_before_end_of_path;
    ai.stop_before_end_of_path_distance = saved.stop_before_end_of_path_distance;
    if let Some(cursor) = saved.macro_cursor {
        ai.macro_command = cursor.command;
        ai.macro_command_offset = cursor.offset;
        ai.macro_command_waypoint = cursor.waypoint;
    }
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
    ai.seek_position = saved.seek_position;
    ai.alert_soldiers_point = saved.alert_soldiers_point;
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
    reset_loaded_ai_completion_ownership(ai);
    ai.likes_to_sit_around = saved.likes_to_sit_around;
    ai.special_action = saved.special_action;
    ai.remaining_tequila_gulps = saved.remaining_tequila_gulps;
    ai.friends_are_alerted = saved.friends_are_alerted;
    ai.is_stay_at_home = saved.is_stay_at_home;
    ai.locks_flag_field = saved.locks_flag_field;
    ai.was_busy = saved.was_busy;
    ai.script_locked = saved.script_locked;
    ai.remember_events = saved.remember_events;
    ai.leave_house_number = saved.leave_house_number;
    ai.last_hint_actuality = saved.last_hint_actuality;
    ai.last_hint_subject = saved.last_hint_subject;
    ai.my_door_index = saved.my_door_index;
    ai.looking_for_help_because_enemy_seen = saved.looking_for_help_because_enemy_seen;
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
    // Mission bootstrap requests a one-shot InitializePatrol, but an Original
    // save already contains its authoritative active/missed patrol ordering.
    // Loading does not call InitializePatrol again in the Original.
    ai.needs_patrol_reinit = false;
    ai.patrol_stopped = saved.patrol_stopped;
    ai.patrol_direction = saved.patrol_direction;
    ai.stimulus_queue = saved.stimulus_queue;
}

fn reset_loaded_ai_completion_ownership(ai: &mut AiController) {
    // The Original serializes the three completion latches above, but not its
    // static Think recursion depth. Consequently a loaded latch has no live
    // EndThink owner: the next StartThink clears it before doing any work.
    // Do not retain ownership left by the initialized mission being replaced.
    ai.completion_latch_inside_think = false;
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

fn convert_npc_leaf(
    payload: &LegacyElementPayload,
    entity_id: EntityId,
    creation_order: u32,
) -> Result<Option<ConvertedNpcLeaf>, LegacyElementAdoptError> {
    use crate::entity_id::EntityIdKind;
    match payload {
        LegacyElementPayload::ActorNpcSoldier(saved) => {
            if entity_id.kind() != EntityIdKind::Soldier {
                return Err(LegacyElementAdoptError::NpcLeafKindMismatch {
                    creation_order,
                    saved_kind: "Soldier",
                    runtime_kind: entity_id.kind(),
                });
            }
            Ok(Some(ConvertedNpcLeaf::Soldier {
                apple_smell: saved.leaf.apple_smell,
            }))
        }
        LegacyElementPayload::ActorNpcCivilian(saved) => {
            if entity_id.kind() != EntityIdKind::Civilian {
                return Err(LegacyElementAdoptError::NpcLeafKindMismatch {
                    creation_order,
                    saved_kind: "Civilian",
                    runtime_kind: entity_id.kind(),
                });
            }
            Ok(Some(ConvertedNpcLeaf::Civilian {
                current_scroll_set: saved.leaf.current_scroll_set,
            }))
        }
        _ => Ok(None),
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

fn motion_state(value: u32, creation_order: u32) -> Result<MotionState, LegacyElementAdoptError> {
    match value {
        0 => Ok(MotionState::Done),
        1 => Ok(MotionState::Start),
        2 => Ok(MotionState::InProgress),
        3 => Ok(MotionState::Terminated),
        4 => Ok(MotionState::Aborted),
        5 => Ok(MotionState::Error),
        value => {
            // RHElementActor::Hourglass overwrites mmotionState with the
            // result of Execute() before its first post-load read. Retail
            // Windows saves can therefore contain arbitrary dormant storage
            // here. Preserve initialized values for diagnostics, but
            // canonicalize an invalid word to the constructor state.
            tracing::debug!(
                creation_order,
                value,
                "canonicalizing dormant legacy actor motion state"
            );
            Ok(MotionState::Done)
        }
    }
}

fn finite(
    value: f32,
    creation_order: u32,
    field: &'static str,
) -> Result<(), LegacyElementAdoptError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(LegacyElementAdoptError::NonFinite {
            creation_order,
            field,
            value,
        })
    }
}

fn finite_point(
    point: LegacyPoint2,
    creation_order: u32,
    field: &'static str,
) -> Result<(), LegacyElementAdoptError> {
    finite(point.x, creation_order, field)?;
    finite(point.y, creation_order, field)
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

/// Preserve `mOldState` exactly, including indeterminate constructor bytes.
///
/// Unlike the neighboring enums, Original never initializes or reads this
/// member before `StartThink`; serialization nevertheless writes its complete
/// four-byte storage word.
fn preserve_old_ai_state(raw: i32) -> i32 {
    raw
}

/// Preserve the indeterminate `RHArtificialMalignity` previous-state pair.
///
/// Original initializes neither member in its constructor. The pair is
/// assigned atomically before the only branch that consumes it, so arbitrary
/// saved words are inert unless that branch is live.
fn preserve_previous_enemy_state_word(raw: i32) -> i32 {
    raw
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

fn question(
    value: i32,
    creation_order: u32,
    field: &'static str,
) -> Result<Question, LegacyElementAdoptError> {
    let raw = u32::try_from(value).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value: value as u32,
    })?;
    Question::try_from(raw).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field,
        value: raw,
    })
}

fn legacy_door_index(
    value: Option<i16>,
    topology: &LegacyPositionTopology,
    creation_order: u32,
    field: &'static str,
) -> Result<Option<u32>, LegacyElementAdoptError> {
    value
        .map(|index| {
            let slot =
                usize::try_from(index).map_err(|_| LegacyElementAdoptError::MissingGate {
                    creation_order,
                    field,
                    index,
                    count: topology.doors.len(),
                })?;
            topology.doors.get(slot).map(|door| door.0).ok_or(
                LegacyElementAdoptError::MissingGate {
                    creation_order,
                    field,
                    index,
                    count: topology.doors.len(),
                },
            )
        })
        .transpose()
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
            let slot = usize::from(index);
            let Some(sector) = topology.sectors.get(slot) else {
                return Err(LegacyElementAdoptError::MissingSector {
                    creation_order,
                    field,
                    index,
                    count: topology.sectors.len(),
                });
            };
            let public = sector.ok_or(LegacyElementAdoptError::MissingSector {
                creation_order,
                field,
                index,
                count: topology.sectors.len(),
            })?;
            // Original serialized an RHSector* as its sparse array slot. The
            // retained topology resolves that pointer to both its public
            // number and exact runtime object. Keeping only the public half
            // makes a later gate search mix an exact actor sector with a
            // number-only target, which the identity-aware graph correctly
            // rejects. Synthetic/older test topologies without the paired
            // identity retain their legacy number-only representation.
            Ok(topology
                .sector_indices
                .get(slot)
                .copied()
                .flatten()
                .map_or(public, |arena| public.with_arena_index(arena)))
        })
        .transpose()
}

#[cfg(test)]
pub(crate) fn adopt_position_sector_for_test(
    sectors: Vec<Option<SectorHandle>>,
    sector_indices: Vec<Option<crate::fast_find_grid::SectorIndex>>,
    saved_slot: u16,
) -> Option<SectorHandle> {
    let slot_count = sectors.len();
    let topology = LegacyPositionTopology {
        sectors,
        sector_indices,
        sector_doors: vec![None; slot_count],
        doors: Vec::new(),
        projection_areas: Vec::new(),
        sight_obstacles: Vec::new(),
    };
    sector(Some(saved_slot), &topology, 0, "initial_position.sector")
        .expect("test saved sector slot resolves")
}

fn seek_sector(
    value: Option<u16>,
    topology: &LegacyPositionTopology,
    creation_order: u32,
    field: &'static str,
) -> Result<Option<ActorSeekSector>, LegacyElementAdoptError> {
    value
        .map(|index| {
            let slot = usize::from(index);
            if let Some(Some(sector)) = topology.sectors.get(slot) {
                return Ok(ActorSeekSector::Position(*sector));
            }
            if let Some(Some(door)) = topology.sector_doors.get(slot) {
                return Ok(ActorSeekSector::Door(*door));
            }
            Err(LegacyElementAdoptError::MissingSector {
                creation_order,
                field,
                index,
                count: topology.sectors.len(),
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

fn ai_position_list(
    saved: &[LegacyAiPosition],
    topology: &LegacyPositionTopology,
    creation_order: u32,
    field: &'static str,
) -> Result<Vec<Position>, LegacyElementAdoptError> {
    saved
        .iter()
        .copied()
        .map(|position| ai_position(position, topology, creation_order, field))
        .collect()
}

fn ambush_status(
    value: i32,
    creation_order: u32,
) -> Result<crate::ai_enemy::AmbushPointStatus, LegacyElementAdoptError> {
    match raw_i32(value, creation_order, "local_ai.enemy.ambush_point_status")? {
        0 => Ok(crate::ai_enemy::AmbushPointStatus::Far),
        1 => Ok(crate::ai_enemy::AmbushPointStatus::Near),
        2 => Ok(crate::ai_enemy::AmbushPointStatus::Checked),
        value => Err(LegacyElementAdoptError::UnknownEnum {
            creation_order,
            field: "local_ai.enemy.ambush_point_status",
            value,
        }),
    }
}

fn convert_ambush_point_statuses(
    initialized: &[crate::ai_enemy::AmbushPointStatus],
    saved: &[i32],
    creation_order: u32,
) -> Result<Vec<crate::ai_enemy::AmbushPointStatus>, LegacyElementAdoptError> {
    // RHArtificialMalignity::Serialize does not clear the per-NPC
    // marrayAmbushPointStatus initialized by InitAI. It appends every saved
    // status after deleting the separate shared marrayAmbushPoints topology.
    let mut statuses = initialized.to_vec();
    statuses.extend(
        saved
            .iter()
            .copied()
            .map(|value| ambush_status(value, creation_order))
            .collect::<Result<Vec<_>, _>>()?,
    );
    Ok(statuses)
}

fn clear_ambush_points_on_enemy_load(ai_global: &mut AiGlobalState) {
    // Original load behavior, including its likely typo, is authoritative:
    // RHArtificialMalignity::Serialize calls marrayAmbushPoints.Delete()
    // (not marrayAmbushPointStatus.Delete()) for every enemy AI it reads.
    ai_global.ambush_points.clear();
}

fn decision(
    value: i32,
    creation_order: u32,
) -> Result<crate::ai::Decision, LegacyElementAdoptError> {
    let value = raw_i32(
        value,
        creation_order,
        "local_ai.enemy.forced_next_battle_decision",
    )?;
    crate::ai::Decision::try_from(value).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field: "local_ai.enemy.forced_next_battle_decision",
        value,
    })
}

fn view_cone(
    value: i32,
    creation_order: u32,
) -> Result<crate::ai::ViewCone, LegacyElementAdoptError> {
    use crate::ai::ViewCone;
    let value = raw_i32(value, creation_order, "local_ai.enemy.initial_view_cone")?;
    let result = match value {
        0 => ViewCone::Commandoslike,
        1 => ViewCone::Patrol,
        2 => ViewCone::QuickSearch,
        3 => ViewCone::GetOverview,
        4 => ViewCone::QuickOverview,
        5 => ViewCone::SlowOverview,
        6 => ViewCone::GattlingOverview,
        7 => ViewCone::LookDown,
        8 => ViewCone::LookTo,
        9 => ViewCone::LookToOrCommandoslikeDependingOnIq,
        10 => ViewCone::LookForward,
        11 => ViewCone::Focus,
        12 => ViewCone::GattlingFocus,
        13 => ViewCone::Idle,
        14 => ViewCone::Slow,
        15 => ViewCone::LongRange,
        16 => ViewCone::Sniper,
        17 => ViewCone::SceneOfTheCrime,
        18 => ViewCone::Valium,
        value => {
            return Err(LegacyElementAdoptError::UnknownEnum {
                creation_order,
                field: "local_ai.enemy.initial_view_cone",
                value,
            });
        }
    };
    Ok(result)
}

fn seek_flags(
    value: u16,
    creation_order: u32,
    field: &'static str,
) -> Result<crate::ai_enemy::SeekFlags, LegacyElementAdoptError> {
    crate::ai_enemy::SeekFlags::from_bits(value).ok_or(LegacyElementAdoptError::InvalidFlags {
        creation_order,
        field,
        value,
    })
}

fn convert_seek_point(
    saved: &super::payload_ai::LegacySeekPoint,
    id: u16,
    topology: &LegacyPositionTopology,
    creation_order: u32,
) -> Result<crate::ai::SeekPoint, LegacyElementAdoptError> {
    Ok(crate::ai::SeekPoint {
        position: Position {
            x: saved.position_x,
            y: saved.position_y,
            sector: sector(
                saved.position_sector.0,
                topology,
                creation_order,
                "local_ai.enemy.personal_seek_point.sector",
            )?,
            level: saved.position_level,
        },
        // SerializeAllData writes these members a second time at the end;
        // the second values are the final state after Original loads.
        frame_when_full_interest: saved.repeated_frame_when_fully_interesting,
        directions: saved.directions.clone(),
        last_calculated_interest: saved.repeated_last_calculated_interest,
        locked: saved.repeated_locked,
        id,
    })
}

fn validate_seek_point_id(
    value: u32,
    has_personal_1: bool,
    has_personal_2: bool,
    global_count: usize,
    creation_order: u32,
    field: &'static str,
) -> Result<u16, LegacyElementAdoptError> {
    match value {
        1111 if has_personal_1 => Ok(1111),
        2222 if has_personal_2 => Ok(2222),
        1111 | 2222 => Err(LegacyElementAdoptError::InvalidAiIndex {
            creation_order,
            field,
            index: value,
            count: global_count,
        }),
        value if usize::try_from(value).is_ok_and(|index| index < global_count) => {
            u16::try_from(value).map_err(|_| LegacyElementAdoptError::InvalidAiIndex {
                creation_order,
                field,
                index: value,
                count: global_count,
            })
        }
        value => Err(LegacyElementAdoptError::InvalidAiIndex {
            creation_order,
            field,
            index: value,
            count: global_count,
        }),
    }
}

fn convert_seek_point_ids(
    values: &[u32],
    has_personal_1: bool,
    has_personal_2: bool,
    global_count: usize,
    creation_order: u32,
) -> Result<Vec<u16>, LegacyElementAdoptError> {
    values
        .iter()
        .copied()
        .map(|value| {
            validate_seek_point_id(
                value,
                has_personal_1,
                has_personal_2,
                global_count,
                creation_order,
                "local_ai.enemy.seek_point_ids",
            )
        })
        .collect()
}

fn convert_actual_seek_point(
    value: u32,
    has_personal_1: bool,
    has_personal_2: bool,
    global_count: usize,
    creation_order: u32,
) -> Result<Option<u16>, LegacyElementAdoptError> {
    if value == 6666 {
        return Ok(None);
    }
    validate_seek_point_id(
        value,
        has_personal_1,
        has_personal_2,
        global_count,
        creation_order,
        "local_ai.enemy.actual_seek_point",
    )
    .map(Some)
}

fn optional_pc_id(
    entity_id: Option<EntityId>,
    creation_order: u32,
    field: &'static str,
) -> Result<Option<crate::entity_id::PcId>, LegacyElementAdoptError> {
    match checked_reference(entity_id, ReferenceKind::Human, creation_order, field)? {
        None => Ok(None),
        Some(EntityId::Pc(id)) => Ok(Some(id)),
        Some(entity_id) => Err(LegacyElementAdoptError::WrongReferenceKind {
            creation_order,
            field,
            entity_id,
            actual: entity_id.kind(),
            expected: "PC",
        }),
    }
}

fn convert_archery_refs(
    saved: &super::payload_ai::LegacyEnemyAiTail,
    ai_global: &AiGlobalState,
    creation_order: u32,
) -> Result<(Option<(u16, u16)>, Option<u16>), LegacyElementAdoptError> {
    let shooting_point = saved
        .shooting_point
        .map(|reference| {
            let sector = ai_global
                .archery_sectors
                .get(usize::from(reference.sector_index))
                .ok_or(LegacyElementAdoptError::InvalidAiIndex {
                    creation_order,
                    field: "local_ai.enemy.shooting_point.sector",
                    index: u32::from(reference.sector_index),
                    count: ai_global.archery_sectors.len(),
                })?;
            if usize::from(reference.point_index) >= sector.points.len() {
                return Err(LegacyElementAdoptError::InvalidAiIndex {
                    creation_order,
                    field: "local_ai.enemy.shooting_point.point",
                    index: u32::from(reference.point_index),
                    count: sector.points.len(),
                });
            }
            Ok((reference.sector_index, reference.point_index))
        })
        .transpose()?;
    let archery_sector = saved
        .archery_sector
        .map(|index| {
            if usize::from(index) >= ai_global.archery_sectors.len() {
                return Err(LegacyElementAdoptError::InvalidAiIndex {
                    creation_order,
                    field: "local_ai.enemy.archery_sector",
                    index: u32::from(index),
                    count: ai_global.archery_sectors.len(),
                });
            }
            Ok(index)
        })
        .transpose()?;
    Ok((shooting_point, archery_sector))
}

fn known_enemy_strike(
    value: i32,
    creation_order: u32,
) -> Result<Option<crate::weapons::SwordStrike>, LegacyElementAdoptError> {
    use crate::{element::Command, weapons::SwordStrike};
    let command = Command::try_from(value).map_err(|_| LegacyElementAdoptError::UnknownEnum {
        creation_order,
        field: "local_ai.enemy.known_enemy_strike",
        value: value as u32,
    })?;
    let strike = match command {
        Command::Null => None,
        Command::SwordstrikeThrustA => Some(SwordStrike::A),
        Command::SwordstrikeThrustB => Some(SwordStrike::B),
        Command::SwordstrikeThrustC => Some(SwordStrike::C),
        Command::SwordstrikeThrustD => Some(SwordStrike::D),
        Command::SwordstrikeThrustE => Some(SwordStrike::E),
        Command::SwordstrikeThrustF => Some(SwordStrike::F),
        Command::SwordstrikeThrustG => Some(SwordStrike::G),
        Command::SwordstrikeThrustH => Some(SwordStrike::H),
        Command::SwordstrikeThrustI => Some(SwordStrike::I),
        _ => {
            return Err(LegacyElementAdoptError::UnknownEnum {
                creation_order,
                field: "local_ai.enemy.known_enemy_strike",
                value: value as u32,
            });
        }
    };
    Ok(strike)
}

fn convert_patrol_path(
    saved: &LegacyAiPathStatus,
    creation_order: u32,
    topology: &LegacyPositionTopology,
    hiking_paths: &[crate::level_data::RawHikingPath],
) -> Result<(Option<PatrolPath>, DetachedPatrolPathStatus), LegacyElementAdoptError> {
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
    let Some(raw_path_id) = saved.hiking_path_index else {
        return Ok((
            None,
            DetachedPatrolPathStatus {
                hiking_path_index: None,
                current_waypoint_index: saved.current_waypoint_index,
                last_waypoint_index: saved.last_waypoint_index,
                forward: saved.forward_movement,
                history,
            },
        ));
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
    if usize::from(saved.current_waypoint_index) >= authored.waypoints.len() {
        return Err(LegacyElementAdoptError::MissingWaypoint {
            creation_order,
            path: raw_path_id,
            field: "current_waypoint_index",
            waypoint: saved.current_waypoint_index,
            count: authored.waypoints.len(),
        });
    }
    // RHPath::SerializeStatus restores mubLastWaypointIndex verbatim and
    // never indexes mpHikingPath with it. The value is historical state used
    // by patrol synchronization comparisons, so it may legitimately be
    // outside the size of the currently authored path.
    Ok((
        Some(PatrolPath {
            hiking_path_index: path_id,
            current_waypoint_index: saved.current_waypoint_index,
            last_waypoint_index: saved.last_waypoint_index,
            forward: saved.forward_movement,
            size,
            history,
        }),
        DetachedPatrolPathStatus::default(),
    ))
}

fn convert_macro_command(
    saved: &LegacyLocalAiCommon,
    creation_order: u32,
    hiking_paths: &[crate::level_data::RawHikingPath],
) -> Result<Option<ConvertedMacroCursor>, LegacyElementAdoptError> {
    if !saved.has_patrol_path {
        if saved.macro_in_progress {
            return Err(LegacyElementAdoptError::MacroWithoutPatrolPath { creation_order });
        }
        // No cursor in the stream: keep the one the mission left behind. It is
        // dormant while the NPC stands on a post, but the NPC can return to its
        // route and stand on the very waypoint the stale cursor points into,
        // and the cursor becomes observable again at that moment.
        // TODO(parity): loaded-save trace segments must retain that dormant
        // cursor's command/waypoint identity from the live Original process.
        // The v48 stream contains no cursor or offset in this branch, so a
        // standalone mission reconstruction cannot recover it from the save.
        return Ok(None);
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
    let (macro_command, macro_command_offset) = convert_waypoint_macro_cursor(
        &waypoint.command,
        offset,
        saved.remaining_macro_bytes,
        saved.macro_in_progress,
        creation_order,
    )?;
    // The Original rebuilds the cursor from the current waypoint's block, so a
    // restored cursor always belongs to that waypoint.
    let waypoint = (!macro_command.is_empty()).then(|| {
        let path_id = PathId::new(raw_path_id)
            .expect("Legacy decoder already maps the 0xffff no-path sentinel to None");
        (path_id, saved.path.current_waypoint_index)
    });
    Ok(Some(ConvertedMacroCursor {
        command: macro_command,
        offset: macro_command_offset,
        waypoint,
    }))
}

fn convert_waypoint_macro_cursor(
    command: &WaypointCommand,
    offset: usize,
    remaining: u16,
    macro_in_progress: bool,
    creation_order: u32,
) -> Result<(Vec<u8>, usize), LegacyElementAdoptError> {
    // ExecuteNextMacroCommand tests the signed remaining-byte count before
    // dereferencing mpubMacroCommand. A zero-byte macro therefore has a
    // dormant pointer even if mbMacroInProgress still contains true.
    let inactive = !macro_in_progress || remaining == 0;
    let macro_data = match command {
        WaypointCommand::Macro(data) => data,
        WaypointCommand::None if inactive => return Ok((Vec::new(), offset)),
        WaypointCommand::Script(_) if inactive => return Ok((Vec::new(), offset)),
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

    // RHArtificialIntelligence::BreakMacro only clears mbMacroInProgress; it
    // deliberately leaves both the byte counter and mpubMacroCommand alone.
    // SerializeThisAI also writes the pointer subtraction whenever a patrol
    // path exists, so an interrupted macro can retain a non-zero count and an
    // indeterminate/out-of-waypoint cursor. Original gates every later resume
    // on mbMacroInProgress and never dereferences those dormant fields.
    // Preserve their diagnostic storage exactly, but keep strict bounds for
    // every cursor whose macro is still live.
    if inactive {
        return Ok((macro_data.clone(), offset));
    }

    let end = offset
        .checked_add(usize::from(remaining))
        .filter(|&end| offset <= macro_data.len() && end <= macro_data.len())
        .ok_or(LegacyElementAdoptError::InvalidMacroCursor {
            creation_order,
            offset,
            remaining,
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
    // Legacy C++ payloads only contain the shipped 0..=Off ordinals. Rust's
    // appended deterministic Distraction variant is never valid input here.
    if value > NoiseType::Off as u32 {
        return Err(LegacyElementAdoptError::UnknownEnum {
            creation_order,
            field,
            value,
        });
    }
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
    let (stimulus_type, info) = match stimulus_type(
        saved.stimulus_type,
        creation_order,
        "local_ai.stimulus_queue.stimulus_type",
    ) {
        Ok(stimulus_type) => (stimulus_type, info),
        Err(_) if is_uninitialized_default_stimulus(saved) => {
            // `RHStimulus::RHStimulus` initialized exactly these four
            // surrounding fields but omitted `mType`. If such a default
            // stimulus was delayed, its arbitrary enum storage was serialized
            // as an active queue entry. Original dispatch gives an unknown
            // type the same default path as `NO_EVENT`; retain the raw word in
            // the payload rather than inventing a low-byte event.
            (
                StimulusType::NoEvent,
                StimulusInfo::LegacyInvalidType(saved.stimulus_type),
            )
        }
        Err(error) => return Err(error),
    };
    Ok(Stimulus {
        stimulus_type,
        info,
        owner: element_handle(
            entities.resolve_element(saved.owner)?,
            ReferenceKind::Npc,
            creation_order,
            "local_ai.stimulus_queue.owner",
        )?,
        to_whole_patrol: saved.to_whole_patrol,
        self_origin: crate::ai::SelfStimulusOrigin::Ordinary,
    })
}

fn is_uninitialized_default_stimulus(saved: &LegacyStimulus) -> bool {
    saved.info_type == 0
        && matches!(saved.info, LegacyStimulusInfo::None)
        && saved.owner.0.is_none()
        && !saved.to_whole_patrol
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
    fn legacy_ai_load_drops_initialized_mission_completion_ownership() {
        let mut ai = AiController {
            couldnt_reachpoint: true,
            already_on_point: true,
            already_turned: true,
            completion_latch_inside_think: true,
            ..AiController::default()
        };

        reset_loaded_ai_completion_ownership(&mut ai);

        assert!(!ai.completion_latch_inside_think);
        assert!(ai.couldnt_reachpoint);
        assert!(ai.already_on_point);
        assert!(ai.already_turned);
    }

    #[test]
    fn sprite_adoption_preserves_original_profile_switch_before_position_restore() {
        use crate::position_interface::Direction;
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

        let mut element = crate::element::ElementData::default();
        let mut primary_conversion = vec![UNMAPPED; NONANIMATION_END];
        primary_conversion[OrderType::RunningUpright as usize] = 224;
        let mut alternate_conversion = vec![UNMAPPED; NONANIMATION_END];
        alternate_conversion[OrderType::RunningUpright as usize] = 1_696;
        let mut primary_scripts = vec![SpriteScript::default(); 240];
        primary_scripts[234].frame_ids = vec![0; 8];
        let alternate_scripts = vec![SpriteScript::default(); 1_712];
        element.sprite.scripts = std::sync::Arc::new(primary_scripts);
        element.sprite.alternate_scripts = Some(std::sync::Arc::new(alternate_scripts));
        element.sprite.conversion = std::sync::Arc::new(primary_conversion);
        element.sprite.alternate_conversion = Some(std::sync::Arc::new(alternate_conversion));
        element.sprite.use_alternate_profile = true;
        element
            .sprite
            .position_iface
            .set_direction_instantly(Direction::from_raw(10));

        let mut saved_position = element.sprite.position_iface.v48_serialized_state();
        saved_position.direction = Direction::from_raw(6);
        saved_position.direction_goal = Direction::from_raw(6);
        let converted = ConvertedElementBase {
            outline_colors: [0; 5],
            current_outline: OutlineColorName::Default,
            outline_width: 0,
            custom_minimap_dot: 0,
            active: true,
            position_map_delayed: false,
            delayed_map_position: MapPoint::new(0.0, 0.0),
            position_delayed: false,
            delayed_position: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0),
            in_honolulu: false,
            index_in_elements_list: 113,
            blipped: false,
            unreachable: false,
            sprite: ConvertedSprite {
                current_row: 230,
                current_frame: 7,
                frame_count: 0,
                flight_frame_countdown: 0,
                current_height: 0,
                current_width: 0,
                last_action: OrderType::RunningUpright,
                alternate_profile: false,
                masked: false,
                behind_display_order_reference: false,
                display_order_reference: None,
                action_done_frame: 0,
                action_done_counter: 0,
                last_sound_id: 0,
                last_processed_order_id: 0,
                animation_replacements: Vec::new(),
                position: saved_position,
            },
        };

        apply_element_base(&mut element, converted);

        assert!(!element.sprite.use_alternate_profile);
        assert_eq!(
            element.sprite.row_for_action(OrderType::RunningUpright),
            Some(224),
            "saved primary-profile selection must replace the candidate's active alternate table"
        );
        assert_eq!(element.sprite.position_iface.get_direction().as_u8(), 6);
        assert_eq!(element.sprite.current_row, 234);
        assert_eq!(element.sprite.current_frame, 7);
    }

    #[test]
    fn actor_seek_sector_preserves_a_saved_door_sector_identity() {
        let door = crate::position_interface::DoorHandle(9);
        let topology = LegacyPositionTopology {
            sectors: vec![SectorHandle::new(3), None],
            sector_indices: vec![None; 2],
            sector_doors: vec![None, Some(door)],
            doors: vec![door],
            projection_areas: Vec::new(),
            sight_obstacles: Vec::new(),
        };

        assert_eq!(
            seek_sector(Some(1), &topology, 326, "seek_sector").unwrap(),
            Some(ActorSeekSector::Door(door))
        );
        assert_eq!(
            seek_sector(Some(0), &topology, 326, "seek_sector").unwrap(),
            Some(ActorSeekSector::Position(SectorHandle::new(3).unwrap()))
        );
    }

    #[test]
    fn saved_position_sector_uses_retained_exact_runtime_identity() {
        let arena = crate::fast_find_grid::SectorIndex::new(95).unwrap();
        let exact = LegacyPositionTopology {
            sectors: vec![SectorHandle::new(249)],
            sector_indices: vec![Some(arena)],
            sector_doors: vec![None],
            doors: Vec::new(),
            projection_areas: Vec::new(),
            sight_obstacles: Vec::new(),
        };
        let number_only = LegacyPositionTopology {
            sector_indices: vec![None],
            ..exact.clone()
        };

        assert_eq!(
            sector(Some(0), &exact, 109, "initial_position.sector").unwrap(),
            SectorHandle::new(249).map(|sector| sector.with_arena_index(arena))
        );
        assert_eq!(
            sector(Some(0), &number_only, 109, "initial_position.sector").unwrap(),
            SectorHandle::new(249),
            "a topology that genuinely lacks retained identity stays number-only"
        );
    }

    fn sample_npc_view() -> LegacyNpcView {
        LegacyNpcView {
            leaning: true,
            leaning_padding: [0; 3],
            alert_status: 2,
            status: 6,
            transitioning: true,
            alpha: 123,
            half_angle: 0.11,
            angle_iterator: 0.12,
            angle_iterator_step: 0.13,
            angle_step: 0.14,
            angle: 0.15,
            half_aperture: 0.21,
            real_half_aperture: 0.22,
            half_aperture_cosine: 0.23,
            future_half_aperture: 0.24,
            half_aperture_step: 0.25,
            half_aperture_changes: true,
            half_aperture_padding: [0; 3],
            crazy_iterator: 0.31,
            crazy_iterator_step: 0.32,
            color: 17,
            color_padding: [0; 3],
            crazy_half_aperture: 0.33,
            direction: LegacyPoint2 { x: 0.41, y: 0.42 },
            left: LegacyPoint2 { x: 0.43, y: 0.44 },
            right: LegacyPoint2 { x: 0.45, y: 0.46 },
            stare: LegacyPoint2 { x: 0.47, y: 0.48 },
            raw_mobile_target_pointer: super::super::payload_base::LegacyOpaquePointer32(0),
            radius_goal: 501,
            radius: 502,
            radius_reduction: 503,
            radius_step: 504,
            long_range: 1.5,
            real_radius: 505,
            real_radius_padding: [0; 2],
            drunkenness: [0.51, 0.52, 0.53, 0.54],
            sniper: true,
            sniper_padding: [0; 3],
        }
    }

    #[test]
    fn npc_view_adoption_preserves_complete_serialized_continuation() {
        let converted = convert_npc_view(&sample_npc_view(), None, 31).unwrap();
        assert!(
            converted.leaning,
            "legacy adoption must preserve serialized bLeanOut independently of posture"
        );
        assert_eq!(converted.angle_iterator, 0.12);
        assert_eq!(converted.angle_iterator_step, 0.13);
        assert_eq!(converted.half_aperture_cosine, 0.23);
        assert_eq!(converted.future_half_aperture, 0.24);
        assert_eq!(converted.half_aperture_step, 0.25);
        assert!(converted.half_aperture_changes);
        assert_eq!(converted.crazy_angle_iterator, 0.31);
        assert_eq!(converted.crazy_angle_iterator_step, 0.32);
        assert_eq!(converted.crazy_color_iterator, 17);
        assert_eq!(converted.crazy_half_angle_range, 0.33);
        assert_eq!(converted.left_side, [0.43, 0.44]);
        assert_eq!(converted.right_side, [0.45, 0.46]);
        assert_eq!(converted.radius_reduction_permil, 503);
        assert!(converted.sniper);
    }

    #[test]
    fn npc_view_adoption_rejects_non_finite_continuation() {
        let mut view = sample_npc_view();
        view.crazy_iterator_step = f32::NAN;
        assert!(matches!(
            convert_npc_view(&view, None, 31),
            Err(LegacyElementAdoptError::NonFinite {
                field: "view.crazy_iterator_step",
                ..
            })
        ));
    }

    #[test]
    fn attached_scroll_requires_exact_scroll_kind() {
        let target = EntityId::new(7, crate::entity_id::EntityIdKind::Target);
        assert!(matches!(
            checked_reference(Some(target), ReferenceKind::Scroll, 31, "attached_scroll"),
            Err(LegacyElementAdoptError::WrongReferenceKind { .. })
        ));
        let scroll = EntityId::new(8, crate::entity_id::EntityIdKind::Scroll);
        assert_eq!(
            checked_reference(Some(scroll), ReferenceKind::Scroll, 31, "attached_scroll").unwrap(),
            Some(scroll)
        );
    }

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
    fn position_adoption_restores_the_original_single_posture_source() {
        let mut element = crate::element::ElementData {
            posture: crate::element::Posture::Spy,
            ..Default::default()
        };
        let mut position = element.sprite.position_iface.v48_serialized_state();
        position.posture = crate::element::Posture::LeaningOut;
        position.old_posture = crate::element::Posture::Upright;

        restore_position_and_gameplay_posture(&mut element, position);

        assert_eq!(element.posture, crate::element::Posture::LeaningOut);
        let restored = element.sprite.position_iface.v48_serialized_state();
        assert_eq!(restored.posture, crate::element::Posture::LeaningOut);
        assert_eq!(restored.old_posture, crate::element::Posture::Upright);
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
            sectors: vec![SectorHandle::new(0), SectorHandle::new(1)],
            sector_indices: vec![None; 2],
            sector_doors: vec![None; 2],
            doors: Vec::new(),
            projection_areas: Vec::new(),
            sight_obstacles: Vec::new(),
        };

        let restored = convert_patrol_path(&saved, 31, &topology, &paths)
            .unwrap()
            .0
            .unwrap();
        assert_eq!(restored.hiking_path_index.get(), 0);
        assert_eq!(restored.current_waypoint_index, 1);
        assert!(!restored.forward);
        assert_eq!(restored.history[0].position.sector.unwrap().get(), 1);
        assert_eq!(restored.history[0].distance, 4);
    }

    #[test]
    fn local_ai_path_preserves_out_of_range_historical_last_waypoint() {
        let saved = LegacyAiPathStatus {
            current_waypoint_index: 0,
            last_waypoint_index: 5,
            forward_movement: true,
            hiking_path_index: Some(0),
            history: Vec::new(),
        };
        let paths = vec![crate::level_data::RawHikingPath {
            waypoints: vec![crate::level_data::RawWaypoint {
                x: 0,
                y: 0,
                sector: 0,
                level: 0,
                command: WaypointCommand::None,
            }],
        }];
        let topology = LegacyPositionTopology {
            sectors: vec![SectorHandle::new(0)],
            sector_indices: vec![None],
            sector_doors: vec![None],
            doors: Vec::new(),
            projection_areas: Vec::new(),
            sight_obstacles: Vec::new(),
        };

        let restored = convert_patrol_path(&saved, 145, &topology, &paths)
            .unwrap()
            .0
            .unwrap();

        assert_eq!(restored.current_waypoint_index, 0);
        assert_eq!(restored.last_waypoint_index, 5);
        assert_eq!(restored.size, 1);
    }

    #[test]
    fn inactive_macro_cursor_preserves_indeterminate_pointer_difference() {
        let command = WaypointCommand::Macro(vec![0; 26]);
        let (bytes, offset) =
            convert_waypoint_macro_cursor(&command, 12_304, 0, false, 89).unwrap();
        assert_eq!(bytes.len(), 26);
        assert_eq!(offset, 12_304);

        let (bytes, offset) =
            convert_waypoint_macro_cursor(&command, 12_304, 1, false, 89).unwrap();
        assert_eq!(bytes.len(), 26);
        assert_eq!(offset, 12_304);
        let (bytes, offset) = convert_waypoint_macro_cursor(&command, 12_304, 0, true, 89).unwrap();
        assert_eq!(bytes.len(), 26);
        assert_eq!(offset, 12_304);
        assert!(matches!(
            convert_waypoint_macro_cursor(&command, 12_304, 1, true, 89),
            Err(LegacyElementAdoptError::InvalidMacroCursor {
                offset: 12_304,
                remaining: 1,
                length: 26,
                ..
            })
        ));
    }

    #[test]
    fn soldier_seek_point_ids_validate_global_and_personal_domains() {
        assert_eq!(
            convert_seek_point_ids(&[0, 2, 1111, 2222], true, true, 3, 31).unwrap(),
            vec![0, 2, 1111, 2222]
        );
        assert!(convert_seek_point_ids(&[3], false, false, 3, 31).is_err());
        assert!(convert_seek_point_ids(&[1111], false, false, 3, 31).is_err());
        assert_eq!(
            convert_actual_seek_point(6666, false, false, 0, 31).unwrap(),
            None
        );
    }

    #[test]
    fn linux_v48_enemy_load_appends_saved_ambush_statuses_to_initialized_list() {
        use crate::ai_enemy::AmbushPointStatus::{Checked, Far, Near};

        assert_eq!(
            convert_ambush_point_statuses(&[Far, Far], &[], 88).unwrap(),
            vec![Far, Far],
            "a zero-length saved list leaves constructor-initialized statuses intact"
        );
        assert_eq!(
            convert_ambush_point_statuses(&[Far, Far], &[0, 1, 2], 88).unwrap(),
            vec![Far, Far, Far, Near, Checked],
        );
        assert!(matches!(
            convert_ambush_point_statuses(&[Far], &[3], 88),
            Err(LegacyElementAdoptError::UnknownEnum {
                field: "local_ai.enemy.ambush_point_status",
                value: 3,
                ..
            })
        ));
    }

    #[test]
    fn linux_v48_enemy_load_deletes_initialized_global_ambush_points() {
        let mut global = AiGlobalState::default();
        global.ambush_points.push(crate::ai::AmbushPoint {
            position: Position::default(),
            direction: 7,
            position_3d: crate::coordinates::WorldPoint3D::new(1.0, 2.0, 3.0),
            id: 4,
        });

        clear_ambush_points_on_enemy_load(&mut global);

        assert!(global.ambush_points.is_empty());
    }

    #[test]
    fn soldier_known_strikes_accept_only_null_and_thrust_commands() {
        use crate::{element::Command, weapons::SwordStrike};
        assert_eq!(known_enemy_strike(Command::Null as i32, 31).unwrap(), None);
        assert_eq!(
            known_enemy_strike(Command::SwordstrikeThrustI as i32, 31).unwrap(),
            Some(SwordStrike::I)
        );
        assert!(known_enemy_strike(Command::Move as i32, 31).is_err());
    }

    #[test]
    fn linux_v48_old_ai_state_preserves_indeterminate_storage_bits() {
        assert_eq!(preserve_old_ai_state(0x0096_0003), 0x0096_0003);
        assert_eq!(preserve_old_ai_state(0x5a3b_010e), 0x5a3b_010e);
    }

    #[test]
    fn linux_v48_previous_enemy_state_preserves_indeterminate_storage_bits() {
        assert_eq!(
            (
                preserve_previous_enemy_state_word(72),
                preserve_previous_enemy_state_word(0x5a3b_010e)
            ),
            (72, 0x5a3b_010e),
        );
    }

    #[test]
    fn invalid_stimulus_is_dormant_only_with_exact_constructor_shape() {
        let mut saved = LegacyStimulus {
            to_whole_patrol: false,
            stimulus_type: -282_591_232,
            info_type: 0,
            owner: super::super::payload_base::LegacyElementRef(None),
            info: LegacyStimulusInfo::None,
        };
        assert!(is_uninitialized_default_stimulus(&saved));

        saved.to_whole_patrol = true;
        assert!(!is_uninitialized_default_stimulus(&saved));
        saved.to_whole_patrol = false;
        saved.owner = super::super::payload_base::LegacyElementRef(Some(103));
        assert!(!is_uninitialized_default_stimulus(&saved));
    }

    #[test]
    fn original_order_ids_share_the_sequence_managers_shifted_domain() {
        assert_eq!(map_legacy_order_id_or_sentinel(0), 1);
        assert_eq!(map_legacy_order_id_or_sentinel(41), 42);
        assert_eq!(map_legacy_order_id_or_sentinel(u32::MAX), u32::MAX);
    }
}
