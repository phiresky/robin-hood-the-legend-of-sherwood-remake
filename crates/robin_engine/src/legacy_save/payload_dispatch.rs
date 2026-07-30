//! Central phase-two element payload dispatcher for Original v48 saves.
//!
//! [`LegacyElementEnvelope`] establishes every element's stable identity in
//! phase one. This module consumes phase two in that same order, validates the
//! repeated identity header before every payload, and invokes the leaf readers
//! at the exact inheritance callback points used by the Original serializers.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::elements::{
    LegacyElementClass, LegacyElementEnvelope, LegacyElementPayloadHeader, LegacyElementRecord,
    read_payload_header,
};
use super::payload_actors::{
    LegacyActorLeafLimits, LegacyCivilianPayload, LegacyPcPayload, LegacySoldierPayload,
    read_civilian_payload, read_pc_payload, read_soldier_payload,
};
use super::payload_base::{
    LegacyHumanPayload, LegacyNpcPayload, LegacyPayloadDecodeContext, LegacyPayloadLimits,
};
use super::payload_nonactors::{
    LegacyBonusPayload, LegacyNonActorPayloadDecodeContext, LegacyNonActorPayloadLimits,
    LegacyScrollPayload, LegacyStandaloneFxMaskedPayload, LegacyStandaloneFxPayload,
    LegacyTargetPayload, read_bonus_payload, read_fx_masked_payload, read_fx_payload,
    read_scroll_payload, read_target_payload,
};
use super::payload_objects::{
    LegacyObjectItemPayload, LegacyObjectPayloadLimits, read_object_item_payload,
};
use super::payload_sequences::LegacyInlineSequence;
#[cfg(test)]
use super::{
    payload_ai::LegacyLocalAiPayload,
    payload_vm::LegacyVmMemberSection,
};

/// All mission-initialized information needed while decoding element payloads.
///
/// A save does not contain enough schema information to decode VM members,
/// inline sequences, local AI, mobile sprite arrays, Scroll scripts, or Target
/// scripts in isolation. Implementations must resolve those shapes from the
/// mission that has already been initialized for the save.
pub trait LegacyElementPayloadDecodeContext:
    LegacyPayloadDecodeContext + LegacyNonActorPayloadDecodeContext
{
}

impl<T> LegacyElementPayloadDecodeContext for T where
    T: LegacyPayloadDecodeContext + LegacyNonActorPayloadDecodeContext
{
}

/// Caller-owned allocation and reference bounds for the complete phase-two
/// element stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyElementPayloadLimits {
    pub base: LegacyPayloadLimits,
    pub actors: LegacyActorLeafLimits,
    pub nonactors: LegacyNonActorPayloadLimits,
    pub objects: LegacyObjectPayloadLimits,
}

/// Complete phase-two stream, including exact byte boundaries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyElementPayloadStream {
    pub start_offset: u64,
    pub records: Vec<LegacyElementPayloadRecord>,
    pub end_offset: u64,
}

/// One validated repeated identity header and its complete concrete payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyElementPayloadRecord {
    pub slot: usize,
    pub start_offset: u64,
    pub header: LegacyElementPayloadHeader,
    pub payload_start_offset: u64,
    pub payload: LegacyElementPayload,
    pub end_offset: u64,
}

/// Concrete payloads supported by shipped v48 saves.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LegacyElementPayload {
    ActorPc(LegacyPcPayload<LegacyHumanPayload, LegacyInlineSequence>),
    ActorNpcSoldier(LegacySoldierPayload<LegacyNpcPayload>),
    ActorNpcCivilian(LegacyCivilianPayload<LegacyNpcPayload>),
    ObjectItem(LegacyObjectItemPayload),
    Bonus(LegacyBonusPayload),
    Scroll(LegacyScrollPayload),
    Target(LegacyTargetPayload),
    Fx(LegacyStandaloneFxPayload),
    FxMasked(LegacyStandaloneFxMaskedPayload),
}

impl LegacyElementPayloadStream {
    /// Decode phase two at the envelope's exact continuation offset.
    ///
    /// The function deliberately has no catch-all byte skipping: a class
    /// whose grammar is not implemented is a hard structural error.
    pub fn read<C: LegacyElementPayloadDecodeContext>(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        envelope: &LegacyElementEnvelope,
        campaign_character_count: usize,
        limits: &LegacyElementPayloadLimits,
        context: &C,
    ) -> LegacyResult<Self> {
        let start_offset = reader.offset();
        if start_offset != envelope.phase2_offset {
            return Err(reader.invalid_value(
                start_offset,
                "phase2_offset",
                start_offset,
                "reader positioned at LegacyElementEnvelope::phase2_offset",
            ));
        }

        let mut records = Vec::new();
        records
            .try_reserve_exact(envelope.records.len())
            .map_err(|_| {
                reader.allocation_error(start_offset, "element_payloads", envelope.records.len())
            })?;

        for expected in &envelope.records {
            let record =
                reader.scope(format!("element_payloads[{}]", expected.slot), |reader| {
                    Self::read_record(
                        reader,
                        abi_profile,
                        expected,
                        campaign_character_count,
                        limits,
                        context,
                    )
                })?;
            records.push(record);
        }

        Ok(Self {
            start_offset,
            records,
            end_offset: reader.offset(),
        })
    }

    fn read_record<C: LegacyElementPayloadDecodeContext>(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        expected: &LegacyElementRecord,
        campaign_character_count: usize,
        limits: &LegacyElementPayloadLimits,
        context: &C,
    ) -> LegacyResult<LegacyElementPayloadRecord> {
        let start_offset = reader.offset();
        let header = read_payload_header(reader, expected, campaign_character_count)?;
        let payload_start_offset = reader.offset();
        let payload = read_concrete_payload(reader, abi_profile, expected, limits, context)?;
        let end_offset = reader.offset();

        // Keep this equality explicit even though read_payload_header already
        // validates every component. In particular this makes the final
        // record's identity part of the returned, inspectable parse result.
        debug_assert_eq!(header.class, expected.class);
        debug_assert_eq!(header.creation_order, expected.creation_order);
        debug_assert_eq!(header.pc_description_index, expected.pc_description_index);

        Ok(LegacyElementPayloadRecord {
            slot: expected.slot,
            start_offset,
            header,
            payload_start_offset,
            payload,
            end_offset,
        })
    }
}

fn read_concrete_payload<C: LegacyElementPayloadDecodeContext>(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
    expected: &LegacyElementRecord,
    limits: &LegacyElementPayloadLimits,
    context: &C,
) -> LegacyResult<LegacyElementPayload> {
    let creation_order = expected.creation_order;
    let class = expected.class;

    Ok(match class {
        LegacyElementClass::ActorPc => {
            let payload = read_pc_payload(
                reader,
                abi_profile,
                &limits.actors,
                |reader, _abi_profile| {
                    LegacyHumanPayload::read(reader, &limits.base, context, creation_order, class)
                },
                |reader, _abi_profile| context.read_inline_sequence(reader),
            )?;
            LegacyElementPayload::ActorPc(payload)
        }
        LegacyElementClass::ActorNpcSoldier => {
            let payload = read_soldier_payload(reader, abi_profile, |reader, _abi_profile| {
                LegacyNpcPayload::read(reader, &limits.base, context, creation_order, class)
            })?;
            LegacyElementPayload::ActorNpcSoldier(payload)
        }
        LegacyElementClass::ActorNpcCivilian => {
            let payload = read_civilian_payload(reader, abi_profile, |reader, _abi_profile| {
                LegacyNpcPayload::read(reader, &limits.base, context, creation_order, class)
            })?;
            LegacyElementPayload::ActorNpcCivilian(payload)
        }

        LegacyElementClass::Object
        | LegacyElementClass::Arrow
        | LegacyElementClass::Apple
        | LegacyElementClass::Purse
        | LegacyElementClass::Stone
        | LegacyElementClass::WaspNest
        | LegacyElementClass::Wasp
        | LegacyElementClass::Net
        | LegacyElementClass::Coin
        | LegacyElementClass::Ale
        | LegacyElementClass::SpyCape
        | LegacyElementClass::Mobile => LegacyElementPayload::ObjectItem(read_object_item_payload(
            reader,
            abi_profile,
            &limits.objects,
            &limits.base,
            context,
            creation_order,
            class,
        )?),

        LegacyElementClass::BonusAle
        | LegacyElementClass::BonusAmulet
        | LegacyElementClass::BonusArrow
        | LegacyElementClass::BonusApple
        | LegacyElementClass::BonusBlazon
        | LegacyElementClass::BonusLambLeg
        | LegacyElementClass::BonusNet
        | LegacyElementClass::BonusPlants
        | LegacyElementClass::BonusPurse
        | LegacyElementClass::BonusStone
        | LegacyElementClass::BonusWaspNest
        | LegacyElementClass::BonusRansom
        | LegacyElementClass::BonusAmpulla
        | LegacyElementClass::BonusCoronationSpoon
        | LegacyElementClass::BonusRichardsCrown
        | LegacyElementClass::BonusRoyalSeal
        | LegacyElementClass::BonusRoyalSceptre
        | LegacyElementClass::BonusDomesdayBook
        | LegacyElementClass::BonusSwordOfTheState => LegacyElementPayload::Bonus(
            read_bonus_payload(reader, abi_profile, &limits.base, creation_order, class)?,
        ),

        LegacyElementClass::Scroll => LegacyElementPayload::Scroll(read_scroll_payload(
            reader,
            abi_profile,
            &limits.base,
            context,
            creation_order,
        )?),
        LegacyElementClass::Target => LegacyElementPayload::Target(read_target_payload(
            reader,
            &limits.base,
            &limits.nonactors,
            context,
            creation_order,
        )?),
        LegacyElementClass::Fx => {
            LegacyElementPayload::Fx(read_fx_payload(reader, &limits.base, creation_order)?)
        }
        LegacyElementClass::FxMasked => LegacyElementPayload::FxMasked(read_fx_masked_payload(
            reader,
            &limits.base,
            creation_order,
        )?),

        // TODO(original-save): Port RHElementActorAnimal and its concrete
        // species serializers if a non-shipped/custom mission demonstrates a
        // real save containing one. The Original marks this subsystem dead and
        // none of the audited shipped-save corpus exercises these classes.
        LegacyElementClass::ActorAnimal
        | LegacyElementClass::ActorDog
        | LegacyElementClass::ActorCow
        | LegacyElementClass::ActorHen
        | LegacyElementClass::ActorPig
        | LegacyElementClass::ActorBird
        | LegacyElementClass::ActorCrocodile => {
            return unsupported_class(reader, class, "implemented non-animal concrete payload");
        }

        // This class is an inheritance base. Shipped records use one of the
        // concrete Soldier/Civilian IDs, so accepting it would lose leaf state.
        LegacyElementClass::ActorNpc => {
            return unsupported_class(reader, class, "concrete Soldier or Civilian actor class");
        }
    })
}

fn unsupported_class<T>(
    reader: &mut LegacyReader<'_>,
    class: LegacyElementClass,
    expected: &'static str,
) -> LegacyResult<T> {
    let offset = reader.offset();
    Err(reader.invalid_value(offset, "class", format_args!("{class:?}"), expected))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::legacy_io::LegacyIoErrorKind;
    use crate::sbfile::{SB_FILE_READ, SbFile};

    use super::super::elements::{
        LegacyDynamicElementFactory, LegacyElementFixupTable, LegacyElementResolution,
    };

    struct EmptyContext;

    impl LegacyPayloadDecodeContext for EmptyContext {
        fn mobile_sprite_count(
            &self,
            _creation_order: u32,
            _maximum: usize,
        ) -> LegacyResult<usize> {
            Ok(0)
        }

        fn read_actor_script_members(
            &self,
            _reader: &mut LegacyReader<'_>,
            _script_class: &str,
        ) -> LegacyResult<LegacyVmMemberSection> {
            unreachable!("synthetic dispatcher tests contain no actor payload")
        }

        fn read_inline_sequence(
            &self,
            _reader: &mut LegacyReader<'_>,
        ) -> LegacyResult<LegacyInlineSequence> {
            unreachable!("synthetic dispatcher tests contain no inline sequence")
        }

        fn read_local_ai(
            &self,
            _reader: &mut LegacyReader<'_>,
            _creation_order: u32,
            _class: LegacyElementClass,
        ) -> LegacyResult<Box<LegacyLocalAiPayload>> {
            unreachable!("synthetic dispatcher tests contain no local AI")
        }
    }

    impl LegacyNonActorPayloadDecodeContext for EmptyContext {
        fn read_script_members(
            &self,
            _reader: &mut LegacyReader<'_>,
            _creation_order: u32,
            _class: LegacyElementClass,
        ) -> LegacyResult<Option<LegacyVmMemberSection>> {
            Ok(None)
        }
    }

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut temporary = NamedTempFile::new().unwrap();
        temporary.write_all(bytes).unwrap();
        temporary.flush().unwrap();
        let path = temporary.path().to_str().unwrap();
        let mut file = SbFile::open(path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    fn record(class: LegacyElementClass, creation_order: u32) -> LegacyElementRecord {
        LegacyElementRecord {
            slot: 0,
            class,
            creation_order,
            pc_description_index: None,
            resolution: LegacyElementResolution::ConstructDynamic {
                factory: LegacyDynamicElementFactory::Ale,
            },
            creation_order_offset: 2,
        }
    }

    fn envelope(record: Option<LegacyElementRecord>) -> LegacyElementEnvelope {
        let records: Vec<_> = record.into_iter().collect();
        let by_creation_order = records
            .iter()
            .map(|record| (record.creation_order, record.slot))
            .collect::<BTreeMap<_, _>>();
        LegacyElementEnvelope {
            start_offset: 0,
            phase2_offset: 0,
            records,
            fixups: LegacyElementFixupTable { by_creation_order },
        }
    }

    #[test]
    fn empty_phase_two_stream_preserves_exact_offsets() {
        with_reader(&[], |reader| {
            let decoded = LegacyElementPayloadStream::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &envelope(None),
                0,
                &LegacyElementPayloadLimits::default(),
                &EmptyContext,
            )
            .unwrap();
            assert_eq!(decoded.start_offset, 0);
            assert_eq!(decoded.end_offset, 0);
            assert!(decoded.records.is_empty());
        });
    }

    #[test]
    fn repeated_header_mismatch_fails_before_payload_dispatch() {
        let expected = record(LegacyElementClass::Ale, 17);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&LegacyElementClass::Coin.raw().to_le_bytes());
        bytes.extend_from_slice(&17_u32.to_le_bytes());

        with_reader(&bytes, |reader| {
            let error = LegacyElementPayloadStream::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &envelope(Some(expected)),
                0,
                &LegacyElementPayloadLimits::default(),
                &EmptyContext,
            )
            .unwrap_err();
            assert!(matches!(
                &error.kind,
                LegacyIoErrorKind::InvalidValue { .. }
            ));
            assert!(error.to_string().contains("same class id"));
        });
    }

    #[test]
    fn unsupported_abstract_class_is_a_hard_error_after_identity() {
        let expected = record(LegacyElementClass::ActorNpc, 42);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&LegacyElementClass::ActorNpc.raw().to_le_bytes());
        bytes.extend_from_slice(&42_u32.to_le_bytes());

        with_reader(&bytes, |reader| {
            let error = LegacyElementPayloadStream::read(
                reader,
                LegacySaveAbiProfile::RetailWindowsX86V48,
                &envelope(Some(expected)),
                0,
                &LegacyElementPayloadLimits::default(),
                &EmptyContext,
            )
            .unwrap_err();
            assert!(matches!(
                &error.kind,
                LegacyIoErrorKind::InvalidValue { .. }
            ));
            assert!(error.to_string().contains("concrete Soldier or Civilian"));
        });
    }
}
