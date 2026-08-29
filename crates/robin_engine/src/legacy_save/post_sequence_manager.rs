//! Original v48 `RHSequenceManager::Serialize` decoding.
//!
//! Manager-owned sequences use `RHSequence::Serialize(file, true)`: each
//! sequence element has three deferred-ID fixups after its orders, and
//! movement elements have one additional linked-seek fixup after their
//! optional inline post-seek sequence. The manager then writes a queue of
//! sequence-element IDs. These are IDs, not phase-one element references.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::payload_base::{LegacySequenceElementRef, read_sequence_element_ref};
use super::payload_sequences::{
    LegacyInlineSequence, LegacyInlineSequenceElement, LegacySequencePayloadLimits,
    read_sequence_with_pre_serialization,
};

const FINGERPRINT_SEQUENCE_MANAGER: [u8; 16] = hex16("2655e6483ffc10f8c935273d80569280");
const FINGERPRINT_ORDER_STATIC: [u8; 16] = hex16("c0a03b6622b85950bea7cb175f3de0f0");
const FINGERPRINT_SEQUENCE_STATIC: [u8; 16] = hex16("56f687760fb3324570654ecc1ff59f92");
const FINGERPRINT_SEQUENCE_ELEMENT_STATIC: [u8; 16] = hex16("5051f78a07d6f907eb5648339133c723");

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
pub struct LegacySequenceManagerLimits {
    pub sequences: usize,
    pub deferred_elements: usize,
    pub payload: LegacySequencePayloadLimits,
}

impl Default for LegacySequenceManagerLimits {
    fn default() -> Self {
        Self {
            sequences: 65_535,
            deferred_elements: 65_535,
            payload: LegacySequencePayloadLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySequenceStaticIds {
    pub order_next_id_offset: u64,
    pub order_next_id: u32,
    pub sequence_next_id_offset: u64,
    pub sequence_next_id: u32,
    pub sequence_element_next_id_offset: u64,
    pub sequence_element_next_id: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyManagedSequence {
    pub start_offset: u64,
    pub body: LegacyInlineSequence,
    pub end_offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyDeferredSequenceElement {
    pub offset: u64,
    pub element: LegacySequenceElementRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySequenceManagerState {
    pub start_offset: u64,
    pub static_ids: LegacySequenceStaticIds,
    pub sequences: Vec<LegacyManagedSequence>,
    pub deferred_elements: Vec<LegacyDeferredSequenceElement>,
    pub end_offset: u64,
}

impl LegacySequenceManagerState {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi: LegacySaveAbiProfile,
        limits: &LegacySequenceManagerLimits,
    ) -> LegacyResult<Self> {
        // Both audited v48 ABIs use the same scalar widths and byte order.
        match abi {
            LegacySaveAbiProfile::RetailWindowsX86V48 | LegacySaveAbiProfile::PortLinuxI386V48 => {}
        }
        reader.scope("RHSequenceManager", |reader| {
            let start_offset = reader.offset();
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_SEQUENCE_MANAGER,
                "RHSequenceManager fingerprint",
            )?;

            reader.read_signature(
                "order_static.fingerprint",
                FINGERPRINT_ORDER_STATIC,
                "RHOrder::SerializeStaticStuff fingerprint",
            )?;
            let order_next_id_offset = reader.offset();
            let order_next_id = reader.read_u32("order_static.next_id")?;

            reader.read_signature(
                "sequence_static.fingerprint",
                FINGERPRINT_SEQUENCE_STATIC,
                "RHSequence::SerializeStaticStuff fingerprint",
            )?;
            let sequence_next_id_offset = reader.offset();
            let sequence_next_id = read_nonzero_next_id(reader, "sequence_static.next_id")?;

            reader.read_signature(
                "sequence_element_static.fingerprint",
                FINGERPRINT_SEQUENCE_ELEMENT_STATIC,
                "RHSequenceElement::SerializeStaticStuff fingerprint",
            )?;
            let sequence_element_next_id_offset = reader.offset();
            let sequence_element_next_id =
                read_nonzero_next_id(reader, "sequence_element_static.next_id")?;

            let static_ids = LegacySequenceStaticIds {
                order_next_id_offset,
                order_next_id,
                sequence_next_id_offset,
                sequence_next_id,
                sequence_element_next_id_offset,
                sequence_element_next_id,
            };

            let sequence_count = reader.read_count_u32("sequences.count", limits.sequences)?;
            let mut sequences = Vec::new();
            reserve(reader, &mut sequences, sequence_count, "sequences")?;
            for index in 0..sequence_count {
                sequences.push(reader.scope(format!("sequences[{index}]"), |reader| {
                    let start_offset = reader.offset();
                    let body = read_sequence_with_pre_serialization(reader, &limits.payload)?;
                    let end_offset = reader.offset();
                    Ok(LegacyManagedSequence {
                        start_offset,
                        body,
                        end_offset,
                    })
                })?);
            }

            let deferred_count =
                reader.read_count_u32("deferred_elements.count", limits.deferred_elements)?;
            let mut deferred_elements = Vec::new();
            reserve(
                reader,
                &mut deferred_elements,
                deferred_count,
                "deferred_elements",
            )?;
            for index in 0..deferred_count {
                deferred_elements.push(reader.scope(
                    format!("deferred_elements[{index}]"),
                    |reader| {
                        let offset = reader.offset();
                        let element = read_sequence_element_ref(reader, "element")?;
                        if element.0.is_none() {
                            return Err(reader.invalid_value(
                                offset,
                                "element",
                                "0xffffffff",
                                "a non-null manager-owned sequence-element ID",
                            ));
                        }
                        Ok(LegacyDeferredSequenceElement { offset, element })
                    },
                )?);
            }

            validate_ids(reader, &static_ids, &sequences, &deferred_elements)?;
            let end_offset = reader.offset();
            Ok(Self {
                start_offset,
                static_ids,
                sequences,
                deferred_elements,
                end_offset,
            })
        })
    }
}

fn read_nonzero_next_id(reader: &mut LegacyReader<'_>, field: &'static str) -> LegacyResult<u32> {
    let offset = reader.offset();
    let value = reader.read_u32(field)?;
    if value == 0 || value == u32::MAX {
        Err(reader.invalid_value(offset, field, value, "a non-zero, non-null next ID"))
    } else {
        Ok(value)
    }
}

fn validate_ids(
    reader: &mut LegacyReader<'_>,
    static_ids: &LegacySequenceStaticIds,
    sequences: &[LegacyManagedSequence],
    deferred_elements: &[LegacyDeferredSequenceElement],
) -> LegacyResult<()> {
    let mut sequence_ids = HashSet::new();
    let manager_sequence_ids = sequences
        .iter()
        .map(|sequence| sequence.body.unique_id.0)
        .collect::<HashSet<_>>();
    let mut element_ids = HashSet::new();
    let mut order_ids = HashSet::new();
    let mut manager_element_ids = HashSet::new();

    for sequence in sequences {
        collect_sequence_ids(
            reader,
            &sequence.body,
            true,
            &mut sequence_ids,
            &mut element_ids,
            &mut order_ids,
            &mut manager_element_ids,
        )?;
    }

    validate_next_id(
        reader,
        static_ids.order_next_id_offset,
        "order_static.next_id",
        static_ids.order_next_id,
        order_ids.iter().copied().max(),
    )?;
    validate_next_id(
        reader,
        static_ids.sequence_next_id_offset,
        "sequence_static.next_id",
        static_ids.sequence_next_id,
        sequence_ids.iter().copied().max(),
    )?;
    validate_next_id(
        reader,
        static_ids.sequence_element_next_id_offset,
        "sequence_element_static.next_id",
        static_ids.sequence_element_next_id,
        element_ids.iter().copied().max(),
    )?;

    for sequence in sequences {
        validate_fixups(
            reader,
            &sequence.body,
            &manager_element_ids,
            &manager_sequence_ids,
        )?;
    }
    for deferred in deferred_elements {
        let id = deferred.element.0.expect("non-null checked while reading");
        if !manager_element_ids.contains(&id) {
            return Err(reader.invalid_value(
                deferred.offset,
                "deferred_elements.element",
                id,
                "an ID of a sequence element owned by this RHSequenceManager",
            ));
        }
        let state = manager_element_state(sequences, id)
            .expect("membership and lookup traverse the same manager sequences");
        if matches!(state, 0 | 5 | 6) {
            return Err(reader.invalid_value(
                deferred.offset,
                "deferred_elements.element",
                format_args!("ID {id} in terminal state {state}"),
                "a non-terminal sequence element (not terminated, impossible, or interrupted)",
            ));
        }
    }
    Ok(())
}

fn collect_sequence_ids(
    reader: &mut LegacyReader<'_>,
    sequence: &LegacyInlineSequence,
    manager_owned: bool,
    sequence_ids: &mut HashSet<u32>,
    element_ids: &mut HashSet<u32>,
    order_ids: &mut HashSet<u32>,
    manager_element_ids: &mut HashSet<u32>,
) -> LegacyResult<()> {
    insert_unique(
        reader,
        sequence_ids,
        sequence.unique_id.0,
        "sequence.unique_id",
        "a unique RHSequence ID",
    )?;
    for element in &sequence.elements {
        let base = element.base();
        insert_unique(
            reader,
            element_ids,
            base.unique_id.0,
            "sequence_element.unique_id",
            "a unique RHSequenceElement ID",
        )?;
        if manager_owned {
            manager_element_ids.insert(base.unique_id.0);
        }
        for order in &base.orders {
            insert_unique(
                reader,
                order_ids,
                order.unique_id.0,
                "order.unique_id",
                "a unique RHOrder ID",
            )?;
        }
        if let LegacyInlineSequenceElement::Movement(movement) = element
            && let Some(post_seek) = movement.post_seek_sequence.as_deref()
        {
            collect_sequence_ids(
                reader,
                post_seek,
                false,
                sequence_ids,
                element_ids,
                order_ids,
                manager_element_ids,
            )?;
        }
    }
    Ok(())
}

fn validate_fixups(
    reader: &mut LegacyReader<'_>,
    sequence: &LegacyInlineSequence,
    manager_element_ids: &HashSet<u32>,
    sequence_ids: &HashSet<u32>,
) -> LegacyResult<()> {
    for element in &sequence.elements {
        let base = element.base();
        let missing_fixups_offset = reader.offset();
        let fixups = base.manager_fixups.as_ref().ok_or_else(|| {
            reader.invalid_value(
                missing_fixups_offset,
                "sequence_element.manager_fixups",
                "missing",
                "three pre-serialized IDs on every manager-owned sequence element",
            )
        })?;
        validate_optional_ref(
            reader,
            fixups.next_offset,
            "next_sequence_element",
            fixups.next.0,
            manager_element_ids,
            "an ID of a sequence element owned by this RHSequenceManager",
        )?;
        validate_optional_ref(
            reader,
            fixups.postponed_offset,
            "postponed_sequence_element",
            fixups.postponed.0,
            manager_element_ids,
            "an ID of a sequence element owned by this RHSequenceManager",
        )?;
        validate_optional_ref(
            reader,
            fixups.mummy_offset,
            "mummy_sequence",
            fixups.mummy.0,
            sequence_ids,
            "an ID of a sequence registered while reading this RHSequenceManager",
        )?;
        if let LegacyInlineSequenceElement::Movement(movement) = element {
            let missing_linked_offset = reader.offset();
            let offset = movement.manager_linked_seek_fixup_offset.ok_or_else(|| {
                reader.invalid_value(
                    missing_linked_offset,
                    "linked_seek_sequence_element",
                    "missing offset",
                    "a pre-serialized linked-seek ID on a manager-owned movement element",
                )
            })?;
            let reference = movement.manager_linked_seek_fixup.as_ref().ok_or_else(|| {
                reader.invalid_value(
                    offset,
                    "linked_seek_sequence_element",
                    "missing",
                    "a pre-serialized linked-seek ID on a manager-owned movement element",
                )
            })?;
            validate_optional_ref(
                reader,
                offset,
                "linked_seek_sequence_element",
                reference.0,
                manager_element_ids,
                "an ID of a sequence element owned by this RHSequenceManager",
            )?;
        }
    }
    Ok(())
}

fn validate_optional_ref(
    reader: &mut LegacyReader<'_>,
    offset: u64,
    field: &'static str,
    id: Option<u32>,
    valid_ids: &HashSet<u32>,
    expected: &'static str,
) -> LegacyResult<()> {
    if let Some(id) = id
        && !valid_ids.contains(&id)
    {
        return Err(reader.invalid_value(offset, field, id, expected));
    }
    Ok(())
}

fn insert_unique(
    reader: &mut LegacyReader<'_>,
    ids: &mut HashSet<u32>,
    id: u32,
    field: &'static str,
    expected: &'static str,
) -> LegacyResult<()> {
    if !ids.insert(id) {
        let offset = reader.offset();
        return Err(reader.invalid_value(offset, field, id, expected));
    }
    Ok(())
}

fn validate_next_id(
    reader: &mut LegacyReader<'_>,
    offset: u64,
    field: &'static str,
    next_id: u32,
    maximum_used: Option<u32>,
) -> LegacyResult<()> {
    if maximum_used.is_some_and(|maximum| next_id <= maximum) {
        return Err(reader.invalid_value(
            offset,
            field,
            next_id,
            "a next ID greater than every serialized ID in its domain",
        ));
    }
    Ok(())
}

fn manager_element_state(sequences: &[LegacyManagedSequence], id: u32) -> Option<i32> {
    sequences.iter().find_map(|sequence| {
        sequence
            .body
            .elements
            .iter()
            .find(|element| element.base().unique_id.0 == id)
            .map(|element| element.base().state)
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
    use crate::legacy_io::LegacyIoErrorKind;
    use crate::sbfile::{SB_FILE_READ, SbFile};

    const FINGERPRINT_SEQUENCE: [u8; 16] = hex16("462542ef9f0ef300dff9647c2091d151");
    const FINGERPRINT_SEQUENCE_ELEMENT: [u8; 16] = hex16("8358d2ae0236d0e6a448a02189c93b67");

    fn empty_manager_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE_MANAGER);
        bytes.extend_from_slice(&FINGERPRINT_ORDER_STATIC);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE_STATIC);
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE_ELEMENT_STATIC);
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes
    }

    fn manager_with_simple_deferred_element() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE_MANAGER);
        bytes.extend_from_slice(&FINGERPRINT_ORDER_STATIC);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE_STATIC);
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE_ELEMENT_STATIC);
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());

        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE);
        bytes.push(1);
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&FINGERPRINT_SEQUENCE_ELEMENT);
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        bytes.extend_from_slice(&3_i32.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&8_i32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        bytes.push(0);
        bytes.push(0);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());

        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes
    }

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut temp = NamedTempFile::new().expect("temporary file");
        temp.write_all(bytes).expect("write fixture");
        let mut file = SbFile::open(temp.path().to_str().expect("utf-8 path"), SB_FILE_READ)
            .expect("open fixture");
        read(&mut LegacyReader::new(&mut file))
    }

    #[test]
    fn reads_empty_manager_for_both_v48_abis() {
        for abi in [
            LegacySaveAbiProfile::RetailWindowsX86V48,
            LegacySaveAbiProfile::PortLinuxI386V48,
        ] {
            let bytes = empty_manager_bytes();
            let state = with_reader(&bytes, |reader| {
                LegacySequenceManagerState::read(
                    reader,
                    abi,
                    &LegacySequenceManagerLimits::default(),
                )
            })
            .expect("empty manager");
            assert_eq!(state.start_offset, 0);
            assert_eq!(state.end_offset, bytes.len() as u64);
            assert!(state.sequences.is_empty());
            assert!(state.deferred_elements.is_empty());
        }
    }

    #[test]
    fn rejects_manager_fingerprint_mismatch_at_exact_offset() {
        let mut bytes = empty_manager_bytes();
        bytes[0] ^= 0xff;
        let error = with_reader(&bytes, |reader| {
            LegacySequenceManagerState::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacySequenceManagerLimits::default(),
            )
        })
        .expect_err("bad fingerprint");
        assert_eq!(error.offset, 0);
        assert!(matches!(error.kind, LegacyIoErrorKind::InvalidValue { .. }));
    }

    #[test]
    fn reads_manager_owned_fixups_and_deferred_queue() {
        let bytes = manager_with_simple_deferred_element();
        let state = with_reader(&bytes, |reader| {
            LegacySequenceManagerState::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacySequenceManagerLimits::default(),
            )
        })
        .expect("manager with one queued element");
        let element = &state.sequences[0].body.elements[0];
        let fixups = element
            .base()
            .manager_fixups
            .as_ref()
            .expect("manager fixups");
        assert_eq!(fixups.next.0, None);
        assert_eq!(fixups.postponed.0, None);
        assert_eq!(fixups.mummy.0, Some(1));
        assert_eq!(state.deferred_elements[0].element.0, Some(1));
        assert_eq!(state.end_offset, bytes.len() as u64);
    }

    #[test]
    fn rejects_sequence_count_above_caller_limit_before_allocating() {
        let mut bytes = empty_manager_bytes();
        let count_offset = bytes.len() - 8;
        bytes[count_offset..count_offset + 4].copy_from_slice(&2_u32.to_le_bytes());
        let limits = LegacySequenceManagerLimits {
            sequences: 1,
            ..LegacySequenceManagerLimits::default()
        };
        let error = with_reader(&bytes, |reader| {
            LegacySequenceManagerState::read(
                reader,
                LegacySaveAbiProfile::RetailWindowsX86V48,
                &limits,
            )
        })
        .expect_err("oversized sequence count");
        assert_eq!(error.offset, count_offset as u64);
        assert!(matches!(
            error.kind,
            LegacyIoErrorKind::CountLimit {
                count: 2,
                maximum: 1
            }
        ));
    }

    #[test]
    fn rejects_null_deferred_sequence_element() {
        let mut bytes = empty_manager_bytes();
        let deferred_count_offset = bytes.len() - 4;
        bytes[deferred_count_offset..].copy_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        let error = with_reader(&bytes, |reader| {
            LegacySequenceManagerState::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacySequenceManagerLimits::default(),
            )
        })
        .expect_err("null deferred element");
        assert_eq!(error.offset, deferred_count_offset as u64 + 4);
        assert!(matches!(error.kind, LegacyIoErrorKind::InvalidValue { .. }));
    }
}
