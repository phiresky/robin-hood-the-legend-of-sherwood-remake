//! Common v48 phase-two element payload readers.
//!
//! Original leaf serializers interleave inherited serializers rather than
//! writing one uniform base-first prefix. Each structure here is therefore an
//! independently callable reader. Leaf readers must invoke it at the exact
//! point where their C++ serializer calls its parent.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::elements::LegacyElementClass;
use super::payload_ai::LegacyLocalAiPayload;
use super::payload_sequences::LegacyInlineSequence;
use super::payload_vm::LegacyVmMemberSection;

const NULL_U32: u32 = u32::MAX;
const FINGERPRINT_ELEMENT: [u8; 16] = hex16("7730a5b25924f7a72c4926ef69f7700f");
const FINGERPRINT_SPRITE: [u8; 16] = hex16("ef8f9051c70a8eb993b6101ac4210ca5");
const FINGERPRINT_POSITION: [u8; 16] = hex16("f41fe85b168584aa52b8bb352f8b593a");
const FINGERPRINT_FX: [u8; 16] = hex16("780c28f3db22e4ecb2fe440fb4db25c1");
const FINGERPRINT_FX_MASKED: [u8; 16] = hex16("40b36826668c188dd5344e4b4c74c8e3");
const FINGERPRINT_MOBILE: [u8; 16] = hex16("5b090444c3c591c2114a5a503b1738e9");
const FINGERPRINT_ACTOR: [u8; 16] = hex16("121569cf426c32cd958ce53dde751dfc");
const FINGERPRINT_HUMAN: [u8; 16] = hex16("ede7221bc4b25f19c0b65eee425a82a5");
const FINGERPRINT_NPC: [u8; 16] = hex16("43960282833355d4ecb17f46320d0dae");
const FINGERPRINT_PATH_STATUS: [u8; 16] = hex16("f2781c304bb147aa1defc89ab1033082");
const FINGERPRINT_DETECTABLE: [u8; 16] = hex16("ef03cf4b42a0a6d23b96f2c434304c92");

const fn hex16(value: &str) -> [u8; 16] {
    let bytes = value.as_bytes();
    let mut result = [0; 16];
    let mut index = 0;
    while index < 16 {
        result[index] = (hex_nibble(bytes[index * 2]) << 4) | hex_nibble(bytes[index * 2 + 1]);
        index += 1;
    }
    result
}

const fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("invalid fingerprint hex"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPayloadLimits {
    pub sprite_animation_replacements: usize,
    pub actor_bypass_points: usize,
    pub human_opponents: usize,
    pub human_sword_victims: usize,
    pub human_shoots: usize,
    pub npc_detectables_per_type: usize,
    pub mobile_sprites: usize,
    pub mobile_vibrations: usize,
    pub mobile_alerted_animals: usize,
    pub path_history: usize,
}

impl Default for LegacyPayloadLimits {
    fn default() -> Self {
        Self {
            sprite_animation_replacements: 4096,
            actor_bypass_points: 4096,
            human_opponents: 4096,
            human_sword_victims: 4096,
            human_shoots: 4096,
            npc_detectables_per_type: 65_535,
            mobile_sprites: 4096,
            mobile_vibrations: 65_535,
            mobile_alerted_animals: 65_535,
            path_history: 65_535,
        }
    }
}

/// Mission-initialized metadata required by portions of the legacy grammar
/// that are not self-describing on disk.
pub trait LegacyPayloadDecodeContext {
    /// Number of embedded `RHElementFXMasked` parts already constructed for a
    /// mobile element. The count is not serialized.
    fn mobile_sprite_count(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        maximum: usize,
    ) -> LegacyResult<usize>;

    /// Decode VM members using the compiled class's ordered member schema.
    fn read_actor_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
        script_class: &str,
    ) -> LegacyResult<LegacyVmMemberSection>;

    /// Decode the full inline `RHSequence::Serialize(file, false)` body.
    fn read_inline_sequence(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<LegacyInlineSequence>;

    /// Decode `RHArtificialIntelligence::SerializeThisAI`.
    fn read_local_ai(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<Box<LegacyLocalAiPayload>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyElementRef(pub Option<u32>);

/// AI-local element references are 16-bit engine-array indices, unlike the
/// 32-bit creation-order IDs used by normal engine element references.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyAiElementRef(pub Option<u16>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySequenceElementRef(pub Option<u32>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySequenceRef(pub Option<u32>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyOrderRef(pub Option<u32>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySectorRef(pub Option<u16>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySignedIndexRef(pub Option<i16>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyLineRef {
    pub layer: Option<u16>,
    pub index: Option<i16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyOpaquePointer32(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPoint2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPoint3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyBoundingBox2 {
    pub top_left: LegacyPoint2,
    pub bottom_right: LegacyPoint2,
    pub bounds_are_set: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyBoundingBox3 {
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
    pub z_min: f32,
    pub z_max: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPlane3 {
    pub a: LegacyPoint3,
    pub b: LegacyPoint3,
    pub normal: LegacyPoint3,
    pub origin: LegacyPoint3,
    pub u: LegacyPoint3,
    pub v: LegacyPoint3,
    pub az: f32,
    pub bz: f32,
    pub dz: f32,
    pub d: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyElementPayloadBase {
    pub creation_order: u32,
    pub outline_colors: [u16; 5],
    pub current_outline: u32,
    pub outline_width: u16,
    pub custom_minimap_dot: u16,
    pub active: bool,
    pub position_map_delayed: bool,
    pub position_delayed: bool,
    pub class: LegacyElementClass,
    pub delayed_map_position: LegacyPoint2,
    pub delayed_position: LegacyPoint3,
    pub in_honolulu: bool,
    pub index_in_elements_list: u16,
    pub blipped: bool,
    pub unreachable: bool,
    pub sprite: LegacySpritePayload,
}

impl LegacyElementPayloadBase {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacyPayloadLimits,
        expected_creation_order: Option<u32>,
        expected_class: Option<LegacyElementClass>,
    ) -> LegacyResult<Self> {
        read_fingerprint(reader, "fingerprint", FINGERPRINT_ELEMENT, "RHElement")?;
        let creation_offset = reader.offset();
        let creation_order = reader.read_u32("creation_order")?;
        if let Some(expected) = expected_creation_order {
            if creation_order != expected {
                return Err(reader.invalid_value(
                    creation_offset,
                    "creation_order",
                    creation_order,
                    "creation order from the phase-one envelope",
                ));
            }
        }
        let mut outline_colors = [0; 5];
        for (index, color) in outline_colors.iter_mut().enumerate() {
            *color = reader.read_u16(format_args!("outline_colors[{index}]"))?;
        }
        let current_outline = reader.read_u32("current_outline")?;
        let outline_width = reader.read_u16("outline_width")?;
        let custom_minimap_dot = reader.read_u16("custom_minimap_dot")?;
        let active = reader.read_bool("active")?;
        let position_map_delayed = reader.read_bool("position_map_delayed")?;
        let position_delayed = reader.read_bool("position_delayed")?;
        let class_offset = reader.offset();
        let raw_class = reader.read_u16("class_id")?;
        let Some(class) = LegacyElementClass::from_raw(raw_class) else {
            return Err(reader.invalid_value(
                class_offset,
                "class_id",
                format_args!("0x{raw_class:04x}"),
                "known RHCLASSID concrete element class",
            ));
        };
        if let Some(expected) = expected_class {
            if class != expected {
                return Err(reader.invalid_value(
                    class_offset,
                    "class_id",
                    format_args!("0x{raw_class:04x}"),
                    "class id from the phase-one envelope",
                ));
            }
        }
        let delayed_map_position = read_point2(reader, "delayed_map_position")?;
        let delayed_position = read_point3(reader, "delayed_position")?;
        let in_honolulu = reader.read_bool("in_honolulu")?;
        let index_in_elements_list = reader.read_u16("index_in_elements_list")?;
        let blipped = reader.read_bool("blipped")?;
        let unreachable = reader.read_bool("unreachable")?;
        let sprite = reader.scope("sprite", |reader| LegacySpritePayload::read(reader, limits))?;
        Ok(Self {
            creation_order,
            outline_colors,
            current_outline,
            outline_width,
            custom_minimap_dot,
            active,
            position_map_delayed,
            position_delayed,
            class,
            delayed_map_position,
            delayed_position,
            in_honolulu,
            index_in_elements_list,
            blipped,
            unreachable,
            sprite,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySpritePayload {
    pub current_row: u16,
    pub current_frame: u16,
    pub frame_count: u16,
    pub current_height: u16,
    pub current_width: u16,
    pub last_action: u32,
    pub already_decompressed: bool,
    pub alternate_profile: bool,
    pub masked: bool,
    pub display_order: f32,
    pub legacy_display_order_dummy: i32,
    pub behind_display_order_reference: bool,
    pub display_order_reference: LegacyElementRef,
    pub action_done_frame: u16,
    pub action_done_counter: u16,
    pub frame_count_down: u16,
    pub last_sound_id: u16,
    pub last_processed_order_id: u32,
    pub bounding_box: LegacyBoundingBox2,
    pub animation_replacements: Vec<(u32, u32)>,
    pub position: LegacyPositionPayload,
}

impl LegacySpritePayload {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyPayloadLimits) -> LegacyResult<Self> {
        read_fingerprint(reader, "fingerprint", FINGERPRINT_SPRITE, "RHSprite")?;
        let current_row = reader.read_u16("current_row")?;
        let current_frame = reader.read_u16("current_frame")?;
        let frame_count = reader.read_u16("frame_count")?;
        let current_height = reader.read_u16("current_height")?;
        let current_width = reader.read_u16("current_width")?;
        let last_action = reader.read_u32("last_action")?;
        let already_decompressed = reader.read_bool("already_decompressed")?;
        let alternate_profile = reader.read_bool("alternate_profile")?;
        let masked = reader.read_bool("masked")?;
        let display_order = reader.read_f32("display_order")?;
        let legacy_display_order_dummy = reader.read_i32("legacy_display_order_dummy")?;
        let behind_display_order_reference = reader.read_bool("behind_display_order_reference")?;
        let display_order_reference = read_element_ref(reader, "display_order_reference")?;
        let action_done_frame = reader.read_u16("action_done_frame")?;
        let action_done_counter = reader.read_u16("action_done_counter")?;
        let frame_count_down = reader.read_u16("frame_count_down")?;
        let last_sound_id = reader.read_u16("last_sound_id")?;
        let last_processed_order_id = reader.read_u32("last_processed_order_id")?;
        let bounding_box = read_box2(reader, "bounding_box")?;
        let count = read_bounded_u32(
            reader,
            "animation_replacements.count",
            limits.sprite_animation_replacements,
        )?;
        let mut animation_replacements = Vec::new();
        reserve(
            reader,
            &mut animation_replacements,
            count,
            "animation_replacements",
        )?;
        for index in 0..count {
            animation_replacements.push(
                reader.scope(format!("animation_replacements[{index}]"), |reader| {
                    Ok((reader.read_u32("from")?, reader.read_u32("to")?))
                })?,
            );
        }
        let position = reader.scope("position", |reader| LegacyPositionPayload::read(reader))?;
        Ok(Self {
            current_row,
            current_frame,
            frame_count,
            current_height,
            current_width,
            last_action,
            already_decompressed,
            alternate_profile,
            masked,
            display_order,
            legacy_display_order_dummy,
            behind_display_order_reference,
            display_order_reference,
            action_done_frame,
            action_done_counter,
            frame_count_down,
            last_sound_id,
            last_processed_order_id,
            bounding_box,
            animation_replacements,
            position,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPositionPayload {
    pub computed_position: u32,
    pub computed_increment: u32,
    pub material: u32,
    pub posture: u32,
    pub old_posture: u32,
    pub direction: i16,
    pub direction_goal: i16,
    pub slow_turn_count: u8,
    pub layer: u16,
    pub layer_goal: u16,
    pub tolerance: f32,
    pub directional_tolerance: bool,
    pub accumulate_movement_map: bool,
    pub anti_collision_on: bool,
    pub goal_next_valid: bool,
    pub deviated: bool,
    pub direction_count: i8,
    pub door_direction: bool,
    pub reversed_movement: bool,
    pub blocked_count: u16,
    pub radius: f32,
    pub use_emergency_lying_box: bool,
    pub sector: LegacySectorRef,
    pub sector_goal: LegacySectorRef,
    pub door: LegacySignedIndexRef,
    pub obstacle: LegacySignedIndexRef,
    pub target_element: LegacyElementRef,
    pub position: LegacyPoint3,
    pub map: LegacyPoint2,
    pub sprite: LegacyPoint2,
    pub old_position: LegacyPoint3,
    pub old_map: LegacyPoint2,
    pub old_sprite: LegacyPoint2,
    pub goal_map: LegacyPoint2,
    pub goal_next_map: LegacyPoint2,
    pub goal: LegacyPoint3,
    pub increment: LegacyPoint3,
    pub increment_map: LegacyPoint2,
    pub accumulated_movement_map: LegacyPoint2,
    pub forecasted_movement: LegacyPoint3,
    pub move_box_map: LegacyBoundingBox2,
    pub blocked_box: LegacyBoundingBox2,
}

impl LegacyPositionPayload {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        read_fingerprint(
            reader,
            "fingerprint",
            FINGERPRINT_POSITION,
            "RHPositionInterface",
        )?;
        let computed_position = reader.read_u32("computed_position")?;
        let computed_increment = reader.read_u32("computed_increment")?;
        let material = reader.read_u32("material")?;
        let posture = reader.read_u32("posture")?;
        let old_posture = reader.read_u32("old_posture")?;
        let direction = reader.read_i16("direction")?;
        let direction_goal = reader.read_i16("direction_goal")?;
        let slow_turn_count = reader.read_u8("slow_turn_count")?;
        let layer = reader.read_u16("layer")?;
        let layer_goal = reader.read_u16("layer_goal")?;
        let tolerance = reader.read_f32("tolerance")?;
        let directional_tolerance = reader.read_bool("directional_tolerance")?;
        let accumulate_movement_map = reader.read_bool("accumulate_movement_map")?;
        let anti_collision_on = reader.read_bool("anti_collision_on")?;
        let goal_next_valid = reader.read_bool("goal_next_valid")?;
        let deviated = reader.read_bool("deviated")?;
        let direction_count = reader.read_i8("direction_count")?;
        let door_direction = reader.read_bool("door_direction")?;
        let reversed_movement = reader.read_bool("reversed_movement")?;
        let blocked_count = reader.read_u16("blocked_count")?;
        let radius = reader.read_f32("radius")?;
        let use_emergency_lying_box = reader.read_bool("use_emergency_lying_box")?;
        let sector = read_sector_ref(reader, "sector")?;
        let sector_goal = read_sector_ref(reader, "sector_goal")?;
        let door = read_signed_ref(reader, "door")?;
        let obstacle = read_signed_ref(reader, "obstacle")?;
        let target_element = read_element_ref(reader, "target_element")?;
        Ok(Self {
            computed_position,
            computed_increment,
            material,
            posture,
            old_posture,
            direction,
            direction_goal,
            slow_turn_count,
            layer,
            layer_goal,
            tolerance,
            directional_tolerance,
            accumulate_movement_map,
            anti_collision_on,
            goal_next_valid,
            deviated,
            direction_count,
            door_direction,
            reversed_movement,
            blocked_count,
            radius,
            use_emergency_lying_box,
            sector,
            sector_goal,
            door,
            obstacle,
            target_element,
            position: read_point3(reader, "position")?,
            map: read_point2(reader, "map")?,
            sprite: read_point2(reader, "sprite")?,
            old_position: read_point3(reader, "old_position")?,
            old_map: read_point2(reader, "old_map")?,
            old_sprite: read_point2(reader, "old_sprite")?,
            goal_map: read_point2(reader, "goal_map")?,
            goal_next_map: read_point2(reader, "goal_next_map")?,
            goal: read_point3(reader, "goal")?,
            increment: read_point3(reader, "increment")?,
            increment_map: read_point2(reader, "increment_map")?,
            accumulated_movement_map: read_point2(reader, "accumulated_movement_map")?,
            forecasted_movement: read_point3(reader, "forecasted_movement")?,
            move_box_map: read_box2(reader, "move_box_map")?,
            blocked_box: read_box2(reader, "blocked_box")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyFxPayload {
    pub patch: LegacySignedIndexRef,
    pub force_display: bool,
    pub restore_background: bool,
    pub element: LegacyElementPayloadBase,
}

impl LegacyFxPayload {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacyPayloadLimits,
        expected_creation_order: Option<u32>,
        expected_class: Option<LegacyElementClass>,
    ) -> LegacyResult<Self> {
        read_fingerprint(reader, "fingerprint", FINGERPRINT_FX, "RHElementFX")?;
        let patch = read_signed_ref(reader, "patch")?;
        let force_display = reader.read_bool("force_display")?;
        let restore_background = reader.read_bool("restore_background")?;
        let element = reader.scope("element", |reader| {
            LegacyElementPayloadBase::read(reader, limits, expected_creation_order, expected_class)
        })?;
        Ok(Self {
            patch,
            force_display,
            restore_background,
            element,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyFxMaskedPayload {
    pub animation_speed: f32,
    pub element: LegacyElementPayloadBase,
}

impl LegacyFxMaskedPayload {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyPayloadLimits) -> LegacyResult<Self> {
        read_fingerprint(
            reader,
            "fingerprint",
            FINGERPRINT_FX_MASKED,
            "RHElementFXMasked",
        )?;
        let animation_speed = reader.read_f32("animation_speed")?;
        let element = reader.scope("element", |reader| {
            LegacyElementPayloadBase::read(reader, limits, None, Some(LegacyElementClass::FxMasked))
        })?;
        Ok(Self {
            animation_speed,
            element,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPathHistoryEntry {
    pub position: LegacyPoint2,
    pub sector: LegacySectorRef,
    pub level: u16,
    pub direction: u8,
    pub distance: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPathStatus {
    pub current_waypoint_index: u8,
    pub last_waypoint_index: u8,
    pub forward_movement: bool,
    pub hiking_path_index: Option<u16>,
    pub history: Vec<LegacyPathHistoryEntry>,
}

impl LegacyPathStatus {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyPayloadLimits) -> LegacyResult<Self> {
        read_fingerprint(
            reader,
            "fingerprint",
            FINGERPRINT_PATH_STATUS,
            "RHPath::SerializeStatus",
        )?;
        let current_waypoint_index = reader.read_u8("current_waypoint_index")?;
        let last_waypoint_index = reader.read_u8("last_waypoint_index")?;
        let forward_movement = reader.read_bool("forward_movement")?;
        let raw_hiking_path = reader.read_u16("hiking_path_index")?;
        let hiking_path_index = (raw_hiking_path != u16::MAX).then_some(raw_hiking_path);
        let history_count = read_bounded_u16(reader, "history.count", limits.path_history)?;
        let mut history = Vec::new();
        reserve(reader, &mut history, history_count, "history")?;
        for index in 0..history_count {
            history.push(reader.scope(format!("history[{index}]"), |reader| {
                Ok(LegacyPathHistoryEntry {
                    position: read_point2(reader, "position")?,
                    sector: read_sector_ref(reader, "sector")?,
                    level: reader.read_u16("level")?,
                    direction: reader.read_u8("direction")?,
                    distance: reader.read_u16("distance")?,
                })
            })?);
        }
        Ok(Self {
            current_waypoint_index,
            last_waypoint_index,
            forward_movement,
            hiking_path_index,
            history,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyMobilePayload {
    pub sprites: Vec<LegacyFxMaskedPayload>,
    pub stopped: bool,
    pub vibrations: Vec<LegacyPoint2>,
    pub animation: u32,
    pub hook: LegacyPoint2,
    pub hooked_actor: LegacyElementRef,
    pub smoke_time: u32,
    pub smoke_delay: u32,
    pub relative_position: LegacyPoint2,
    pub path: LegacyPathStatus,
    pub on_waypoint: bool,
    pub waypoint_data_offset: Option<u32>,
    pub waypoint_bytes_remaining: Option<u16>,
    pub wait_time: u32,
    pub speed: f32,
    pub speed_goal: f32,
    pub acceleration: f32,
    pub adaptive_speed: bool,
    pub front: LegacyPoint2,
    pub back: LegacyPoint2,
    pub alerted_animals: Vec<LegacyElementRef>,
    pub steam_sound_1: i16,
    pub steam_sound_2: i16,
    pub steam: bool,
    pub brakes_sound: i16,
    pub brakes: bool,
    pub element: LegacyElementPayloadBase,
}

impl LegacyMobilePayload {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacyPayloadLimits,
        context: &dyn LegacyPayloadDecodeContext,
        expected_creation_order: u32,
    ) -> LegacyResult<Self> {
        read_fingerprint(reader, "fingerprint", FINGERPRINT_MOBILE, "RHElementMobile")?;
        let sprite_count =
            context.mobile_sprite_count(reader, expected_creation_order, limits.mobile_sprites)?;
        if sprite_count > limits.mobile_sprites {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "sprites",
                sprite_count,
                "context sprite count within the caller-supplied limit",
            ));
        }
        let mut sprites = Vec::new();
        reserve(reader, &mut sprites, sprite_count, "sprites")?;
        for index in 0..sprite_count {
            sprites.push(reader.scope(format!("sprites[{index}]"), |reader| {
                LegacyFxMaskedPayload::read(reader, limits)
            })?);
        }
        let stopped = reader.read_bool("stopped")?;
        let vibration_count =
            read_bounded_u32(reader, "vibrations.count", limits.mobile_vibrations)?;
        let mut vibrations = Vec::new();
        reserve(reader, &mut vibrations, vibration_count, "vibrations")?;
        for index in 0..vibration_count {
            vibrations.push(read_point2(reader, format!("vibrations[{index}]"))?);
        }
        let animation = reader.read_u32("animation")?;
        let hook = read_point2(reader, "hook")?;
        let hooked_actor = read_element_ref(reader, "hooked_actor")?;
        let smoke_time = reader.read_u32("smoke_time")?;
        let smoke_delay = reader.read_u32("smoke_delay")?;
        let relative_position = read_point2(reader, "relative_position")?;
        let path = reader.scope("path", |reader| LegacyPathStatus::read(reader, limits))?;
        let on_waypoint = reader.read_bool("on_waypoint")?;
        let (waypoint_data_offset, waypoint_bytes_remaining) = if on_waypoint {
            let raw_offset = reader.read_u32("waypoint_data_offset")?;
            (
                (raw_offset != NULL_U32).then_some(raw_offset),
                Some(reader.read_u16("waypoint_bytes_remaining")?),
            )
        } else {
            (None, None)
        };
        let wait_time = reader.read_u32("wait_time")?;
        let speed = reader.read_f32("speed")?;
        let speed_goal = reader.read_f32("speed_goal")?;
        let acceleration = reader.read_f32("acceleration")?;
        let adaptive_speed = reader.read_bool("adaptive_speed")?;
        let front = read_point2(reader, "front")?;
        let back = read_point2(reader, "back")?;
        let animal_count = read_bounded_u32(
            reader,
            "alerted_animals.count",
            limits.mobile_alerted_animals,
        )?;
        let mut alerted_animals = Vec::new();
        reserve(
            reader,
            &mut alerted_animals,
            animal_count,
            "alerted_animals",
        )?;
        for index in 0..animal_count {
            alerted_animals.push(read_element_ref(
                reader,
                format!("alerted_animals[{index}]"),
            )?);
        }
        let steam_sound_1 = reader.read_i16("steam_sound_1")?;
        let steam_sound_2 = reader.read_i16("steam_sound_2")?;
        let steam = reader.read_bool("steam")?;
        let brakes_sound = reader.read_i16("brakes_sound")?;
        let brakes = reader.read_bool("brakes")?;
        let element = reader.scope("element", |reader| {
            LegacyElementPayloadBase::read(
                reader,
                limits,
                Some(expected_creation_order),
                Some(LegacyElementClass::Mobile),
            )
        })?;
        Ok(Self {
            sprites,
            stopped,
            vibrations,
            animation,
            hook,
            hooked_actor,
            smoke_time,
            smoke_delay,
            relative_position,
            path,
            on_waypoint,
            waypoint_data_offset,
            waypoint_bytes_remaining,
            wait_time,
            speed,
            speed_goal,
            acceleration,
            adaptive_speed,
            front,
            back,
            alerted_animals,
            steam_sound_1,
            steam_sound_2,
            steam,
            brakes_sound,
            brakes,
            element,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyActorPayload {
    pub last_order_id: u32,
    pub old_action: u32,
    pub action_state: u32,
    pub execution_frozen: bool,
    pub about_to_surrender: bool,
    pub ignored_for_anti_collision: bool,
    pub surrendering: bool,
    pub distance_to_boundary_first: f32,
    pub new_order: bool,
    pub distance_to_boundary_second: f32,
    pub motion_state: u32,
    pub wait_time: u32,
    pub seek_layer: u16,
    pub bypassing: bool,
    pub on_railroad: bool,
    pub seek_distance: f32,
    pub seek_to_point: bool,
    pub check_for_jump: bool,
    pub passing_door_directly: bool,
    pub bypass_exit: LegacyPoint2,
    pub last_seek_target_position: LegacyPoint2,
    pub position_at_last_distance_request: LegacyPoint2,
    pub menacer: LegacyElementRef,
    pub seek_target: LegacyElementRef,
    pub bypass_reference: LegacyElementRef,
    pub material_sector: LegacySectorRef,
    pub seek_sector: LegacySectorRef,
    pub sequence_element: LegacySequenceElementRef,
    pub wait_sequence_element: LegacySequenceElementRef,
    pub order: LegacyOrderRef,
    pub sequence_element_started: bool,
    pub post_seek_sequence: Option<LegacyInlineSequence>,
    pub bypass_points: Vec<LegacyPoint2>,
    pub script_class: String,
    pub script_members: Option<LegacyVmMemberSection>,
    pub element: LegacyElementPayloadBase,
}

impl LegacyActorPayload {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacyPayloadLimits,
        context: &dyn LegacyPayloadDecodeContext,
        expected_creation_order: u32,
        expected_class: LegacyElementClass,
    ) -> LegacyResult<Self> {
        read_fingerprint(reader, "fingerprint", FINGERPRINT_ACTOR, "RHElementActor")?;
        let last_order_id = reader.read_u32("last_order_id")?;
        let old_action = reader.read_u32("old_action")?;
        let action_state = reader.read_u32("action_state")?;
        let execution_frozen = reader.read_bool("execution_frozen")?;
        let about_to_surrender = reader.read_bool("about_to_surrender")?;
        let ignored_for_anti_collision = reader.read_bool("ignored_for_anti_collision")?;
        let surrendering = reader.read_bool("surrendering")?;
        let distance_to_boundary_first = reader.read_f32("distance_to_boundary_first")?;
        let new_order = reader.read_bool("new_order")?;
        let distance_to_boundary_second = reader.read_f32("distance_to_boundary_second")?;
        let motion_state = reader.read_u32("motion_state")?;
        let wait_time = reader.read_u32("wait_time")?;
        let seek_layer = reader.read_u16("seek_layer")?;
        let bypassing = reader.read_bool("bypassing")?;
        let on_railroad = reader.read_bool("on_railroad")?;
        let seek_distance = reader.read_f32("seek_distance")?;
        let seek_to_point = reader.read_bool("seek_to_point")?;
        let check_for_jump = reader.read_bool("check_for_jump")?;
        let passing_door_directly = reader.read_bool("passing_door_directly")?;
        let bypass_exit = read_point2(reader, "bypass_exit")?;
        let last_seek_target_position = read_point2(reader, "last_seek_target_position")?;
        let position_at_last_distance_request =
            read_point2(reader, "position_at_last_distance_request")?;
        let menacer = read_element_ref(reader, "menacer")?;
        let seek_target = read_element_ref(reader, "seek_target")?;
        let bypass_reference = read_element_ref(reader, "bypass_reference")?;
        let material_sector = read_sector_ref(reader, "material_sector")?;
        let seek_sector = read_sector_ref(reader, "seek_sector")?;
        let sequence_element = read_sequence_element_ref(reader, "sequence_element")?;
        let wait_sequence_element = read_sequence_element_ref(reader, "wait_sequence_element")?;
        let order = read_order_ref(reader, "order")?;
        let sequence_element_started = reader.read_bool("sequence_element_started")?;
        let has_post_seek = reader.read_bool("has_post_seek_sequence")?;
        let post_seek_sequence = if has_post_seek {
            Some(reader.scope("post_seek_sequence", |reader| {
                context.read_inline_sequence(reader, expected_creation_order, expected_class)
            })?)
        } else {
            None
        };
        let bypass_count =
            read_bounded_u16(reader, "bypass_points.count", limits.actor_bypass_points)?;
        let mut bypass_points = Vec::new();
        reserve(reader, &mut bypass_points, bypass_count, "bypass_points")?;
        for index in 0..bypass_count {
            bypass_points.push(read_point2(reader, format!("bypass_points[{index}]"))?);
        }
        let script_class = reader.read_string("script_class")?;
        let script_members = if script_class.is_empty() {
            None
        } else {
            Some(reader.scope("script_members", |reader| {
                context.read_actor_script_members(
                    reader,
                    expected_creation_order,
                    expected_class,
                    &script_class,
                )
            })?)
        };
        // The Original intentionally brackets script state with the same
        // actor fingerprint.
        read_fingerprint(
            reader,
            "trailing_fingerprint",
            FINGERPRINT_ACTOR,
            "RHElementActor",
        )?;
        let element = reader.scope("element", |reader| {
            LegacyElementPayloadBase::read(
                reader,
                limits,
                Some(expected_creation_order),
                Some(expected_class),
            )
        })?;
        Ok(Self {
            last_order_id,
            old_action,
            action_state,
            execution_frozen,
            about_to_surrender,
            ignored_for_anti_collision,
            surrendering,
            distance_to_boundary_first,
            new_order,
            distance_to_boundary_second,
            motion_state,
            wait_time,
            seek_layer,
            bypassing,
            on_railroad,
            seek_distance,
            seek_to_point,
            check_for_jump,
            passing_door_directly,
            bypass_exit,
            last_seek_target_position,
            position_at_last_distance_request,
            menacer,
            seek_target,
            bypass_reference,
            material_sector,
            seek_sector,
            sequence_element,
            wait_sequence_element,
            order,
            sequence_element_started,
            post_seek_sequence,
            bypass_points,
            script_class,
            script_members,
            element,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyRepulsivePoint {
    pub position: LegacyPoint2,
    pub concave: bool,
    pub limit_left: LegacyPoint2,
    pub limit_right: LegacyPoint2,
    pub action_radius: f32,
    pub force_a: f32,
    pub force_b: f32,
    pub radius: f32,
    pub id: u32,
    pub affects_pcs: bool,
    pub affects_soldiers: bool,
    pub affects_civilians: bool,
    pub affects_animals: bool,
}

impl LegacyRepulsivePoint {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        Ok(Self {
            position: read_point2(reader, "position")?,
            concave: reader.read_bool("concave")?,
            limit_left: read_point2(reader, "limit_left")?,
            limit_right: read_point2(reader, "limit_right")?,
            action_radius: reader.read_f32("action_radius")?,
            force_a: reader.read_f32("force_a")?,
            force_b: reader.read_f32("force_b")?,
            radius: reader.read_f32("radius")?,
            id: reader.read_u32("id")?,
            affects_pcs: reader.read_bool("affects_pcs")?,
            affects_soldiers: reader.read_bool("affects_soldiers")?,
            affects_civilians: reader.read_bool("affects_civilians")?,
            affects_animals: reader.read_bool("affects_animals")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyShieldPayload {
    pub points: [LegacyShieldPoint; 4],
    pub top_plane: LegacyPlane3,
    pub bottom_plane: LegacyPlane3,
    pub box_3d: LegacyBoundingBox3,
    pub ground_box: LegacyBoundingBox2,
    pub screen_box: LegacyBoundingBox2,
    pub on_ground: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyShieldPoint {
    pub obstacle: [f32; 4],
    pub polygon: LegacyPoint2,
}

impl LegacyShieldPayload {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        let mut points = [LegacyShieldPoint {
            obstacle: [0.0; 4],
            polygon: LegacyPoint2 { x: 0.0, y: 0.0 },
        }; 4];
        for (index, point) in points.iter_mut().enumerate() {
            *point = reader.scope(format!("points[{index}]"), |reader| {
                Ok(LegacyShieldPoint {
                    obstacle: [
                        reader.read_f32("obstacle.x")?,
                        reader.read_f32("obstacle.y")?,
                        reader.read_f32("obstacle.z_top")?,
                        reader.read_f32("obstacle.z_bottom")?,
                    ],
                    polygon: read_point2(reader, "polygon")?,
                })
            })?;
        }
        Ok(Self {
            points,
            top_plane: read_plane3(reader, "top_plane")?,
            bottom_plane: read_plane3(reader, "bottom_plane")?,
            box_3d: read_box3(reader, "box_3d")?,
            ground_box: read_box2(reader, "ground_box")?,
            screen_box: read_box2(reader, "screen_box")?,
            on_ground: reader.read_bool("on_ground")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySwordOpponent {
    pub opponent: LegacyElementRef,
    pub jump_line: LegacyLineRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyHumanPayload {
    pub already_detectable_body: bool,
    pub concussion_healing_timeout: u16,
    pub unconscious: bool,
    pub tiredness: u16,
    pub concussion: u16,
    pub parry_counter: u16,
    pub detectable_list_index: u16,
    pub invulnerable: bool,
    pub last_motion_was_step_back: bool,
    pub smalltalk_initiative: bool,
    pub received_smalltalk_initiative: bool,
    pub relative_fighting_ability: u16,
    pub hollow_man: bool,
    pub opponents: Vec<LegacySwordOpponent>,
    pub killed_by_accident: bool,
    pub running_hulk: u32,
    pub time_hulk: u32,
    pub hulk_level: u16,
    pub hulk_direction: bool,
    pub hulk_speed: f32,
    pub carrier: LegacyElementRef,
    pub repulsive_point: LegacyRepulsivePoint,
    pub small_repulsive_radius: bool,
    pub building: LegacySectorRef,
    /// Original `CHECKENUM(mCurrentlyProducedNoise)` writes only the first
    /// four bytes of the `RHnoise` struct, which are `posOrigin.mX`.
    pub currently_produced_noise_first_word: f32,
    pub actor: LegacyActorPayload,
    pub shield: LegacyShieldPayload,
    pub sword_strike_victims: Vec<LegacyElementRef>,
    pub initial_strike_angle: f32,
    pub current_strike_angle: f32,
    pub final_strike_angle: f32,
    pub stuck_under_nets_counter: u16,
    pub sword_strike_boredom: [u16; 9],
    pub shoots: Vec<LegacySequenceElementRef>,
    pub smalltalk_hint: u32,
    pub hint_opponent: LegacyElementRef,
}

impl LegacyHumanPayload {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacyPayloadLimits,
        context: &dyn LegacyPayloadDecodeContext,
        expected_creation_order: u32,
        expected_class: LegacyElementClass,
    ) -> LegacyResult<Self> {
        read_fingerprint(
            reader,
            "fingerprint",
            FINGERPRINT_HUMAN,
            "RHElementActorHuman",
        )?;
        let already_detectable_body = reader.read_bool("already_detectable_body")?;
        let concussion_healing_timeout = reader.read_u16("concussion_healing_timeout")?;
        let unconscious = reader.read_bool("unconscious")?;
        let tiredness = reader.read_u16("tiredness")?;
        let concussion = reader.read_u16("concussion")?;
        let parry_counter = reader.read_u16("parry_counter")?;
        let detectable_list_index = reader.read_u16("detectable_list_index")?;
        let invulnerable = reader.read_bool("invulnerable")?;
        let last_motion_was_step_back = reader.read_bool("last_motion_was_step_back")?;
        let smalltalk_initiative = reader.read_bool("smalltalk_initiative")?;
        let received_smalltalk_initiative = reader.read_bool("received_smalltalk_initiative")?;
        let relative_fighting_ability = reader.read_u16("relative_fighting_ability")?;
        let hollow_man = reader.read_bool("hollow_man")?;
        let opponent_count = read_bounded_u16(reader, "opponents.count", limits.human_opponents)?;
        let mut opponents = Vec::new();
        reserve(reader, &mut opponents, opponent_count, "opponents")?;
        for index in 0..opponent_count {
            opponents.push(reader.scope(format!("opponents[{index}]"), |reader| {
                Ok(LegacySwordOpponent {
                    opponent: read_element_ref(reader, "opponent")?,
                    jump_line: read_line_ref(reader, "jump_line")?,
                })
            })?);
        }
        let killed_by_accident = reader.read_bool("killed_by_accident")?;
        let running_hulk = reader.read_u32("running_hulk")?;
        let time_hulk = reader.read_u32("time_hulk")?;
        let hulk_level = reader.read_u16("hulk_level")?;
        let hulk_direction = reader.read_bool("hulk_direction")?;
        let hulk_speed = reader.read_f32("hulk_speed")?;
        let carrier = read_element_ref(reader, "carrier")?;
        let repulsive_point = reader.scope("repulsive_point", LegacyRepulsivePoint::read)?;
        let small_repulsive_radius = reader.read_bool("small_repulsive_radius")?;
        let building = read_sector_ref(reader, "building")?;
        let currently_produced_noise_first_word =
            reader.read_f32("currently_produced_noise_first_word")?;
        let actor = reader.scope("actor", |reader| {
            LegacyActorPayload::read(
                reader,
                limits,
                context,
                expected_creation_order,
                expected_class,
            )
        })?;
        let shield = reader.scope("shield", LegacyShieldPayload::read)?;
        let victim_count = read_bounded_u16(
            reader,
            "sword_strike_victims.count",
            limits.human_sword_victims,
        )?;
        let mut sword_strike_victims = Vec::new();
        reserve(
            reader,
            &mut sword_strike_victims,
            victim_count,
            "sword_strike_victims",
        )?;
        for index in 0..victim_count {
            sword_strike_victims.push(read_element_ref(
                reader,
                format!("sword_strike_victims[{index}]"),
            )?);
        }
        let initial_strike_angle = reader.read_f32("initial_strike_angle")?;
        let current_strike_angle = reader.read_f32("current_strike_angle")?;
        let final_strike_angle = reader.read_f32("final_strike_angle")?;
        let stuck_under_nets_counter = reader.read_u16("stuck_under_nets_counter")?;
        let mut sword_strike_boredom = [0; 9];
        for (index, value) in sword_strike_boredom.iter_mut().enumerate() {
            *value = reader.read_u16(format_args!("sword_strike_boredom[{index}]"))?;
        }
        let shoot_count = read_bounded_u32(reader, "shoots.count", limits.human_shoots)?;
        let mut shoots = Vec::new();
        reserve(reader, &mut shoots, shoot_count, "shoots")?;
        for index in 0..shoot_count {
            shoots.push(read_sequence_element_ref(
                reader,
                format!("shoots[{index}]"),
            )?);
        }
        let smalltalk_hint = reader.read_u32("smalltalk_hint")?;
        let hint_opponent = read_element_ref(reader, "hint_opponent")?;
        Ok(Self {
            already_detectable_body,
            concussion_healing_timeout,
            unconscious,
            tiredness,
            concussion,
            parry_counter,
            detectable_list_index,
            invulnerable,
            last_motion_was_step_back,
            smalltalk_initiative,
            received_smalltalk_initiative,
            relative_fighting_ability,
            hollow_man,
            opponents,
            killed_by_accident,
            running_hulk,
            time_hulk,
            hulk_level,
            hulk_direction,
            hulk_speed,
            carrier,
            repulsive_point,
            small_repulsive_radius,
            building,
            currently_produced_noise_first_word,
            actor,
            shield,
            sword_strike_victims,
            initial_strike_angle,
            current_strike_angle,
            final_strike_angle,
            stuck_under_nets_counter,
            sword_strike_boredom,
            shoots,
            smalltalk_hint,
            hint_opponent,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyNpcView {
    pub leaning: bool,
    pub leaning_padding: [u8; 3],
    pub alert_status: u32,
    pub status: u8,
    pub transitioning: bool,
    pub alpha: u16,
    pub half_angle: f32,
    pub angle_iterator: f32,
    pub angle_iterator_step: f32,
    pub angle_step: f32,
    pub angle: f32,
    pub half_aperture: f32,
    pub real_half_aperture: f32,
    pub half_aperture_cosine: f32,
    pub future_half_aperture: f32,
    pub half_aperture_step: f32,
    pub half_aperture_changes: bool,
    pub half_aperture_padding: [u8; 3],
    pub crazy_iterator: f32,
    pub crazy_iterator_step: f32,
    pub color: u8,
    pub color_padding: [u8; 3],
    pub crazy_half_aperture: f32,
    pub direction: LegacyPoint2,
    pub left: LegacyPoint2,
    pub right: LegacyPoint2,
    pub stare: LegacyPoint2,
    /// Raw 32-bit host pointer echo from the original i386 ABI.
    pub raw_mobile_target_pointer: LegacyOpaquePointer32,
    pub radius_goal: u16,
    pub radius: u16,
    pub radius_reduction: u16,
    pub radius_step: u16,
    pub long_range: f32,
    pub real_radius: u16,
    pub real_radius_padding: [u8; 2],
    pub drunkenness: [f32; 4],
    pub sniper: bool,
    pub sniper_padding: [u8; 3],
}

impl LegacyNpcView {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        Ok(Self {
            leaning: reader.read_bool("leaning")?,
            leaning_padding: read_array(reader, "leaning_padding")?,
            alert_status: reader.read_u32("alert_status")?,
            status: reader.read_u8("status")?,
            transitioning: reader.read_bool("transitioning")?,
            alpha: reader.read_u16("alpha")?,
            half_angle: reader.read_f32("half_angle")?,
            angle_iterator: reader.read_f32("angle_iterator")?,
            angle_iterator_step: reader.read_f32("angle_iterator_step")?,
            angle_step: reader.read_f32("angle_step")?,
            angle: reader.read_f32("angle")?,
            half_aperture: reader.read_f32("half_aperture")?,
            real_half_aperture: reader.read_f32("real_half_aperture")?,
            half_aperture_cosine: reader.read_f32("half_aperture_cosine")?,
            future_half_aperture: reader.read_f32("future_half_aperture")?,
            half_aperture_step: reader.read_f32("half_aperture_step")?,
            half_aperture_changes: reader.read_bool("half_aperture_changes")?,
            half_aperture_padding: read_array(reader, "half_aperture_padding")?,
            crazy_iterator: reader.read_f32("crazy_iterator")?,
            crazy_iterator_step: reader.read_f32("crazy_iterator_step")?,
            color: reader.read_u8("color")?,
            color_padding: read_array(reader, "color_padding")?,
            crazy_half_aperture: reader.read_f32("crazy_half_aperture")?,
            direction: read_point2(reader, "direction")?,
            left: read_point2(reader, "left")?,
            right: read_point2(reader, "right")?,
            stare: read_point2(reader, "stare")?,
            raw_mobile_target_pointer: LegacyOpaquePointer32(
                reader.read_u32("raw_mobile_target_pointer")?,
            ),
            radius_goal: reader.read_u16("radius_goal")?,
            radius: reader.read_u16("radius")?,
            radius_reduction: reader.read_u16("radius_reduction")?,
            radius_step: reader.read_u16("radius_step")?,
            long_range: reader.read_f32("long_range")?,
            real_radius: reader.read_u16("real_radius")?,
            real_radius_padding: read_array(reader, "real_radius_padding")?,
            drunkenness: [
                reader.read_f32("drunkenness[0]")?,
                reader.read_f32("drunkenness[1]")?,
                reader.read_f32("drunkenness[2]")?,
                reader.read_f32("drunkenness[3]")?,
            ],
            sniper: reader.read_bool("sniper")?,
            sniper_padding: read_array(reader, "sniper_padding")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyNpcInitialPosition {
    pub x: f32,
    pub y: f32,
    /// Raw sector pointer echoed by the Original before the logical sector ID.
    pub raw_sector_pointer: LegacyOpaquePointer32,
    pub level: u16,
    pub padding: [u8; 2],
    pub sector: LegacySectorRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyDetectable {
    pub detectable_type: u32,
    pub seen_last: bool,
    pub seen_now: bool,
    pub shadow_seen_last: bool,
    pub heard_last: bool,
    pub visibility: f32,
    pub element: LegacyAiElementRef,
}

impl LegacyDetectable {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        read_fingerprint(
            reader,
            "fingerprint",
            FINGERPRINT_DETECTABLE,
            "RHdetectable",
        )?;
        Ok(Self {
            detectable_type: reader.read_u32("detectable_type")?,
            seen_last: reader.read_bool("seen_last")?,
            seen_now: reader.read_bool("seen_now")?,
            shadow_seen_last: reader.read_bool("shadow_seen_last")?,
            heard_last: reader.read_bool("heard_last")?,
            visibility: reader.read_f32("visibility")?,
            element: read_ai_element_ref(reader, "element")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyDetectableBucket {
    pub entries: Vec<LegacyDetectable>,
    pub suspect: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyNpcPayload {
    pub life: i16,
    pub arrows: u16,
    pub old_direction: i16,
    pub register: u16,
    pub attached_scroll: LegacyElementRef,
    pub inform: bool,
    pub money: u32,
    pub wasp: bool,
    pub body_visitors: u16,
    pub view: LegacyNpcView,
    pub mobile_target: LegacyElementRef,
    pub initial_position: LegacyNpcInitialPosition,
    pub initial_view: LegacyPoint2,
    pub fried: bool,
    pub local_ai: Box<LegacyLocalAiPayload>,
    pub old_deafness: u16,
    pub old_frame: u32,
    pub detectable_buckets: [LegacyDetectableBucket; 6],
    pub maximum_suspect: u16,
    pub worst_detectable_type: u16,
    pub custom_values: [i32; 10],
    pub gave_money: bool,
    pub human: LegacyHumanPayload,
}

impl LegacyNpcPayload {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacyPayloadLimits,
        context: &dyn LegacyPayloadDecodeContext,
        expected_creation_order: u32,
        expected_class: LegacyElementClass,
    ) -> LegacyResult<Self> {
        read_fingerprint(reader, "fingerprint", FINGERPRINT_NPC, "RHElementActorNPC")?;
        let life = reader.read_i16("life")?;
        let arrows = reader.read_u16("arrows")?;
        let old_direction = reader.read_i16("old_direction")?;
        let register = reader.read_u16("register")?;
        let attached_scroll = read_element_ref(reader, "attached_scroll")?;
        let inform = reader.read_bool("inform")?;
        let money = reader.read_u32("money")?;
        let wasp = reader.read_bool("wasp")?;
        let body_visitors = reader.read_u16("body_visitors")?;
        let view = reader.scope("view", LegacyNpcView::read)?;
        let mobile_target = read_element_ref(reader, "mobile_target")?;
        let initial_position = reader.scope("initial_position", |reader| {
            Ok(LegacyNpcInitialPosition {
                x: reader.read_f32("x")?,
                y: reader.read_f32("y")?,
                raw_sector_pointer: LegacyOpaquePointer32(reader.read_u32("raw_sector_pointer")?),
                level: reader.read_u16("level")?,
                padding: read_array(reader, "padding")?,
                sector: read_sector_ref(reader, "sector")?,
            })
        })?;
        let initial_view = read_point2(reader, "initial_view")?;
        let fried = reader.read_bool("fried")?;
        let local_ai = reader.scope("local_ai", |reader| {
            context.read_local_ai(reader, expected_creation_order, expected_class)
        })?;
        let old_deafness = reader.read_u16("old_deafness")?;
        let old_frame = reader.read_u32("old_frame")?;
        let mut buckets = Vec::with_capacity(6);
        for bucket_index in 0..6 {
            buckets.push(reader.scope(
                format!("detectable_buckets[{bucket_index}]"),
                |reader| {
                    let count =
                        read_bounded_u32(reader, "entries.count", limits.npc_detectables_per_type)?;
                    let mut entries = Vec::new();
                    reserve(reader, &mut entries, count, "entries")?;
                    for index in 0..count {
                        entries.push(
                            reader.scope(format!("entries[{index}]"), LegacyDetectable::read)?,
                        );
                    }
                    Ok(LegacyDetectableBucket {
                        entries,
                        suspect: reader.read_u16("suspect")?,
                    })
                },
            )?);
        }
        let buckets_offset = reader.offset();
        let detectable_buckets: [LegacyDetectableBucket; 6] =
            buckets.try_into().map_err(|values: Vec<_>| {
                reader.invalid_value(
                    buckets_offset,
                    "detectable_buckets",
                    values.len(),
                    "exactly six detectable buckets",
                )
            })?;
        let maximum_suspect = reader.read_u16("maximum_suspect")?;
        let worst_detectable_type = reader.read_u16("worst_detectable_type")?;
        let mut custom_values = [0; 10];
        for (index, value) in custom_values.iter_mut().enumerate() {
            *value = reader.read_i32(format_args!("custom_values[{index}]"))?;
        }
        let gave_money = reader.read_bool("gave_money")?;
        let human = reader.scope("human", |reader| {
            LegacyHumanPayload::read(
                reader,
                limits,
                context,
                expected_creation_order,
                expected_class,
            )
        })?;
        Ok(Self {
            life,
            arrows,
            old_direction,
            register,
            attached_scroll,
            inform,
            money,
            wasp,
            body_visitors,
            view,
            mobile_target,
            initial_position,
            initial_view,
            fried,
            local_ai,
            old_deafness,
            old_frame,
            detectable_buckets,
            maximum_suspect,
            worst_detectable_type,
            custom_values,
            gave_money,
            human,
        })
    }
}

fn read_fingerprint(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    expected: [u8; 16],
    description: &'static str,
) -> LegacyResult<()> {
    reader.read_signature(field, expected, description)
}

fn read_array<const N: usize>(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
) -> LegacyResult<[u8; N]> {
    let mut bytes = [0; N];
    reader.read_bytes(field, &mut bytes)?;
    Ok(bytes)
}

pub fn read_element_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyElementRef> {
    let raw = reader.read_u32(field)?;
    Ok(LegacyElementRef((raw != NULL_U32).then_some(raw)))
}

pub fn read_ai_element_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyAiElementRef> {
    const NULL_AI_ELEMENT: u16 = 54_321;
    let raw = reader.read_u16(field)?;
    Ok(LegacyAiElementRef((raw != NULL_AI_ELEMENT).then_some(raw)))
}

pub fn read_sequence_element_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacySequenceElementRef> {
    read_nonzero_u32_ref(reader, field).map(LegacySequenceElementRef)
}

pub fn read_sequence_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacySequenceRef> {
    read_nonzero_u32_ref(reader, field).map(LegacySequenceRef)
}

pub fn read_order_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyOrderRef> {
    read_nonzero_u32_ref(reader, field).map(LegacyOrderRef)
}

fn read_nonzero_u32_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<Option<u32>> {
    let field = field.to_string();
    let offset = reader.offset();
    let raw = reader.read_u32(field.as_str())?;
    match raw {
        NULL_U32 => Ok(None),
        0 => Err(reader.invalid_value(
            offset,
            field.as_str(),
            raw,
            "non-zero unique ID or 0xffffffff null sentinel",
        )),
        _ => Ok(Some(raw)),
    }
}

pub fn read_sector_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacySectorRef> {
    let raw = reader.read_u16(field)?;
    Ok(LegacySectorRef((raw != u16::MAX).then_some(raw)))
}

pub fn read_signed_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacySignedIndexRef> {
    let raw = reader.read_i16(field)?;
    Ok(LegacySignedIndexRef((raw != -1).then_some(raw)))
}

pub fn read_line_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyLineRef> {
    let field = field.to_string();
    let layer = reader.read_u16(format_args!("{field}.layer"))?;
    let index = reader.read_i16(format_args!("{field}.index"))?;
    Ok(LegacyLineRef {
        layer: (layer != u16::MAX).then_some(layer),
        index: (index != -1).then_some(index),
    })
}

fn read_point2(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyPoint2> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyPoint2 {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
        })
    })
}

fn read_point3(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyPoint3> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyPoint3 {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
            z: reader.read_f32("z")?,
        })
    })
}

fn read_box2(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyBoundingBox2> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyBoundingBox2 {
            top_left: read_point2(reader, "top_left")?,
            bottom_right: read_point2(reader, "bottom_right")?,
            bounds_are_set: reader.read_bool("bounds_are_set")?,
        })
    })
}

fn read_box3(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyBoundingBox3> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyBoundingBox3 {
            x_min: reader.read_f32("x_min")?,
            x_max: reader.read_f32("x_max")?,
            y_min: reader.read_f32("y_min")?,
            y_max: reader.read_f32("y_max")?,
            z_min: reader.read_f32("z_min")?,
            z_max: reader.read_f32("z_max")?,
        })
    })
}

fn read_plane3(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyPlane3> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyPlane3 {
            a: read_point3(reader, "a")?,
            b: read_point3(reader, "b")?,
            normal: read_point3(reader, "normal")?,
            origin: read_point3(reader, "origin")?,
            u: read_point3(reader, "u")?,
            v: read_point3(reader, "v")?,
            az: reader.read_f32("az")?,
            bz: reader.read_f32("bz")?,
            dz: reader.read_f32("dz")?,
            d: reader.read_f32("d")?,
        })
    })
}

fn read_bounded_u32(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display + Copy,
    maximum: usize,
) -> LegacyResult<usize> {
    reader.read_count_u32(field, maximum)
}

fn read_bounded_u16(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display + Copy,
    maximum: usize,
) -> LegacyResult<usize> {
    let offset = reader.offset();
    let raw = reader.read_u16(field)?;
    let count = usize::from(raw);
    if count > maximum {
        return Err(reader.invalid_value(
            offset,
            field,
            count,
            "item count within the caller-supplied limit",
        ));
    }
    Ok(count)
}

fn reserve<T>(
    reader: &mut LegacyReader<'_>,
    values: &mut Vec<T>,
    count: usize,
    field: &'static str,
) -> LegacyResult<()> {
    let offset = reader.offset();
    values
        .try_reserve_exact(count)
        .map_err(|_| reader.allocation_error(offset, field, count))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::sbfile::{SB_FILE_READ, SbFile};

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut fixture = NamedTempFile::new().unwrap();
        fixture.write_all(bytes).unwrap();
        fixture.flush().unwrap();
        let path = fixture.path().to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    #[test]
    fn reference_codecs_preserve_distinct_wire_id_spaces() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&17_u32.to_le_bytes());
        bytes.extend_from_slice(&54_321_u16.to_le_bytes());
        bytes.extend_from_slice(&23_u16.to_le_bytes());
        with_reader(&bytes, |reader| {
            assert_eq!(
                read_element_ref(reader, "engine_null").unwrap(),
                LegacyElementRef(None)
            );
            assert_eq!(
                read_element_ref(reader, "engine_value").unwrap(),
                LegacyElementRef(Some(17))
            );
            assert_eq!(
                read_ai_element_ref(reader, "ai_null").unwrap(),
                LegacyAiElementRef(None)
            );
            assert_eq!(
                read_ai_element_ref(reader, "ai_value").unwrap(),
                LegacyAiElementRef(Some(23))
            );
            assert_eq!(reader.offset(), bytes.len() as u64);
        });
    }

    #[test]
    fn unique_id_reference_rejects_zero() {
        with_reader(&0_u32.to_le_bytes(), |reader| {
            let error = read_sequence_element_ref(reader, "sequence").unwrap_err();
            assert_eq!(error.offset, 0);
            assert_eq!(error.field, "sequence");
            assert!(error.to_string().contains("non-zero unique ID"));
        });
    }

    #[test]
    fn bounded_u16_count_fails_before_allocation() {
        with_reader(&2_u16.to_le_bytes(), |reader| {
            let error = read_bounded_u16(reader, "items.count", 1).unwrap_err();
            assert_eq!(error.offset, 0);
            assert_eq!(error.field, "items.count");
            assert!(error.to_string().contains("caller-supplied limit"));
        });
    }

    #[test]
    fn detectable_truncation_reports_nested_reference_field() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FINGERPRINT_DETECTABLE);
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 0, 1, 0]);
        bytes.extend_from_slice(&0.5_f32.to_le_bytes());
        bytes.push(7); // first byte of the two-byte AI-local reference
        with_reader(&bytes, |reader| {
            let error = LegacyDetectable::read(reader).unwrap_err();
            assert_eq!(error.offset, 28);
            assert_eq!(error.field, "element");
        });
    }
}
