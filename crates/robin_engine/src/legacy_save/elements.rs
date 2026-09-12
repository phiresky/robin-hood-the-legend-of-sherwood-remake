//! Phase-one envelope decoder for the original game's v48 element records.
//!
//! The Original writes the element count followed by two passes over the same
//! creation-order-sorted element list. This module deliberately consumes only
//! the first pass:
//!
//! ```text
//! u32 element_count
//! repeated element_count times:
//!     u16 class_id
//!     u32 creation_order
//!     u32 pc_description_index  // only for RHCLASSID_ACTOR_PC
//! ```
//!
//! Class payloads begin in pass two at [`LegacyElementEnvelope::phase2_offset`]
//! and are a later importer milestone.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

const NULL_ELEMENT_CREATION_ORDER: u32 = u32::MAX;

/// Caller-owned safety and identity context for a phase-one decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyElementReadConfig {
    /// Hard allocation bound for the serialized element count.
    pub maximum_elements: usize,
    /// Length of the already-decoded live campaign character table.
    pub campaign_character_count: usize,
    /// Static-element creation count from the engine prefix.
    ///
    /// Creation orders below this value are resolved against mission-created
    /// static elements. Orders at or above it must be constructible by the
    /// dynamic-element factory in the legacy loader.
    pub static_creation_order_boundary: u32,
}

/// The complete phase-one envelope and the byte where phase two begins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyElementEnvelope {
    pub start_offset: u64,
    pub phase2_offset: u64,
    pub records: Vec<LegacyElementRecord>,
    /// Identity table used by phase-two payload pointers and later fixups.
    pub fixups: LegacyElementFixupTable,
}

impl LegacyElementEnvelope {
    pub fn read_phase1(
        reader: &mut LegacyReader<'_>,
        config: &LegacyElementReadConfig,
    ) -> LegacyResult<Self> {
        let start_offset = reader.offset();
        let count_offset = reader.offset();
        let count = reader.read_count_u32("element_count", config.maximum_elements)?;
        let mut records = Vec::new();
        records
            .try_reserve_exact(count)
            .map_err(|_| reader.allocation_error(count_offset, "elements", count))?;
        let mut by_creation_order = BTreeMap::new();
        let mut previous_creation_order = None;

        for slot in 0..count {
            let record = reader.scope(format!("elements[{slot}]"), |reader| {
                LegacyElementRecord::read(reader, slot, config)
            })?;

            if let Some(previous) = previous_creation_order
                && record.creation_order <= previous
            {
                return Err(reader.invalid_value(
                    record.creation_order_offset,
                    format_args!("elements[{slot}].creation_order"),
                    record.creation_order,
                    "unique creation order greater than the preceding phase-one record",
                ));
            }
            previous_creation_order = Some(record.creation_order);

            // The strict ordering check above makes replacement impossible;
            // retain this assertion as a local invariant of the fixup table.
            let replaced_slot = by_creation_order.insert(record.creation_order, record.slot);
            debug_assert!(replaced_slot.is_none());
            records.push(record);
        }

        let phase2_offset = reader.offset();
        Ok(Self {
            start_offset,
            phase2_offset,
            records,
            fixups: LegacyElementFixupTable { by_creation_order },
        })
    }
}

/// Stable identity lookup prepared before any phase-two pointer is decoded.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyElementFixupTable {
    pub by_creation_order: BTreeMap<u32, usize>,
}

impl LegacyElementFixupTable {
    pub fn slot_for_creation_order(&self, creation_order: u32) -> Option<usize> {
        self.by_creation_order.get(&creation_order).copied()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyElementRecord {
    pub slot: usize,
    pub class: LegacyElementClass,
    pub creation_order: u32,
    pub pc_description_index: Option<u32>,
    pub resolution: LegacyElementResolution,
    /// Exact offset of the creation-order identity, retained for later
    /// diagnostics without retaining the source reader.
    pub creation_order_offset: u64,
}

impl LegacyElementRecord {
    fn read(
        reader: &mut LegacyReader<'_>,
        slot: usize,
        config: &LegacyElementReadConfig,
    ) -> LegacyResult<Self> {
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

        let creation_order_offset = reader.offset();
        let creation_order = reader.read_u32("creation_order")?;
        if creation_order == NULL_ELEMENT_CREATION_ORDER {
            return Err(reader.invalid_value(
                creation_order_offset,
                "creation_order",
                "0xffffffff",
                "non-null element creation order",
            ));
        }
        let pc_description_index = if class == LegacyElementClass::ActorPc {
            let index_offset = reader.offset();
            let index = reader.read_u32("pc_description_index")?;
            if index as usize >= config.campaign_character_count {
                return Err(reader.invalid_value(
                    index_offset,
                    "pc_description_index",
                    index,
                    "index into the decoded live campaign character table",
                ));
            }
            Some(index)
        } else {
            None
        };

        let resolution = if creation_order < config.static_creation_order_boundary {
            LegacyElementResolution::ResolveStatic {
                fallback_factory: class.dynamic_factory(),
            }
        } else {
            let Some(factory) = class.dynamic_factory() else {
                return Err(reader.invalid_value(
                    class_offset,
                    "class_id",
                    format_args!("0x{raw_class:04x} ({class:?})"),
                    "class supported by legacy dynamic-element loading",
                ));
            };
            LegacyElementResolution::ConstructDynamic { factory }
        };

        Ok(Self {
            slot,
            class,
            creation_order,
            pc_description_index,
            resolution,
            creation_order_offset,
        })
    }
}

/// Repeated identity prefix immediately before one phase-two class payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyElementPayloadHeader {
    pub class: LegacyElementClass,
    pub creation_order: u32,
    pub pc_description_index: Option<u32>,
}

/// Consume and validate the repeated phase-two identity header for one
/// phase-one record, leaving the reader at that element's class payload.
///
/// Future class decoders should call this before reading any payload fields.
pub fn read_payload_header(
    reader: &mut LegacyReader<'_>,
    expected: &LegacyElementRecord,
    campaign_character_count: usize,
) -> LegacyResult<LegacyElementPayloadHeader> {
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
    if class != expected.class {
        return Err(reader.invalid_value(
            class_offset,
            "class_id",
            format_args!("0x{raw_class:04x} ({class:?})"),
            "same class id as the corresponding phase-one record",
        ));
    }

    let creation_order_offset = reader.offset();
    let creation_order = reader.read_u32("creation_order")?;
    if creation_order != expected.creation_order {
        return Err(reader.invalid_value(
            creation_order_offset,
            "creation_order",
            creation_order,
            "same creation order as the corresponding phase-one record",
        ));
    }

    let pc_description_index = if class == LegacyElementClass::ActorPc {
        let index_offset = reader.offset();
        let index = reader.read_u32("pc_description_index")?;
        if index as usize >= campaign_character_count {
            return Err(reader.invalid_value(
                index_offset,
                "pc_description_index",
                index,
                "index into the decoded live campaign character table",
            ));
        }
        if Some(index) != expected.pc_description_index {
            return Err(reader.invalid_value(
                index_offset,
                "pc_description_index",
                index,
                "same PC description index as the corresponding phase-one record",
            ));
        }
        Some(index)
    } else {
        None
    };

    Ok(LegacyElementPayloadHeader {
        class,
        creation_order,
        pc_description_index,
    })
}

/// First-pass action to take once the mission's static identity table exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyElementResolution {
    /// Match an existing mission-created element by creation order and class.
    /// `fallback_factory` mirrors the Original's behavior if no static match
    /// exists; `None` means absence is structural corruption.
    ResolveStatic {
        fallback_factory: Option<LegacyDynamicElementFactory>,
    },
    /// Create an on-the-fly element before phase-two payload fixups.
    ConstructDynamic {
        factory: LegacyDynamicElementFactory,
    },
}

/// Element kinds supported by the v48 save format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyDynamicElementFactory {
    ActorPc = 0x0001,
    Apple = 0x3002,
    Arrow = 0x3001,
    Coin = 0x3008,
    Net = 0x3007,
    Purse = 0x3003,
    WaspNest = 0x3005,
    Wasp = 0x3006,
    Stone = 0x3004,
    Ale = 0x3009,
    SpyCape = 0x300a,
    BonusAle = 0x4001,
    BonusArrow = 0x4003,
    BonusApple = 0x4004,
    BonusLambLeg = 0x4006,
    BonusNet = 0x4007,
    BonusPlants = 0x4008,
    BonusPurse = 0x4009,
    BonusStone = 0x400a,
    BonusWaspNest = 0x400b,
    Scroll = 0x400c,
    BonusAmulet = 0x4002,
    BonusRansom = 0x400d,
    BonusAmpulla = 0x400e,
    BonusCoronationSpoon = 0x400f,
    BonusRichardsCrown = 0x4010,
    BonusRoyalSeal = 0x4011,
    BonusRoyalSceptre = 0x4012,
    BonusDomesdayBook = 0x4013,
    BonusSwordOfTheState = 0x4014,
}

/// Concrete type identifiers accepted in an original-game v48 element table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u16)]
#[derive(num_enum::TryFromPrimitive, num_enum::IntoPrimitive)]
pub enum LegacyElementClass {
    ActorPc,
    ActorNpc = 0x0101,
    ActorNpcCivilian = 0x0102,
    ActorNpcSoldier = 0x0103,
    ActorAnimal = 0x0210,
    ActorDog = 0x0211,
    ActorCow = 0x0212,
    ActorHen = 0x0213,
    ActorPig = 0x0214,
    ActorBird = 0x0215,
    ActorCrocodile = 0x0216,
    Object = 0x0301,
    Arrow,
    Apple,
    Purse,
    Stone,
    WaspNest,
    Wasp,
    Net,
    Coin,
    Ale,
    SpyCape,
    BonusAle,
    BonusAmulet,
    BonusArrow,
    BonusApple,
    BonusBlazon = 0x4005,
    BonusLambLeg,
    BonusNet,
    BonusPlants,
    BonusPurse,
    BonusStone,
    BonusWaspNest,
    Scroll,
    BonusRansom,
    BonusAmpulla,
    BonusCoronationSpoon,
    BonusRichardsCrown,
    BonusRoyalSeal,
    BonusRoyalSceptre,
    BonusDomesdayBook,
    BonusSwordOfTheState,
    Target = 0x0800,
    Fx = 0x1001,
    FxMasked = 0x1002,
    Mobile = 0x1003,
}

impl LegacyElementClass {
    pub fn from_raw(value: u16) -> Option<Self> {
        Self::try_from(value).ok()
    }

    pub fn raw(self) -> u16 {
        self.into()
    }

    pub fn dynamic_factory(self) -> Option<LegacyDynamicElementFactory> {
        Some(match self {
            Self::ActorPc => LegacyDynamicElementFactory::ActorPc,
            Self::Apple => LegacyDynamicElementFactory::Apple,
            Self::Arrow => LegacyDynamicElementFactory::Arrow,
            Self::Coin => LegacyDynamicElementFactory::Coin,
            Self::Net => LegacyDynamicElementFactory::Net,
            Self::Purse => LegacyDynamicElementFactory::Purse,
            Self::WaspNest => LegacyDynamicElementFactory::WaspNest,
            Self::Wasp => LegacyDynamicElementFactory::Wasp,
            Self::Stone => LegacyDynamicElementFactory::Stone,
            Self::Ale => LegacyDynamicElementFactory::Ale,
            Self::SpyCape => LegacyDynamicElementFactory::SpyCape,
            Self::BonusAle => LegacyDynamicElementFactory::BonusAle,
            Self::BonusArrow => LegacyDynamicElementFactory::BonusArrow,
            Self::BonusApple => LegacyDynamicElementFactory::BonusApple,
            Self::BonusLambLeg => LegacyDynamicElementFactory::BonusLambLeg,
            Self::BonusNet => LegacyDynamicElementFactory::BonusNet,
            Self::BonusPlants => LegacyDynamicElementFactory::BonusPlants,
            Self::BonusPurse => LegacyDynamicElementFactory::BonusPurse,
            Self::BonusStone => LegacyDynamicElementFactory::BonusStone,
            Self::BonusWaspNest => LegacyDynamicElementFactory::BonusWaspNest,
            Self::Scroll => LegacyDynamicElementFactory::Scroll,
            Self::BonusAmulet => LegacyDynamicElementFactory::BonusAmulet,
            Self::BonusRansom => LegacyDynamicElementFactory::BonusRansom,
            Self::BonusAmpulla => LegacyDynamicElementFactory::BonusAmpulla,
            Self::BonusCoronationSpoon => LegacyDynamicElementFactory::BonusCoronationSpoon,
            Self::BonusRichardsCrown => LegacyDynamicElementFactory::BonusRichardsCrown,
            Self::BonusRoyalSeal => LegacyDynamicElementFactory::BonusRoyalSeal,
            Self::BonusRoyalSceptre => LegacyDynamicElementFactory::BonusRoyalSceptre,
            Self::BonusDomesdayBook => LegacyDynamicElementFactory::BonusDomesdayBook,
            Self::BonusSwordOfTheState => LegacyDynamicElementFactory::BonusSwordOfTheState,
            Self::ActorNpc
            | Self::ActorNpcCivilian
            | Self::ActorNpcSoldier
            | Self::ActorAnimal
            | Self::ActorDog
            | Self::ActorCow
            | Self::ActorHen
            | Self::ActorPig
            | Self::ActorBird
            | Self::ActorCrocodile
            | Self::Object
            | Self::BonusBlazon
            | Self::Target
            | Self::Fx
            | Self::FxMasked
            | Self::Mobile => return None,
        })
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::legacy_io::LegacyIoErrorKind;
    use crate::sbfile::SbFile;

    use crate::legacy_save::test_support::with_reader;

    fn config() -> LegacyElementReadConfig {
        LegacyElementReadConfig {
            maximum_elements: 16,
            campaign_character_count: 2,
            static_creation_order_boundary: 5,
        }
    }

    fn envelope(records: &[(u16, u32, Option<u32>)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for &(class, creation_order, description) in records {
            bytes.extend_from_slice(&class.to_le_bytes());
            bytes.extend_from_slice(&creation_order.to_le_bytes());
            if class == LegacyElementClass::ActorPc.raw() {
                bytes.extend_from_slice(
                    &description
                        .expect("PC synthetic record requires description index")
                        .to_le_bytes(),
                );
            }
        }
        bytes
    }

    #[test]
    fn decodes_static_and_dynamic_identities_and_stops_at_phase_two() {
        let mut bytes = envelope(&[
            (LegacyElementClass::ActorNpcSoldier.raw(), 3, None),
            (LegacyElementClass::ActorPc.raw(), 10, Some(1)),
        ]);
        bytes.push(0xaa);

        with_reader(&bytes, |reader| {
            let decoded = LegacyElementEnvelope::read_phase1(reader, &config()).unwrap();
            assert_eq!(decoded.start_offset, 0);
            assert_eq!(decoded.phase2_offset, 20);
            assert_eq!(decoded.records.len(), 2);
            assert_eq!(
                decoded.records[0].resolution,
                LegacyElementResolution::ResolveStatic {
                    fallback_factory: None
                }
            );
            assert_eq!(
                decoded.records[1].resolution,
                LegacyElementResolution::ConstructDynamic {
                    factory: LegacyDynamicElementFactory::ActorPc
                }
            );
            assert_eq!(decoded.records[1].pc_description_index, Some(1));
            assert_eq!(decoded.fixups.slot_for_creation_order(3), Some(0));
            assert_eq!(decoded.fixups.slot_for_creation_order(10), Some(1));
            assert_eq!(reader.read_u8("phase2.first_byte").unwrap(), 0xaa);

            let json = serde_json::to_string(&decoded).unwrap();
            let round_trip: LegacyElementEnvelope = serde_json::from_str(&json).unwrap();
            assert_eq!(round_trip, decoded);
        });
    }

    #[test]
    fn rejects_count_above_caller_bound_before_record_allocation() {
        let error = with_reader(&u32::MAX.to_le_bytes(), |reader| {
            LegacyElementEnvelope::read_phase1(reader, &config()).unwrap_err()
        });
        assert_eq!(error.offset, 0);
        assert_eq!(error.field, "element_count");
        assert!(matches!(
            error.kind,
            LegacyIoErrorKind::CountLimit {
                count: u32::MAX,
                maximum: 16
            }
        ));
    }

    #[test]
    fn rejects_duplicate_and_descending_creation_orders() {
        for records in [
            [
                (LegacyElementClass::Arrow.raw(), 5, None),
                (LegacyElementClass::Apple.raw(), 5, None),
            ],
            [
                (LegacyElementClass::Arrow.raw(), 6, None),
                (LegacyElementClass::Apple.raw(), 5, None),
            ],
        ] {
            let bytes = envelope(&records);
            let error = with_reader(&bytes, |reader| {
                LegacyElementEnvelope::read_phase1(reader, &config()).unwrap_err()
            });
            assert_eq!(error.offset, 12);
            assert_eq!(error.field, "elements[1].creation_order");
            assert!(matches!(error.kind, LegacyIoErrorKind::InvalidValue { .. }));
        }
    }

    #[test]
    fn rejects_unknown_class_and_nonconstructible_dynamic_class() {
        let unknown = envelope(&[(0x7777, 5, None)]);
        let error = with_reader(&unknown, |reader| {
            LegacyElementEnvelope::read_phase1(reader, &config()).unwrap_err()
        });
        assert_eq!(error.offset, 4);
        assert_eq!(error.field, "elements[0].class_id");

        let static_only = envelope(&[(LegacyElementClass::ActorNpcSoldier.raw(), 5, None)]);
        let error = with_reader(&static_only, |reader| {
            LegacyElementEnvelope::read_phase1(reader, &config()).unwrap_err()
        });
        assert_eq!(error.offset, 4);
        assert_eq!(error.field, "elements[0].class_id");
        assert!(matches!(error.kind, LegacyIoErrorKind::InvalidValue { .. }));
    }

    #[test]
    fn rejects_reserved_creation_order_and_bad_pc_description() {
        let reserved = envelope(&[(LegacyElementClass::Arrow.raw(), u32::MAX, None)]);
        let error = with_reader(&reserved, |reader| {
            LegacyElementEnvelope::read_phase1(reader, &config()).unwrap_err()
        });
        assert_eq!(error.offset, 6);
        assert_eq!(error.field, "elements[0].creation_order");

        let bad_pc = envelope(&[(LegacyElementClass::ActorPc.raw(), 5, Some(2))]);
        let error = with_reader(&bad_pc, |reader| {
            LegacyElementEnvelope::read_phase1(reader, &config()).unwrap_err()
        });
        assert_eq!(error.offset, 10);
        assert_eq!(error.field, "elements[0].pc_description_index");
    }

    #[test]
    fn reports_truncated_pc_description_identity() {
        let mut bytes = envelope(&[(LegacyElementClass::ActorPc.raw(), 5, Some(1))]);
        bytes.truncate(bytes.len() - 2);
        let error = with_reader(&bytes, |reader| {
            LegacyElementEnvelope::read_phase1(reader, &config()).unwrap_err()
        });
        assert_eq!(error.offset, 10);
        assert_eq!(error.field, "elements[0].pc_description_index");
        assert!(matches!(error.kind, LegacyIoErrorKind::SbFile(_)));
    }

    #[test]
    fn payload_header_revalidates_phase_one_identity_and_stops_at_payload() {
        let phase1 = envelope(&[(LegacyElementClass::ActorPc.raw(), 5, Some(1))]);
        let expected = with_reader(&phase1, |reader| {
            LegacyElementEnvelope::read_phase1(reader, &config())
                .unwrap()
                .records
                .remove(0)
        });
        let mut phase2 = Vec::new();
        phase2.extend_from_slice(&LegacyElementClass::ActorPc.raw().to_le_bytes());
        phase2.extend_from_slice(&5_u32.to_le_bytes());
        phase2.extend_from_slice(&1_u32.to_le_bytes());
        phase2.push(0xaa);

        with_reader(&phase2, |reader| {
            let header = read_payload_header(reader, &expected, 2).unwrap();
            assert_eq!(
                header,
                LegacyElementPayloadHeader {
                    class: LegacyElementClass::ActorPc,
                    creation_order: 5,
                    pc_description_index: Some(1),
                }
            );
            assert_eq!(reader.read_u8("payload.first_byte").unwrap(), 0xaa);
        });
    }

    #[test]
    fn payload_header_rejects_identity_drift_from_phase_one() {
        let phase1 = envelope(&[(LegacyElementClass::Arrow.raw(), 5, None)]);
        let expected = with_reader(&phase1, |reader| {
            LegacyElementEnvelope::read_phase1(reader, &config())
                .unwrap()
                .records
                .remove(0)
        });
        let phase2 = envelope(&[(LegacyElementClass::Arrow.raw(), 6, None)])[4..].to_vec();
        let error = with_reader(&phase2, |reader| {
            read_payload_header(reader, &expected, 2).unwrap_err()
        });
        assert_eq!(error.offset, 2);
        assert_eq!(error.field, "creation_order");
        assert!(matches!(error.kind, LegacyIoErrorKind::InvalidValue { .. }));
    }
}
