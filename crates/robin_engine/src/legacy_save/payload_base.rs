//! Common v48 phase-two element payload readers.
//!
//! Original leaf serializers interleave inherited serializers rather than
//! writing one uniform base-first prefix. Each structure here is therefore an
//! independently callable reader. Leaf readers must invoke it at the exact
//! point where the original-game serializer handles shared state.
//!
//! Field declaration order is wire order: the `LegacyRead` derives read the
//! fields top to bottom.

use super::read_helpers::hex16;
use super::read_helpers::{DEFAULT_BULK_LIMIT, DEFAULT_LIST_LIMIT};
use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyContext, LegacyRead, LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
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
            sprite_animation_replacements: DEFAULT_LIST_LIMIT,
            actor_bypass_points: DEFAULT_LIST_LIMIT,
            human_opponents: DEFAULT_LIST_LIMIT,
            human_sword_victims: DEFAULT_LIST_LIMIT,
            human_shoots: DEFAULT_LIST_LIMIT,
            npc_detectables_per_type: DEFAULT_BULK_LIMIT,
            mobile_sprites: DEFAULT_LIST_LIMIT,
            mobile_vibrations: DEFAULT_BULK_LIMIT,
            mobile_alerted_animals: DEFAULT_BULK_LIMIT,
            path_history: DEFAULT_BULK_LIMIT,
        }
    }
}

/// Mission-initialized metadata required by portions of the legacy grammar
/// that are not self-describing on disk.
pub trait LegacyPayloadDecodeContext {
    /// Number of embedded masked-effect parts already constructed for a
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

    /// Decode the full inline sequence body.
    fn read_inline_sequence(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<LegacyInlineSequence>;

    /// Decode one AI state payload.
    fn read_local_ai(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<Box<LegacyLocalAiPayload>>;
}

/// Decode context for [`LegacyElementPayloadBase`] and payloads that embed it:
/// the caller's limits plus the phase-one identity the element must match.
#[derive(Clone, Copy)]
pub struct LegacyElementBaseDecode<'a> {
    pub limits: &'a LegacyPayloadLimits,
    pub expected_creation_order: Option<u32>,
    pub expected_class: Option<LegacyElementClass>,
}

/// Decode context for actor-hierarchy and mobile leaves.
#[derive(Clone, Copy)]
pub struct LegacyLeafDecode<'a> {
    pub limits: &'a LegacyPayloadLimits,
    pub context: &'a dyn LegacyPayloadDecodeContext,
    pub creation_order: u32,
    pub class: LegacyElementClass,
}

impl LegacyLeafDecode<'_> {
    fn element(&self) -> LegacyElementBaseDecode<'_> {
        LegacyElementBaseDecode {
            limits: self.limits,
            expected_creation_order: Some(self.creation_order),
            expected_class: Some(self.class),
        }
    }
}

/// Decode context for the Human and NPC payloads, whose repulsive-point
/// geometry width depends on the producer ABI.
#[derive(Clone, Copy)]
pub struct LegacyHumanDecode<'a> {
    pub abi_profile: LegacySaveAbiProfile,
    pub leaf: LegacyLeafDecode<'a>,
}

impl<'a> LegacyHumanDecode<'a> {
    fn new(
        abi_profile: LegacySaveAbiProfile,
        limits: &'a LegacyPayloadLimits,
        context: &'a dyn LegacyPayloadDecodeContext,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> Self {
        Self {
            abi_profile,
            leaf: LegacyLeafDecode {
                limits,
                context,
                creation_order,
                class,
            },
        }
    }
}

/// Decode context for sprite and position-interface state. The engine-owned
/// projectile helper reads the same layout with its own limit and fingerprint
/// descriptions.
#[derive(Clone, Copy)]
pub struct LegacySpriteDecode {
    pub animation_replacements: usize,
    pub sprite_fingerprint: &'static str,
    pub position_fingerprint: &'static str,
}

impl LegacySpriteDecode {
    fn payload(limits: &LegacyPayloadLimits) -> Self {
        Self {
            animation_replacements: limits.sprite_animation_replacements,
            sprite_fingerprint: "sprite",
            position_fingerprint: "position interface",
        }
    }
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

/// Scalar references read exactly like the scalar they wrap.
macro_rules! scalar_ref_legacy_read {
    ($($ty:ty => $read:ident),* $(,)?) => {$(
        impl<C: ?Sized> LegacyRead<C> for $ty {
            fn read(reader: &mut LegacyReader<'_>, _: &C) -> LegacyResult<Self> {
                $read(reader, "")
            }

            fn read_field(
                reader: &mut LegacyReader<'_>,
                field: impl Into<LegacyContext>,
                _: &C,
            ) -> LegacyResult<Self> {
                $read(reader, field.into())
            }
        }
    )*};
}

scalar_ref_legacy_read!(
    LegacyElementRef => read_element_ref,
    LegacyAiElementRef => read_ai_element_ref,
    LegacySequenceElementRef => read_sequence_element_ref,
    LegacySequenceRef => read_sequence_ref,
    LegacyOrderRef => read_order_ref,
    LegacySectorRef => read_sector_ref,
    LegacySignedIndexRef => read_signed_ref,
    LegacyOpaquePointer32 => read_opaque_pointer32,
);

/// Reported as `field.layer` / `field.index`.
impl<C: ?Sized> LegacyRead<C> for LegacyLineRef {
    fn read(reader: &mut LegacyReader<'_>, _: &C) -> LegacyResult<Self> {
        let layer = reader.read_u16("layer")?;
        let index = reader.read_i16("index")?;
        Ok(Self {
            layer: (layer != u16::MAX).then_some(layer),
            index: (index != -1).then_some(index),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyPoint2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyPoint3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyBoundingBox2 {
    pub top_left: LegacyPoint2,
    pub bottom_right: LegacyPoint2,
    pub bounds_are_set: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyBoundingBox3 {
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
    pub z_min: f32,
    pub z_max: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyElementBaseDecode<'_>,
    fingerprint = FINGERPRINT_ELEMENT,
    expected = "element"
)]
pub struct LegacyElementPayloadBase {
    #[legacy(read = read_creation_order(reader, ctx.expected_creation_order))]
    pub creation_order: u32,
    pub outline_colors: [u16; 5],
    pub current_outline: u32,
    pub outline_width: u16,
    pub custom_minimap_dot: u16,
    pub active: bool,
    pub position_map_delayed: bool,
    pub position_delayed: bool,
    #[legacy(read = read_element_class(reader, ctx.expected_class))]
    pub class: LegacyElementClass,
    pub delayed_map_position: LegacyPoint2,
    pub delayed_position: LegacyPoint3,
    pub in_honolulu: bool,
    pub index_in_elements_list: u16,
    pub blipped: bool,
    pub unreachable: bool,
    #[legacy(read = LegacySpritePayload::read_field(
        reader,
        "sprite",
        &LegacySpriteDecode::payload(ctx.limits),
    ))]
    pub sprite: LegacySpritePayload,
}

impl LegacyElementPayloadBase {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacyPayloadLimits,
        expected_creation_order: Option<u32>,
        expected_class: Option<LegacyElementClass>,
    ) -> LegacyResult<Self> {
        <Self as LegacyRead<_>>::read(
            reader,
            &LegacyElementBaseDecode {
                limits,
                expected_creation_order,
                expected_class,
            },
        )
    }
}

fn read_creation_order(reader: &mut LegacyReader<'_>, expected: Option<u32>) -> LegacyResult<u32> {
    let offset = reader.offset();
    let creation_order = reader.read_u32("creation_order")?;
    if let Some(expected) = expected
        && creation_order != expected
    {
        return Err(reader.invalid_value(
            offset,
            "creation_order",
            creation_order,
            "creation order from the phase-one envelope",
        ));
    }
    Ok(creation_order)
}

fn read_element_class(
    reader: &mut LegacyReader<'_>,
    expected: Option<LegacyElementClass>,
) -> LegacyResult<LegacyElementClass> {
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
    if let Some(expected) = expected
        && class != expected
    {
        return Err(reader.invalid_value(
            class_offset,
            "class_id",
            format_args!("0x{raw_class:04x}"),
            "class id from the phase-one envelope",
        ));
    }
    Ok(class)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacySpriteDecode,
    fingerprint = FINGERPRINT_SPRITE,
    expected = ctx.sprite_fingerprint
)]
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
    #[legacy(read = read_animation_replacements(reader, ctx.animation_replacements))]
    pub animation_replacements: Vec<(u32, u32)>,
    pub position: LegacyPositionPayload,
}

fn read_animation_replacements(
    reader: &mut LegacyReader<'_>,
    maximum: usize,
) -> LegacyResult<Vec<(u32, u32)>> {
    let count = reader.read_count_u32("animation_replacements.count", maximum)?;
    reader.read_list("animation_replacements", count, |reader, item| {
        reader.scope(item, |reader| {
            Ok((reader.read_u32("from")?, reader.read_u32("to")?))
        })
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacySpriteDecode,
    fingerprint = FINGERPRINT_POSITION,
    expected = ctx.position_fingerprint
)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyElementBaseDecode<'_>,
    fingerprint = FINGERPRINT_FX,
    expected = "effect element"
)]
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
        <Self as LegacyRead<_>>::read(
            reader,
            &LegacyElementBaseDecode {
                limits,
                expected_creation_order,
                expected_class,
            },
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyPayloadLimits,
    fingerprint = FINGERPRINT_FX_MASKED,
    expected = "masked effect element"
)]
pub struct LegacyFxMaskedPayload {
    pub animation_speed: f32,
    #[legacy(read = LegacyElementPayloadBase::read_field(
        reader,
        "element",
        &LegacyElementBaseDecode {
            limits: ctx,
            expected_creation_order: None,
            expected_class: Some(LegacyElementClass::FxMasked),
        },
    ))]
    pub element: LegacyElementPayloadBase,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyPathHistoryEntry {
    pub position: LegacyPoint2,
    pub sector: LegacySectorRef,
    pub level: u16,
    pub direction: u8,
    pub distance: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyPayloadLimits,
    fingerprint = FINGERPRINT_PATH_STATUS,
    expected = "path status serialization"
)]
pub struct LegacyPathStatus {
    pub current_waypoint_index: u8,
    pub last_waypoint_index: u8,
    pub forward_movement: bool,
    #[legacy(with = read_nullable_u16)]
    pub hiking_path_index: Option<u16>,
    #[legacy(count_u16 = ctx.path_history)]
    pub history: Vec<LegacyPathHistoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyLeafDecode<'_>,
    fingerprint = FINGERPRINT_MOBILE,
    expected = "mobile element"
)]
pub struct LegacyMobilePayload {
    #[legacy(read = read_mobile_sprites(reader, ctx))]
    pub sprites: Vec<LegacyFxMaskedPayload>,
    pub stopped: bool,
    #[legacy(count_u32 = ctx.limits.mobile_vibrations)]
    pub vibrations: Vec<LegacyPoint2>,
    pub animation: u32,
    pub hook: LegacyPoint2,
    pub hooked_actor: LegacyElementRef,
    pub smoke_time: u32,
    pub smoke_delay: u32,
    pub relative_position: LegacyPoint2,
    #[legacy(read = LegacyPathStatus::read_field(reader, "path", ctx.limits))]
    pub path: LegacyPathStatus,
    pub on_waypoint: bool,
    #[legacy(read = if on_waypoint {
        read_nullable_u32_ref(reader, "waypoint_data_offset")
    } else {
        Ok(None)
    })]
    pub waypoint_data_offset: Option<u32>,
    #[legacy(when = on_waypoint)]
    pub waypoint_bytes_remaining: Option<u16>,
    pub wait_time: u32,
    pub speed: f32,
    pub speed_goal: f32,
    pub acceleration: f32,
    pub adaptive_speed: bool,
    pub front: LegacyPoint2,
    pub back: LegacyPoint2,
    #[legacy(count_u32 = ctx.limits.mobile_alerted_animals)]
    pub alerted_animals: Vec<LegacyElementRef>,
    pub steam_sound_1: i16,
    pub steam_sound_2: i16,
    pub steam: bool,
    pub brakes_sound: i16,
    pub brakes: bool,
    #[legacy(read = LegacyElementPayloadBase::read_field(reader, "element", &ctx.element()))]
    pub element: LegacyElementPayloadBase,
}

impl LegacyMobilePayload {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacyPayloadLimits,
        context: &dyn LegacyPayloadDecodeContext,
        expected_creation_order: u32,
    ) -> LegacyResult<Self> {
        <Self as LegacyRead<_>>::read(
            reader,
            &LegacyLeafDecode {
                limits,
                context,
                creation_order: expected_creation_order,
                class: LegacyElementClass::Mobile,
            },
        )
    }
}

fn read_mobile_sprites(
    reader: &mut LegacyReader<'_>,
    ctx: &LegacyLeafDecode<'_>,
) -> LegacyResult<Vec<LegacyFxMaskedPayload>> {
    let limits = ctx.limits;
    let sprite_count =
        ctx.context
            .mobile_sprite_count(reader, ctx.creation_order, limits.mobile_sprites)?;
    if sprite_count > limits.mobile_sprites {
        let offset = reader.offset();
        return Err(reader.invalid_value(
            offset,
            "sprites",
            sprite_count,
            "context sprite count within the caller-supplied limit",
        ));
    }
    reader.read_list("sprites", sprite_count, |reader, item| {
        LegacyFxMaskedPayload::read_field(reader, item, limits)
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyLeafDecode<'_>,
    fingerprint = FINGERPRINT_ACTOR,
    expected = "actor element"
)]
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
    #[legacy(read = read_post_seek_sequence(reader, ctx))]
    pub post_seek_sequence: Option<LegacyInlineSequence>,
    #[legacy(count_u16 = ctx.limits.actor_bypass_points)]
    pub bypass_points: Vec<LegacyPoint2>,
    pub script_class: String,
    #[legacy(read = read_actor_script_members(reader, ctx, &script_class))]
    pub script_members: Option<LegacyVmMemberSection>,
    #[legacy(read = read_actor_element(reader, ctx))]
    pub element: LegacyElementPayloadBase,
}

fn read_post_seek_sequence(
    reader: &mut LegacyReader<'_>,
    ctx: &LegacyLeafDecode<'_>,
) -> LegacyResult<Option<LegacyInlineSequence>> {
    if !reader.read_bool("has_post_seek_sequence")? {
        return Ok(None);
    }
    reader
        .scope("post_seek_sequence", |reader| {
            ctx.context
                .read_inline_sequence(reader, ctx.creation_order, ctx.class)
        })
        .map(Some)
}

fn read_actor_script_members(
    reader: &mut LegacyReader<'_>,
    ctx: &LegacyLeafDecode<'_>,
    script_class: &str,
) -> LegacyResult<Option<LegacyVmMemberSection>> {
    if script_class.is_empty() {
        return Ok(None);
    }
    reader
        .scope("script_members", |reader| {
            ctx.context.read_actor_script_members(
                reader,
                ctx.creation_order,
                ctx.class,
                script_class,
            )
        })
        .map(Some)
}

fn read_actor_element(
    reader: &mut LegacyReader<'_>,
    ctx: &LegacyLeafDecode<'_>,
) -> LegacyResult<LegacyElementPayloadBase> {
    // The Original intentionally brackets script state with the same
    // actor fingerprint.
    read_fingerprint(
        reader,
        "trailing_fingerprint",
        FINGERPRINT_ACTOR,
        "actor element",
    )?;
    LegacyElementPayloadBase::read_field(reader, "element", &ctx.element())
}

/// Context: whether the geometry uses the wide Windows retail layout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = bool)]
pub struct LegacyRepulsivePoint {
    #[legacy(read = read_geometry_point2(reader, "position", *ctx))]
    pub position: LegacyPoint2,
    pub concave: bool,
    #[legacy(read = read_geometry_point2(reader, "limit_left", *ctx))]
    pub limit_left: LegacyPoint2,
    #[legacy(read = read_geometry_point2(reader, "limit_right", *ctx))]
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

fn read_geometry_point2(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    wide_geometry: bool,
) -> LegacyResult<LegacyPoint2> {
    if wide_geometry {
        Ok(LegacyPoint2 {
            x: reader.read_f64(format_args!("{field}.x"))? as f32,
            y: reader.read_f64(format_args!("{field}.y"))? as f32,
        })
    } else {
        LegacyPoint2::read_field(reader, field, &())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyShieldPayload {
    pub points: [LegacyShieldPoint; 4],
    pub top_plane: LegacyPlane3,
    pub bottom_plane: LegacyPlane3,
    pub box_3d: LegacyBoundingBox3,
    pub ground_box: LegacyBoundingBox2,
    pub screen_box: LegacyBoundingBox2,
    pub on_ground: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyShieldPoint {
    #[legacy(read = read_shield_obstacle(reader))]
    pub obstacle: [f32; 4],
    pub polygon: LegacyPoint2,
}

fn read_shield_obstacle(reader: &mut LegacyReader<'_>) -> LegacyResult<[f32; 4]> {
    Ok([
        reader.read_f32("obstacle.x")?,
        reader.read_f32("obstacle.y")?,
        reader.read_f32("obstacle.z_top")?,
        reader.read_f32("obstacle.z_bottom")?,
    ])
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacySwordOpponent {
    pub opponent: LegacyElementRef,
    pub jump_line: LegacyLineRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyHumanDecode<'_>,
    fingerprint = FINGERPRINT_HUMAN,
    expected = "human actor element"
)]
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
    #[legacy(count_u16 = ctx.leaf.limits.human_opponents)]
    pub opponents: Vec<LegacySwordOpponent>,
    pub killed_by_accident: bool,
    pub running_hulk: u32,
    pub time_hulk: u32,
    pub hulk_level: u16,
    pub hulk_direction: bool,
    pub hulk_speed: f32,
    pub carrier: LegacyElementRef,
    #[legacy(read = LegacyRepulsivePoint::read_field(
        reader,
        "repulsive_point",
        &(ctx.abi_profile == LegacySaveAbiProfile::RetailWindowsX86V48),
    ))]
    pub repulsive_point: LegacyRepulsivePoint,
    pub small_repulsive_radius: bool,
    pub building: LegacySectorRef,
    /// The original game's enum serialization writes only the first
    /// four bytes of the noise record, which begin with the origin's X coordinate.
    pub currently_produced_noise_first_word: f32,
    #[legacy(read = LegacyActorPayload::read_field(reader, "actor", &ctx.leaf))]
    pub actor: LegacyActorPayload,
    pub shield: LegacyShieldPayload,
    #[legacy(count_u16 = ctx.leaf.limits.human_sword_victims)]
    pub sword_strike_victims: Vec<LegacyElementRef>,
    pub initial_strike_angle: f32,
    pub current_strike_angle: f32,
    pub final_strike_angle: f32,
    pub stuck_under_nets_counter: u16,
    pub sword_strike_boredom: [u16; 9],
    #[legacy(count_u32 = ctx.leaf.limits.human_shoots)]
    pub shoots: Vec<LegacySequenceElementRef>,
    pub smalltalk_hint: u32,
    pub hint_opponent: LegacyElementRef,
}

impl LegacyHumanPayload {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyPayloadLimits,
        context: &dyn LegacyPayloadDecodeContext,
        expected_creation_order: u32,
        expected_class: LegacyElementClass,
    ) -> LegacyResult<Self> {
        <Self as LegacyRead<_>>::read(
            reader,
            &LegacyHumanDecode::new(
                abi_profile,
                limits,
                context,
                expected_creation_order,
                expected_class,
            ),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyNpcView {
    pub leaning: bool,
    #[legacy(bytes)]
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
    #[legacy(bytes)]
    pub half_aperture_padding: [u8; 3],
    pub crazy_iterator: f32,
    pub crazy_iterator_step: f32,
    pub color: u8,
    #[legacy(bytes)]
    pub color_padding: [u8; 3],
    pub crazy_half_aperture: f32,
    pub direction: LegacyPoint2,
    pub left: LegacyPoint2,
    pub right: LegacyPoint2,
    pub stare: LegacyPoint2,
    /// Raw 32-bit host-reference echo from the original i386 save layout.
    pub raw_mobile_target_pointer: LegacyOpaquePointer32,
    pub radius_goal: u16,
    pub radius: u16,
    pub radius_reduction: u16,
    pub radius_step: u16,
    pub long_range: f32,
    pub real_radius: u16,
    #[legacy(bytes)]
    pub real_radius_padding: [u8; 2],
    pub drunkenness: [f32; 4],
    pub sniper: bool,
    #[legacy(bytes)]
    pub sniper_padding: [u8; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyNpcInitialPosition {
    pub x: f32,
    pub y: f32,
    /// Raw sector identity echoed by the original game before the logical sector ID.
    pub raw_sector_pointer: LegacyOpaquePointer32,
    pub level: u16,
    #[legacy(bytes)]
    pub padding: [u8; 2],
    pub sector: LegacySectorRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(fingerprint = FINGERPRINT_DETECTABLE, expected = "detectable")]
pub struct LegacyDetectable {
    pub detectable_type: u32,
    pub seen_last: bool,
    pub seen_now: bool,
    pub shadow_seen_last: bool,
    pub heard_last: bool,
    pub visibility: f32,
    pub element: LegacyAiElementRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyPayloadLimits)]
pub struct LegacyDetectableBucket {
    #[legacy(count_u32 = ctx.npc_detectables_per_type)]
    pub entries: Vec<LegacyDetectable>,
    pub suspect: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyHumanDecode<'_>,
    fingerprint = FINGERPRINT_NPC,
    expected = "NPC actor element"
)]
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
    #[legacy(read = reader.scope("local_ai", |reader| {
        ctx.leaf
            .context
            .read_local_ai(reader, ctx.leaf.creation_order, ctx.leaf.class)
    }))]
    pub local_ai: Box<LegacyLocalAiPayload>,
    pub old_deafness: u16,
    pub old_frame: u32,
    #[legacy(read = <[LegacyDetectableBucket; 6]>::read_field(
        reader,
        "detectable_buckets",
        ctx.leaf.limits,
    ))]
    pub detectable_buckets: [LegacyDetectableBucket; 6],
    pub maximum_suspect: u16,
    /// Despite the stored value's misleading prefix, the original game treats this as the
    /// 32-bit detectable-type value and serializes its raw storage.
    pub worst_detectable_type: u32,
    pub custom_values: [i32; 10],
    pub gave_money: bool,
    pub human: LegacyHumanPayload,
}

impl LegacyNpcPayload {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyPayloadLimits,
        context: &dyn LegacyPayloadDecodeContext,
        expected_creation_order: u32,
        expected_class: LegacyElementClass,
    ) -> LegacyResult<Self> {
        <Self as LegacyRead<_>>::read(
            reader,
            &LegacyHumanDecode::new(
                abi_profile,
                limits,
                context,
                expected_creation_order,
                expected_class,
            ),
        )
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

pub(super) fn read_element_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyElementRef> {
    read_nullable_u32_ref(reader, field).map(LegacyElementRef)
}

pub(super) fn read_ai_element_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyAiElementRef> {
    const NULL_AI_ELEMENT: u16 = 54_321;
    let raw = reader.read_u16(field)?;
    Ok(LegacyAiElementRef((raw != NULL_AI_ELEMENT).then_some(raw)))
}

pub(super) fn read_sequence_element_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacySequenceElementRef> {
    read_nonzero_u32_ref(reader, field).map(LegacySequenceElementRef)
}

pub(super) fn read_sequence_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacySequenceRef> {
    read_nullable_u32_ref(reader, field).map(LegacySequenceRef)
}

pub(super) fn read_order_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyOrderRef> {
    read_nullable_u32_ref(reader, field).map(LegacyOrderRef)
}

fn read_opaque_pointer32(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyOpaquePointer32> {
    reader.read_u32(field).map(LegacyOpaquePointer32)
}

/// `None` for the `0xffffffff` null sentinel.
pub(super) fn read_nullable_u32_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<Option<u32>> {
    let raw = reader.read_u32(field)?;
    Ok((raw != NULL_U32).then_some(raw))
}

/// `None` for the `0xffff` null sentinel.
pub(super) fn read_nullable_u16(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<Option<u16>> {
    let raw = reader.read_u16(field)?;
    Ok((raw != u16::MAX).then_some(raw))
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

pub(super) fn read_sector_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacySectorRef> {
    read_nullable_u16(reader, field).map(LegacySectorRef)
}

pub(super) fn read_signed_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacySignedIndexRef> {
    let raw = reader.read_i16(field)?;
    Ok(LegacySignedIndexRef((raw != -1).then_some(raw)))
}

pub(super) fn read_line_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyLineRef> {
    LegacyLineRef::read_field(reader, field.to_string(), &())
}

#[cfg(test)]
mod tests {

    use super::*;

    use crate::legacy_save::test_support::with_reader;

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
    fn order_reference_preserves_zero_as_a_non_null_id() {
        with_reader(&0_u32.to_le_bytes(), |reader| {
            assert_eq!(
                read_order_ref(reader, "order").unwrap(),
                LegacyOrderRef(Some(0))
            );
        });
    }

    #[test]
    fn bounded_u16_count_fails_before_allocation() {
        with_reader(&2_u16.to_le_bytes(), |reader| {
            let error = reader.read_count_u16("items.count", 1).unwrap_err();
            assert_eq!(error.offset, 0);
            assert_eq!(error.field, "items.count");
            assert!(error.to_string().contains("caller-supplied limit"));
        });
    }

    #[test]
    fn line_reference_reports_nested_layer_and_index_fields() {
        let mut bytes = u16::MAX.to_le_bytes().to_vec();
        bytes.push(0);
        with_reader(&bytes, |reader| {
            let error = reader
                .scope("opponents[2]", |reader| read_line_ref(reader, "jump_line"))
                .unwrap_err();
            assert_eq!(error.offset, 2);
            assert_eq!(error.field, "opponents[2].jump_line.index");
        });
    }

    #[test]
    fn windows_human_repulsive_point_reads_wide_geometry() {
        let mut bytes = Vec::new();
        for value in [1.25_f64, -2.5, 3.5, 4.5, 5.5, 6.5] {
            bytes.extend_from_slice(&value.to_le_bytes());
            if value == -2.5 {
                bytes.push(1);
            }
        }
        for value in [7.5_f32, 8.5, 9.5, 10.5] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&42_u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 0, 1, 0]);

        with_reader(&bytes, |reader| {
            let point = LegacyRepulsivePoint::read(reader, &true).unwrap();
            assert_eq!(point.position, LegacyPoint2 { x: 1.25, y: -2.5 });
            assert!(point.concave);
            assert_eq!(point.limit_left, LegacyPoint2 { x: 3.5, y: 4.5 });
            assert_eq!(point.limit_right, LegacyPoint2 { x: 5.5, y: 6.5 });
            assert_eq!(point.action_radius, 7.5);
            assert_eq!(point.force_a, 8.5);
            assert_eq!(point.force_b, 9.5);
            assert_eq!(point.radius, 10.5);
            assert_eq!(point.id, 42);
            assert!(point.affects_pcs);
            assert!(!point.affects_soldiers);
            assert!(point.affects_civilians);
            assert!(!point.affects_animals);
            assert_eq!(reader.offset(), bytes.len() as u64);
        });
    }

    #[test]
    fn wide_repulsive_point_truncation_reports_the_component_field() {
        let bytes = 1.25_f64.to_le_bytes();
        with_reader(&bytes, |reader| {
            let error =
                LegacyRepulsivePoint::read_field(reader, "repulsive_point", &true).unwrap_err();
            assert_eq!(error.offset, 8);
            assert_eq!(error.field, "repulsive_point.position.y");
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
            let error = LegacyDetectable::read(reader, &()).unwrap_err();
            assert_eq!(error.offset, 28);
            assert_eq!(error.field, "element");
        });
    }

    #[test]
    fn npc_view_padding_is_one_raw_field_and_drunkenness_is_indexed() {
        // leaning(1) + 2 of the 3 padding bytes: the raw read fails as a whole.
        with_reader(&[1, 0, 0], |reader| {
            let error = LegacyNpcView::read_field(reader, "view", &()).unwrap_err();
            assert_eq!(error.offset, 1);
            assert_eq!(error.field, "view.leaning_padding");
        });
        // Everything up to and including drunkenness[1].
        let prefix = 1 + 3 + 4 + 1 + 1 + 2 + 4 * 10 + 1 + 3 + 4 * 2 + 1 + 3 + 4 + 4 * 8;
        // raw pointer, four u16 radii, long range, real radius + padding,
        // drunkenness[0] and [1].
        let bytes = vec![0; prefix + 4 + 2 * 4 + 4 + 2 + 2 + 4 * 2];
        with_reader(&bytes, |reader| {
            let error = LegacyNpcView::read(reader, &()).unwrap_err();
            assert_eq!(error.offset, bytes.len() as u64);
            assert_eq!(error.field, "drunkenness[2]");
        });
    }
}
