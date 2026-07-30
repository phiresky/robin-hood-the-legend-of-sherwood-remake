//! Original v48 inline `RHSequence` payload decoding.
//!
//! This module implements the wire grammar used when the Original calls
//! `RHSequence::Serialize(file, false)`. Inline sequences deliberately do not
//! contain SequenceManager pre-serialized pointer IDs. The ID-bearing
//! structures are nevertheless shared wire-domain types so the manager-owned
//! form can add those three pointer fixups without redefining the payload.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::payload_base::{
    read_element_ref, read_line_ref, read_sector_ref, LegacyElementRef, LegacyLineRef,
    LegacyPoint2, LegacyPoint3, LegacySectorRef,
};

const FINGERPRINT_SEQUENCE: [u8; 16] = hex16("462542ef9f0ef300dff9647c2091d151");
const FINGERPRINT_SEQUENCE_ELEMENT: [u8; 16] = hex16("8358d2ae0236d0e6a448a02189c93b67");
const FINGERPRINT_ORDER: [u8; 16] = hex16("2000b559de6275b3d22859aac6522a56");
const NULL_U32: u32 = u32::MAX;

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
pub struct LegacySequencePayloadLimits {
    pub elements: usize,
    pub orders_per_element: usize,
    pub generic_fields: usize,
    pub nested_sequences: usize,
}

impl Default for LegacySequencePayloadLimits {
    fn default() -> Self {
        Self {
            elements: 65_535,
            orders_per_element: 65_535,
            generic_fields: 4096,
            nested_sequences: 256,
        }
    }
}

/// Unique-ID domain used by `RHSequenceManager::RegisterReadSequence`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyInlineSequenceId(pub u32);

/// Unique-ID domain used by `RHSequenceManager::RegisterReadSequenceElement`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyInlineSequenceElementId(pub u32);

/// Unique-ID domain used by `RHSequenceManager::RegisterReadOrder`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyInlineOrderId(pub u32);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyInlineSequence {
    pub started: bool,
    pub current_command_level: u16,
    pub running_elements: u16,
    pub sequence_element_cursor: u16,
    pub unique_id: LegacyInlineSequenceId,
    pub elements: Vec<LegacyInlineSequenceElement>,
    pub elements_in_progress: u16,
}

impl LegacyInlineSequence {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacySequencePayloadLimits,
    ) -> LegacyResult<Self> {
        reader.scope("RHSequence", |reader| read_sequence(reader, limits, 0))
    }
}

fn read_sequence(
    reader: &mut LegacyReader<'_>,
    limits: &LegacySequencePayloadLimits,
    nesting_depth: usize,
) -> LegacyResult<LegacyInlineSequence> {
    if nesting_depth > limits.nested_sequences {
        let offset = reader.offset();
        return Err(reader.invalid_value(
            offset,
            "nesting_depth",
            nesting_depth,
            "an inline sequence nesting depth within the caller-supplied limit",
        ));
    }

    reader.read_signature(
        "fingerprint",
        FINGERPRINT_SEQUENCE,
        "RHSequence fingerprint",
    )?;
    let started = reader.read_bool("started")?;
    let current_command_level = reader.read_u16("current_command_level")?;
    let running_elements = reader.read_u16("running_elements")?;
    let sequence_element_cursor = reader.read_u16("sequence_element_cursor")?;
    let unique_id = read_required_id(reader, "unique_id").map(LegacyInlineSequenceId)?;
    let count = reader.read_count_u32("elements.count", limits.elements)?;
    let mut elements = Vec::new();
    reserve(reader, &mut elements, count, "elements")?;
    let mut counted_in_progress = 0_usize;
    for index in 0..count {
        let element = reader.scope(format!("elements[{index}]"), |reader| {
            LegacyInlineSequenceElement::read(reader, limits, nesting_depth)
        })?;
        if element.base().state == 2 {
            counted_in_progress += 1;
        }
        elements.push(element);
    }
    let in_progress_offset = reader.offset();
    let elements_in_progress = reader.read_u16("elements_in_progress")?;
    if usize::from(elements_in_progress) != counted_in_progress {
        return Err(reader.invalid_value(
            in_progress_offset,
            "elements_in_progress",
            elements_in_progress,
            "the number of elements whose RHsequenceState is RHSEQ_INPROGRESS",
        ));
    }
    Ok(LegacyInlineSequence {
        started,
        current_command_level,
        running_elements,
        sequence_element_cursor,
        unique_id,
        elements,
        elements_in_progress,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LegacyInlineSequenceElement {
    Simple(LegacySequenceElementBase),
    Damage(LegacySequenceElementDamage),
    Generic(LegacySequenceElementGeneric),
    Interaction(LegacySequenceElementInteraction),
    Movement(LegacySequenceElementMovement),
}

impl LegacyInlineSequenceElement {
    fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacySequencePayloadLimits,
        nesting_depth: usize,
    ) -> LegacyResult<Self> {
        let type_offset = reader.offset();
        let element_type = reader.read_u8("type")?;
        match element_type {
            0 => LegacySequenceElementBase::read(reader, limits).map(Self::Simple),
            1 => LegacySequenceElementDamage::read(reader, limits).map(Self::Damage),
            2 => LegacySequenceElementGeneric::read(reader, limits).map(Self::Generic),
            3 => LegacySequenceElementInteraction::read(reader, limits).map(Self::Interaction),
            4 => LegacySequenceElementMovement::read(reader, limits, nesting_depth)
                .map(Self::Movement),
            _ => Err(reader.invalid_value(
                type_offset,
                "type",
                element_type,
                "SEQUENCEELEMENT_SIMPLE..=SEQUENCEELEMENT_MOVEMENT (0..=4)",
            )),
        }
    }

    pub fn base(&self) -> &LegacySequenceElementBase {
        match self {
            Self::Simple(value) => value,
            Self::Damage(value) => &value.base,
            Self::Generic(value) => &value.base,
            Self::Interaction(value) => &value.base,
            Self::Movement(value) => &value.base,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySequenceElementBase {
    pub command: i32,
    pub state: i32,
    pub command_level: u16,
    pub priority: i32,
    pub unique_id: LegacyInlineSequenceElementId,
    pub posture_after_transition: i32,
    pub action_state_after_transition: i32,
    pub deleted: bool,
    pub script_driven: bool,
    pub owner: LegacyElementRef,
    pub orders: Vec<LegacyInlineOrder>,
}

impl LegacySequenceElementBase {
    fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacySequencePayloadLimits,
    ) -> LegacyResult<Self> {
        reader.read_signature(
            "fingerprint",
            FINGERPRINT_SEQUENCE_ELEMENT,
            "RHSequenceElement::Serialize fingerprint",
        )?;
        let command = reader.read_i32("command")?;
        let state_offset = reader.offset();
        let state = reader.read_i32("state")?;
        if !(0..=6).contains(&state) {
            return Err(reader.invalid_value(
                state_offset,
                "state",
                state,
                "RHSEQ_TERMINATED..=RHSEQ_INTERRUPTED (0..=6)",
            ));
        }
        let command_level = reader.read_u16("command_level")?;
        let priority_offset = reader.offset();
        let priority = reader.read_i32("priority")?;
        if !(0..=11).contains(&priority) {
            return Err(reader.invalid_value(
                priority_offset,
                "priority",
                priority,
                "RHPRIORITY_NON_INTERRUPTABLE..=RHPRIORITY_NOT_YET_SET (0..=11)",
            ));
        }
        let unique_id = read_required_id(reader, "unique_id").map(LegacyInlineSequenceElementId)?;
        let posture_after_transition = reader.read_i32("posture_after_transition")?;
        let action_state_after_transition = reader.read_i32("action_state_after_transition")?;
        let deleted = reader.read_bool("deleted")?;
        // All supported saves are v48; the field was introduced in v40.
        let script_driven = reader.read_bool("script_driven")?;
        let owner = read_element_ref(reader, "owner")?;
        let order_count = reader.read_count_u32("orders.count", limits.orders_per_element)?;
        let mut orders = Vec::new();
        reserve(reader, &mut orders, order_count, "orders")?;
        for index in 0..order_count {
            orders.push(reader.scope(format!("orders[{index}]"), LegacyInlineOrder::read)?);
        }
        // bUsePreSerialization=false: no next, postponed, or mummy pointer IDs.
        Ok(Self {
            command,
            state,
            command_level,
            priority,
            unique_id,
            posture_after_transition,
            action_state_after_transition,
            deleted,
            script_driven,
            owner,
            orders,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyInlineOrder {
    pub action: i32,
    pub apply_transition_at_this_point: bool,
    pub compute_direction: bool,
    pub can_fly: bool,
    pub lock_ai: bool,
    pub reverse: bool,
    pub transition: bool,
    pub tolerance: f32,
    pub unique_id: LegacyInlineOrderId,
    pub destination_2d: LegacyPoint2,
    pub destination_3d: LegacyPoint3,
    pub flight_vector: LegacyPoint2,
    pub antagonist: LegacyElementRef,
}

impl LegacyInlineOrder {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        reader.read_signature("fingerprint", FINGERPRINT_ORDER, "RHOrder fingerprint")?;
        Ok(Self {
            action: reader.read_i32("action")?,
            apply_transition_at_this_point: reader.read_bool("apply_transition_at_this_point")?,
            compute_direction: reader.read_bool("compute_direction")?,
            can_fly: reader.read_bool("can_fly")?,
            lock_ai: reader.read_bool("lock_ai")?,
            reverse: reader.read_bool("reverse")?,
            transition: reader.read_bool("transition")?,
            tolerance: reader.read_f32("tolerance")?,
            // RHOrder::mulNextID starts at zero, so ID zero is valid in this
            // ID domain (unlike sequence and sequence-element IDs).
            unique_id: LegacyInlineOrderId(reader.read_u32("unique_id")?),
            destination_2d: read_point2(reader, "destination_2d")?,
            destination_3d: read_point3(reader, "destination_3d")?,
            flight_vector: read_point2(reader, "flight_vector")?,
            antagonist: read_element_ref(reader, "antagonist")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySequenceElementDamage {
    pub base: LegacySequenceElementBase,
    pub harder_hit: bool,
    pub sword_strike: i32,
    pub concussion: u16,
    pub damage: u16,
    pub origin: LegacyElementRef,
    pub sword: Option<LegacyHandToHandProfileRef>,
    pub arrow: LegacyElementRef,
}

impl LegacySequenceElementDamage {
    fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacySequencePayloadLimits,
    ) -> LegacyResult<Self> {
        let base = reader.scope("base", |reader| {
            LegacySequenceElementBase::read(reader, limits)
        })?;
        let harder_hit = reader.read_bool("harder_hit")?;
        let sword_strike = reader.read_i32("sword_strike")?;
        let concussion = reader.read_u16("concussion")?;
        let damage = reader.read_u16("damage")?;
        let origin = read_element_ref(reader, "origin")?;
        let sword = if reader.read_bool("sword.present")? {
            Some(LegacyHandToHandProfileRef(
                read_optional_index(reader, "sword.profile")?.ok_or_else(|| {
                    let offset = reader.offset().saturating_sub(4);
                    reader.invalid_value(
                        offset,
                        "sword.profile",
                        "0xffffffff",
                        "a non-null hand-to-hand profile index for a present RHSword",
                    )
                })?,
            ))
        } else {
            None
        };
        let arrow = read_element_ref(reader, "arrow")?;
        Ok(Self {
            base,
            harder_hit,
            sword_strike,
            concussion,
            damage,
            origin,
            sword,
            arrow,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyHandToHandProfileRef(pub u32);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySequenceElementInteraction {
    pub base: LegacySequenceElementBase,
    pub element: LegacyElementRef,
}

impl LegacySequenceElementInteraction {
    fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacySequencePayloadLimits,
    ) -> LegacyResult<Self> {
        Ok(Self {
            base: reader.scope("base", |reader| {
                LegacySequenceElementBase::read(reader, limits)
            })?,
            element: read_element_ref(reader, "element")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySequenceElementGeneric {
    pub base: LegacySequenceElementBase,
    pub fields: Vec<LegacyGenericField>,
}

impl LegacySequenceElementGeneric {
    fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacySequencePayloadLimits,
    ) -> LegacyResult<Self> {
        let base = reader.scope("base", |reader| {
            LegacySequenceElementBase::read(reader, limits)
        })?;
        let count = reader.read_count_u32("fields.count", limits.generic_fields)?;
        let mut fields = Vec::new();
        reserve(reader, &mut fields, count, "fields")?;
        for index in 0..count {
            fields.push(reader.scope(format!("fields[{index}]"), |reader| {
                LegacyGenericField::read(reader)
            })?);
        }
        Ok(Self { base, fields })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyGenericField {
    pub kind: LegacyGenericFieldKind,
    pub value: LegacyGenericFieldValue,
}

impl LegacyGenericField {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        let kind_offset = reader.offset();
        let raw_kind = reader.read_i32("kind")?;
        let kind = LegacyGenericFieldKind::from_wire(raw_kind).ok_or_else(|| {
            reader.invalid_value(
                kind_offset,
                "kind",
                raw_kind,
                "a v48 RHfield discriminant (0..=42)",
            )
        })?;
        let value = match kind.storage() {
            LegacyGenericFieldStorage::Element => {
                LegacyGenericFieldValue::Element(read_element_ref(reader, "value")?)
            }
            LegacyGenericFieldStorage::Line => {
                LegacyGenericFieldValue::Line(read_line_ref(reader, "value")?)
            }
            LegacyGenericFieldStorage::Gate => {
                LegacyGenericFieldValue::Gate(read_gate_ref(reader, "value")?)
            }
            LegacyGenericFieldStorage::Geo3 => LegacyGenericFieldValue::Geo3([
                reader.read_f32("value[0]")?,
                reader.read_f32("value[1]")?,
                reader.read_f32("value[2]")?,
            ]),
            LegacyGenericFieldStorage::RawUnion => {
                let mut bytes = [0; 12];
                reader.read_bytes("value", &mut bytes)?;
                LegacyGenericFieldValue::RawUnion12(bytes)
            }
        };
        Ok(Self { kind, value })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum LegacyGenericFieldKind {
    Direction = 0,
    Event = 1,
    Timer = 2,
    Message = 3,
    MessageArgument = 4,
    MessageExtendedArgument = 5,
    BowTargetGuy = 6,
    BowTargetPoint = 7,
    CameraPoint = 8,
    CameraZoomLevel = 9,
    CameraSpeed = 10,
    ActionId = 11,
    ActionAvailable = 12,
    CharacterAvailable = 13,
    ConcussionLevel = 14,
    SpeakId = 15,
    SpeakFlags = 16,
    SpeakVariant = 17,
    DialogId = 18,
    DialogSource = 19,
    PopupTextId = 20,
    AnimationId = 21,
    MapDisplay = 22,
    JumpLineSource = 23,
    JumpLineDestination = 24,
    Amount = 25,
    ShieldDangerPoint = 26,
    ShieldDangerPointLayer = 27,
    ShieldProtected = 28,
    RollPoint = 29,
    PurseTarget = 30,
    NetTarget = 31,
    WaspNestTarget = 32,
    Opponent = 33,
    SwordfightPrepared = 34,
    Gate = 35,
    Door = 36,
    OldAnimation = 37,
    NewAnimation = 38,
    Freeze = 39,
    Scroll = 40,
    ScrollReader = 41,
    ScrollOwner = 42,
}

impl LegacyGenericFieldKind {
    fn from_wire(value: i32) -> Option<Self> {
        Some(match value {
            0 => Self::Direction,
            1 => Self::Event,
            2 => Self::Timer,
            3 => Self::Message,
            4 => Self::MessageArgument,
            5 => Self::MessageExtendedArgument,
            6 => Self::BowTargetGuy,
            7 => Self::BowTargetPoint,
            8 => Self::CameraPoint,
            9 => Self::CameraZoomLevel,
            10 => Self::CameraSpeed,
            11 => Self::ActionId,
            12 => Self::ActionAvailable,
            13 => Self::CharacterAvailable,
            14 => Self::ConcussionLevel,
            15 => Self::SpeakId,
            16 => Self::SpeakFlags,
            17 => Self::SpeakVariant,
            18 => Self::DialogId,
            19 => Self::DialogSource,
            20 => Self::PopupTextId,
            21 => Self::AnimationId,
            22 => Self::MapDisplay,
            23 => Self::JumpLineSource,
            24 => Self::JumpLineDestination,
            25 => Self::Amount,
            26 => Self::ShieldDangerPoint,
            27 => Self::ShieldDangerPointLayer,
            28 => Self::ShieldProtected,
            29 => Self::RollPoint,
            30 => Self::PurseTarget,
            31 => Self::NetTarget,
            32 => Self::WaspNestTarget,
            33 => Self::Opponent,
            34 => Self::SwordfightPrepared,
            35 => Self::Gate,
            36 => Self::Door,
            37 => Self::OldAnimation,
            38 => Self::NewAnimation,
            39 => Self::Freeze,
            40 => Self::Scroll,
            41 => Self::ScrollReader,
            42 => Self::ScrollOwner,
            _ => return None,
        })
    }

    fn storage(self) -> LegacyGenericFieldStorage {
        match self {
            Self::BowTargetGuy
            | Self::ShieldProtected
            | Self::Opponent
            | Self::ScrollReader
            | Self::ScrollOwner
            | Self::Scroll => LegacyGenericFieldStorage::Element,
            Self::JumpLineSource | Self::JumpLineDestination => LegacyGenericFieldStorage::Line,
            Self::Door | Self::Gate => LegacyGenericFieldStorage::Gate,
            Self::MapDisplay
            | Self::ActionAvailable
            | Self::SwordfightPrepared
            | Self::Freeze
            | Self::CharacterAvailable => LegacyGenericFieldStorage::RawUnion,
            _ => LegacyGenericFieldStorage::Geo3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LegacyGenericFieldStorage {
    Element,
    Line,
    Gate,
    Geo3,
    RawUnion,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LegacyGenericFieldValue {
    Element(LegacyElementRef),
    Line(LegacyLineRef),
    Gate(LegacyGateRef),
    Geo3([f32; 3]),
    /// Full i386 `RHfieldValue` union bytes. Only the first byte is meaningful
    /// for the boolean-like fields, but the Original writes all 12 bytes.
    RawUnion12([u8; 12]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyGateRef(pub Option<i16>);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySequenceElementMovement {
    pub base: LegacySequenceElementBase,
    pub action: i32,
    pub tolerance: f32,
    pub direction: i16,
    pub flags: u32,
    pub layer: u16,
    pub speed_factor: f32,
    pub destination: LegacyPoint2,
    pub element: LegacyElementRef,
    pub gate: LegacyGateRef,
    pub line: LegacyLineRef,
    pub sector: LegacySectorRef,
    pub post_seek_sequence: Option<Box<LegacyInlineSequence>>,
}

impl LegacySequenceElementMovement {
    fn read(
        reader: &mut LegacyReader<'_>,
        limits: &LegacySequencePayloadLimits,
        nesting_depth: usize,
    ) -> LegacyResult<Self> {
        let base = reader.scope("base", |reader| {
            LegacySequenceElementBase::read(reader, limits)
        })?;
        let action = reader.read_i32("action")?;
        let tolerance = reader.read_f32("tolerance")?;
        let direction = reader.read_i16("direction")?;
        let flags = reader.read_u32("flags")?;
        let layer = reader.read_u16("layer")?;
        let speed_factor = reader.read_f32("speed_factor")?;
        let destination = read_point2(reader, "destination")?;
        let element = read_element_ref(reader, "element")?;
        let gate = read_gate_ref(reader, "gate")?;
        let line = read_line_ref(reader, "line")?;
        let sector = read_sector_ref(reader, "sector")?;
        let post_seek_sequence = if reader.read_bool("post_seek_sequence.present")? {
            Some(Box::new(reader.scope("post_seek_sequence", |reader| {
                read_sequence(reader, limits, nesting_depth + 1)
            })?))
        } else {
            None
        };
        // bUsePreSerialization=false: no linked-seek element pointer ID.
        Ok(Self {
            base,
            action,
            tolerance,
            direction,
            flags,
            layer,
            speed_factor,
            destination,
            element,
            gate,
            line,
            sector,
            post_seek_sequence,
        })
    }
}

fn read_required_id(reader: &mut LegacyReader<'_>, field: &'static str) -> LegacyResult<u32> {
    let offset = reader.offset();
    let value = reader.read_u32(field)?;
    if value == 0 || value == NULL_U32 {
        Err(reader.invalid_value(offset, field, value, "a non-zero, non-null unique ID"))
    } else {
        Ok(value)
    }
}

fn read_optional_index(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
) -> LegacyResult<Option<u32>> {
    let value = reader.read_u32(field)?;
    Ok((value != NULL_U32).then_some(value))
}

fn read_gate_ref(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyGateRef> {
    let value = reader.read_i16(field)?;
    Ok(LegacyGateRef((value != -1).then_some(value)))
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
    use crate::sbfile::{SbFile, SB_FILE_READ};

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut fixture = NamedTempFile::new().unwrap();
        fixture.write_all(bytes).unwrap();
        fixture.flush().unwrap();
        let path = fixture.path().to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_empty_sequence(bytes: &mut Vec<u8>, id: u32) {
        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE);
        bytes.push(0);
        push_u16(bytes, 0);
        push_u16(bytes, 0);
        push_u16(bytes, 0);
        push_u32(bytes, id);
        push_u32(bytes, 0);
        push_u16(bytes, 0);
    }

    fn push_base(bytes: &mut Vec<u8>, id: u32, state: i32) {
        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE_ELEMENT);
        push_i32(bytes, 0);
        push_i32(bytes, state);
        push_u16(bytes, 0);
        push_i32(bytes, 8);
        push_u32(bytes, id);
        push_i32(bytes, 0);
        push_i32(bytes, 0);
        bytes.push(0);
        bytes.push(0);
        push_u32(bytes, u32::MAX);
        push_u32(bytes, 0);
    }

    fn push_base_with_one_order(bytes: &mut Vec<u8>, id: u32) {
        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE_ELEMENT);
        push_i32(bytes, 0);
        push_i32(bytes, 0);
        push_u16(bytes, 0);
        push_i32(bytes, 8);
        push_u32(bytes, id);
        push_i32(bytes, 0);
        push_i32(bytes, 0);
        bytes.push(0);
        bytes.push(0);
        push_u32(bytes, u32::MAX);
        push_u32(bytes, 1);

        bytes.extend_from_slice(&FINGERPRINT_ORDER);
        push_i32(bytes, 17);
        bytes.extend_from_slice(&[1, 0, 1, 0, 1, 0]);
        push_f32(bytes, 2.5);
        push_u32(bytes, 0);
        for value in [1.0_f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0] {
            push_f32(bytes, value);
        }
        push_u32(bytes, 22);
    }

    fn wrap_one(element_type: u8, body: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE);
        bytes.push(1);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 1);
        push_u32(&mut bytes, 1);
        bytes.push(element_type);
        body(&mut bytes);
        push_u16(&mut bytes, 0);
        bytes
    }

    #[test]
    fn reads_empty_inline_sequence_at_exact_boundary() {
        let mut bytes = Vec::new();
        push_empty_sequence(&mut bytes, 7);
        with_reader(&bytes, |reader| {
            let sequence =
                LegacyInlineSequence::read(reader, &LegacySequencePayloadLimits::default())
                    .unwrap();
            assert_eq!(sequence.unique_id, LegacyInlineSequenceId(7));
            assert!(sequence.elements.is_empty());
            assert_eq!(reader.offset(), bytes.len() as u64);
        });
    }

    #[test]
    fn reads_all_five_element_subtypes() {
        let fixtures = [
            wrap_one(0, |bytes| push_base(bytes, 10, 0)),
            wrap_one(1, |bytes| {
                push_base(bytes, 11, 0);
                bytes.push(1);
                push_i32(bytes, 2);
                push_u16(bytes, 3);
                push_u16(bytes, 4);
                push_u32(bytes, u32::MAX);
                bytes.push(0);
                push_u32(bytes, u32::MAX);
            }),
            wrap_one(2, |bytes| {
                push_base(bytes, 12, 0);
                push_u32(bytes, 1);
                push_i32(bytes, LegacyGenericFieldKind::Direction as i32);
                push_f32(bytes, 1.0);
                push_f32(bytes, 2.0);
                push_f32(bytes, 3.0);
            }),
            wrap_one(3, |bytes| {
                push_base(bytes, 13, 0);
                push_u32(bytes, 44);
            }),
            wrap_one(4, |bytes| {
                push_base(bytes, 14, 0);
                push_i32(bytes, 0);
                push_f32(bytes, 1.5);
                bytes.extend_from_slice(&(-1_i16).to_le_bytes());
                push_u32(bytes, 0);
                push_u16(bytes, 0);
                push_f32(bytes, 1.0);
                push_f32(bytes, 10.0);
                push_f32(bytes, 20.0);
                push_u32(bytes, u32::MAX);
                bytes.extend_from_slice(&(-1_i16).to_le_bytes());
                push_u16(bytes, u16::MAX);
                bytes.extend_from_slice(&(-1_i16).to_le_bytes());
                push_u16(bytes, u16::MAX);
                bytes.push(1);
                push_empty_sequence(bytes, 15);
            }),
        ];
        for (index, bytes) in fixtures.iter().enumerate() {
            with_reader(bytes, |reader| {
                let sequence =
                    LegacyInlineSequence::read(reader, &LegacySequencePayloadLimits::default())
                        .unwrap_or_else(|error| panic!("fixture {index}: {error}"));
                assert_eq!(sequence.elements.len(), 1);
                assert_eq!(reader.offset(), bytes.len() as u64);
            });
        }
    }

    #[test]
    fn rejects_unknown_u8_element_type_before_body() {
        let bytes = wrap_one(5, |_| {});
        with_reader(&bytes, |reader| {
            let error = LegacyInlineSequence::read(reader, &LegacySequencePayloadLimits::default())
                .unwrap_err();
            assert!(error.to_string().contains("0..=4"));
        });
    }

    #[test]
    fn reads_order_id_zero_and_stops_at_exact_boundary() {
        let bytes = wrap_one(0, |bytes| push_base_with_one_order(bytes, 10));
        with_reader(&bytes, |reader| {
            let sequence =
                LegacyInlineSequence::read(reader, &LegacySequencePayloadLimits::default())
                    .unwrap();
            let order = &sequence.elements[0].base().orders[0];
            assert_eq!(order.unique_id, LegacyInlineOrderId(0));
            assert_eq!(order.antagonist, LegacyElementRef(Some(22)));
            assert_eq!(reader.offset(), bytes.len() as u64);
        });
    }

    #[test]
    fn rejects_mismatched_in_progress_count() {
        let mut bytes = wrap_one(0, |bytes| push_base(bytes, 10, 2));
        let length = bytes.len();
        bytes[length - 2..].copy_from_slice(&0_u16.to_le_bytes());
        with_reader(&bytes, |reader| {
            let error = LegacyInlineSequence::read(reader, &LegacySequencePayloadLimits::default())
                .unwrap_err();
            assert!(error.to_string().contains("elements_in_progress"));
        });
    }
}
