//! Mission-aware context for Original v48 phase-two element payloads.
//!
//! Several payload shapes are deliberately absent from the save stream. The
//! Original recovers them from the already initialized mission: SCB member
//! layouts, an element's concrete local-AI subtype, and the number of masked
//! sprites owned by a mobile. This adapter keeps those facts explicit and
//! rejects save/mission mismatches instead of guessing a byte layout.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};
use crate::scb::ScbFile;

use super::elements::LegacyElementClass;
use super::payload_ai::{
    LegacyLocalAiDecodeConfig, LegacyLocalAiKind, LegacyLocalAiLimits, LegacyLocalAiPayload,
};
use super::payload_base::LegacyPayloadDecodeContext;
use super::payload_nonactors::LegacyNonActorPayloadDecodeContext;
use super::payload_sequences::{LegacyInlineSequence, LegacySequencePayloadLimits};
use super::payload_vm::{LegacyVmDecodeLimits, LegacyVmMemberDecoder, LegacyVmMemberSection};

/// Save-shape facts recovered from one initialized mission element.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyElementPayloadMetadata {
    pub class: LegacyElementClass,
    pub script_class: Option<String>,
    pub local_ai_kind: Option<LegacyLocalAiKind>,
    pub mobile_sprite_count: Option<usize>,
}

/// Mission element metadata indexed by the Original creation-order identity.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyMissionPayloadMetadata {
    pub by_creation_order: BTreeMap<u32, LegacyElementPayloadMetadata>,
}

/// Independent hard limits applied by the typed payload readers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyMissionPayloadDecodeLimits {
    pub sequences: LegacySequencePayloadLimits,
    pub local_ai: LegacyLocalAiLimits,
    pub vm: LegacyVmDecodeLimits,
}

/// Borrowed decoder context for one phase-two owner.
///
/// Actor script serialization does not repeat the owner's creation order at
/// the VM callback boundary. The dispatcher must therefore select an owner
/// before invoking a leaf reader. Other callbacks repeat the identity and
/// validate it against `owner_creation_order`.
pub struct LegacyMissionPayloadDecodeContext<'a> {
    scb: &'a ScbFile,
    metadata: &'a LegacyMissionPayloadMetadata,
    owner_creation_order: u32,
    limits: LegacyMissionPayloadDecodeLimits,
}

impl<'a> LegacyMissionPayloadDecodeContext<'a> {
    pub fn new(
        scb: &'a ScbFile,
        metadata: &'a LegacyMissionPayloadMetadata,
        owner_creation_order: u32,
        limits: LegacyMissionPayloadDecodeLimits,
    ) -> Self {
        Self {
            scb,
            metadata,
            owner_creation_order,
            limits,
        }
    }

    pub fn with_default_limits(
        scb: &'a ScbFile,
        metadata: &'a LegacyMissionPayloadMetadata,
        owner_creation_order: u32,
    ) -> Self {
        Self::new(
            scb,
            metadata,
            owner_creation_order,
            LegacyMissionPayloadDecodeLimits::default(),
        )
    }

    fn owner_metadata<'r>(
        &self,
        reader: &mut LegacyReader<'r>,
    ) -> LegacyResult<&LegacyElementPayloadMetadata> {
        self.metadata
            .by_creation_order
            .get(&self.owner_creation_order)
            .ok_or_else(|| {
                let offset = reader.offset();
                reader.invalid_value(
                    offset,
                    "owner_creation_order",
                    self.owner_creation_order,
                    "an element present in initialized mission payload metadata",
                )
            })
    }

    fn validate_identity(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<&LegacyElementPayloadMetadata> {
        if creation_order != self.owner_creation_order {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "creation_order",
                format_args!(
                    "{creation_order} (selected owner is {})",
                    self.owner_creation_order
                ),
                "the selected phase-two owner creation order",
            ));
        }
        let metadata = self.owner_metadata(reader)?;
        if metadata.class != class {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "element_class",
                format_args!(
                    "{class:?} (initialized mission class is {:?})",
                    metadata.class
                ),
                "the initialized mission element class",
            ));
        }
        Ok(metadata)
    }

    fn validate_actor_owner(
        &self,
        reader: &mut LegacyReader<'_>,
    ) -> LegacyResult<&LegacyElementPayloadMetadata> {
        let metadata = self.owner_metadata(reader)?;
        if !is_actor_class(metadata.class) {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "element_class",
                format_args!("{:?}", metadata.class),
                "an actor class at the actor VM-member callback",
            ));
        }
        Ok(metadata)
    }
}

impl LegacyPayloadDecodeContext for LegacyMissionPayloadDecodeContext<'_> {
    fn mobile_sprite_count(&self, creation_order: u32, maximum: usize) -> LegacyResult<usize> {
        // No bytes precede this callback, but the reader-less trait method
        // cannot manufacture a structured LegacyIoError. Validate the
        // metadata at construction/use sites through the same strict helper
        // exposed below.
        let metadata = self
            .metadata
            .by_creation_order
            .get(&creation_order)
            .filter(|_| creation_order == self.owner_creation_order)
            .ok_or_else(|| missing_context_error("mobile owner metadata", creation_order))?;
        if metadata.class != LegacyElementClass::Mobile {
            return Err(missing_context_error(
                "mobile owner class compatibility",
                creation_order,
            ));
        }
        let count = metadata
            .mobile_sprite_count
            .ok_or_else(|| missing_context_error("required mobile sprite count", creation_order))?;
        if count > maximum {
            return Err(missing_context_error(
                "mobile sprite count within decode limit",
                creation_order,
            ));
        }
        Ok(count)
    }

    fn read_actor_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        script_class: &str,
    ) -> LegacyResult<LegacyVmMemberSection> {
        let metadata = self.validate_actor_owner(reader)?;
        let expected = metadata.script_class.as_deref().ok_or_else(|| {
            let offset = reader.offset();
            reader.invalid_value(
                offset,
                "script_class",
                script_class,
                "a script class attached to the initialized mission actor",
            )
        })?;
        if expected != script_class {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "script_class",
                format_args!("{script_class:?} (initialized mission class is {expected:?})"),
                "the initialized mission script class",
            ));
        }
        LegacyVmMemberDecoder::new(self.scb, self.limits.vm)
            .read_class_members(reader, script_class)
    }

    fn read_inline_sequence(
        &self,
        reader: &mut LegacyReader<'_>,
    ) -> LegacyResult<LegacyInlineSequence> {
        LegacyInlineSequence::read(reader, &self.limits.sequences)
    }

    fn read_local_ai(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<Box<LegacyLocalAiPayload>> {
        let metadata = self.validate_identity(reader, creation_order, class)?;
        let required_kind = ai_kind_for_class(class).ok_or_else(|| {
            let offset = reader.offset();
            reader.invalid_value(
                offset,
                "element_class",
                format_args!("{class:?}"),
                "ActorNpcCivilian or ActorNpcSoldier for a local-AI payload",
            )
        })?;
        let kind = metadata.local_ai_kind.ok_or_else(|| {
            let offset = reader.offset();
            reader.invalid_value(
                offset,
                "local_ai_kind",
                "missing",
                "the AI kind required by the initialized mission element class",
            )
        })?;
        if kind != required_kind {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "local_ai_kind",
                format_args!("{kind:?} ({class:?} requires {required_kind:?})"),
                "the AI kind required by the initialized mission element class",
            ));
        }
        Ok(Box::new(LegacyLocalAiPayload::read(
            reader,
            &LegacyLocalAiDecodeConfig {
                kind: Some(kind),
                limits: self.limits.local_ai,
            },
        )?))
    }
}

impl LegacyNonActorPayloadDecodeContext for LegacyMissionPayloadDecodeContext<'_> {
    fn read_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<Option<LegacyVmMemberSection>> {
        let metadata = self.validate_identity(reader, creation_order, class)?;
        if !matches!(
            class,
            LegacyElementClass::Scroll | LegacyElementClass::Target
        ) {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "element_class",
                format_args!("{class:?}"),
                "Scroll or Target at the non-actor VM-member callback",
            ));
        }
        metadata
            .script_class
            .as_deref()
            .map(|class_name| {
                LegacyVmMemberDecoder::new(self.scb, self.limits.vm)
                    .read_class_members(reader, class_name)
            })
            .transpose()
    }
}

fn is_actor_class(class: LegacyElementClass) -> bool {
    matches!(
        class,
        LegacyElementClass::ActorPc
            | LegacyElementClass::ActorNpc
            | LegacyElementClass::ActorNpcCivilian
            | LegacyElementClass::ActorNpcSoldier
            | LegacyElementClass::ActorAnimal
            | LegacyElementClass::ActorDog
            | LegacyElementClass::ActorCow
            | LegacyElementClass::ActorHen
            | LegacyElementClass::ActorPig
            | LegacyElementClass::ActorBird
            | LegacyElementClass::ActorCrocodile
    )
}

fn ai_kind_for_class(class: LegacyElementClass) -> Option<LegacyLocalAiKind> {
    match class {
        LegacyElementClass::ActorNpcCivilian => Some(LegacyLocalAiKind::Friendly),
        LegacyElementClass::ActorNpcSoldier => Some(LegacyLocalAiKind::Enemy),
        _ => None,
    }
}

/// `mobile_sprite_count` predates reader-aware context errors. Keep failures
/// explicit until that trait is extended to pass the current reader.
fn missing_context_error(
    expectation: &str,
    creation_order: u32,
) -> crate::legacy_io::LegacyIoError {
    crate::legacy_io::LegacyIoError {
        path: "<initialized mission metadata>".to_owned(),
        offset: 0,
        field: "creation_order".to_owned(),
        kind: crate::legacy_io::LegacyIoErrorKind::InvalidValue {
            value: format!("{creation_order} ({expectation} unavailable)"),
            expected: "complete initialized mission payload metadata",
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::legacy_save::payload_vm::LegacyVmMemberValue;
    use crate::sbfile::{SB_FILE_READ, SbFile};
    use crate::scb::{ClassEntry, MemberVariable, ScType, TypeTag};

    fn scb() -> ScbFile {
        ScbFile {
            version: 1.5,
            classes: vec![ClassEntry {
                source_file: "fixture.scs".into(),
                class_name: "Fixture".into(),
                size_of_member_variables: 4,
                member_variables: vec![MemberVariable {
                    ty: ScType {
                        tag: TypeTag::Int,
                        native_type_name: String::new(),
                    },
                    name: "value".into(),
                    address: 0,
                }],
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        }
    }

    fn metadata(
        creation_order: u32,
        class: LegacyElementClass,
        script_class: Option<&str>,
        ai: Option<LegacyLocalAiKind>,
        mobile_sprite_count: Option<usize>,
    ) -> LegacyMissionPayloadMetadata {
        LegacyMissionPayloadMetadata {
            by_creation_order: [(
                creation_order,
                LegacyElementPayloadMetadata {
                    class,
                    script_class: script_class.map(str::to_owned),
                    local_ai_kind: ai,
                    mobile_sprite_count,
                },
            )]
            .into_iter()
            .collect(),
        }
    }

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut fixture = NamedTempFile::new().unwrap();
        fixture.write_all(bytes).unwrap();
        fixture.flush().unwrap();
        let path = fixture.path().to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    #[test]
    fn decodes_typed_actor_and_nonactor_vm_results() {
        let schema = scb();
        let actor_metadata = metadata(
            7,
            LegacyElementClass::ActorNpcSoldier,
            Some("Fixture"),
            Some(LegacyLocalAiKind::Enemy),
            None,
        );
        let actor =
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &actor_metadata, 7);
        with_reader(&0x1234_5678_u32.to_le_bytes(), |reader| {
            let section = actor.read_actor_script_members(reader, "Fixture").unwrap();
            assert_eq!(
                section.members[0].value,
                LegacyVmMemberValue::Raw32 { bits: 0x1234_5678 }
            );
        });

        let scroll_metadata = metadata(9, LegacyElementClass::Scroll, Some("Fixture"), None, None);
        let scroll =
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &scroll_metadata, 9);
        with_reader(&42_u32.to_le_bytes(), |reader| {
            let section = scroll
                .read_script_members(reader, 9, LegacyElementClass::Scroll)
                .unwrap()
                .unwrap();
            assert_eq!(
                section.members[0].value,
                LegacyVmMemberValue::Raw32 { bits: 42 }
            );
        });
    }

    #[test]
    fn returns_none_only_for_metadata_declared_unscripted_nonactors() {
        let schema = scb();
        let metadata = metadata(11, LegacyElementClass::Target, None, None, None);
        let context =
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &metadata, 11);
        with_reader(&[], |reader| {
            assert_eq!(
                context
                    .read_script_members(reader, 11, LegacyElementClass::Target)
                    .unwrap(),
                None
            );
        });
    }

    #[test]
    fn reads_mobile_count_only_from_matching_owner_metadata() {
        let schema = scb();
        let metadata = metadata(13, LegacyElementClass::Mobile, None, None, Some(3));
        let context =
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &metadata, 13);
        assert_eq!(context.mobile_sprite_count(13, 4).unwrap(), 3);
        assert!(context.mobile_sprite_count(13, 2).is_err());
    }

    #[test]
    fn rejects_absent_owner_and_required_shape_metadata() {
        let schema = scb();
        let absent = LegacyMissionPayloadMetadata::default();
        let absent_context =
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &absent, 21);
        with_reader(&[], |reader| {
            let error = absent_context
                .read_script_members(reader, 21, LegacyElementClass::Scroll)
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("initialized mission payload metadata")
            );
        });

        let mobile_metadata = metadata(22, LegacyElementClass::Mobile, None, None, None);
        let mobile =
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &mobile_metadata, 22);
        assert!(mobile.mobile_sprite_count(22, 8).is_err());

        let npc_metadata = metadata(23, LegacyElementClass::ActorNpcSoldier, None, None, None);
        let npc =
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &npc_metadata, 23);
        with_reader(&[], |reader| {
            let error = npc
                .read_local_ai(reader, 23, LegacyElementClass::ActorNpcSoldier)
                .unwrap_err();
            assert!(error.to_string().contains("local_ai_kind"));
        });
    }

    #[test]
    fn rejects_class_and_serialized_script_conflicts() {
        let schema = scb();
        let metadata = metadata(
            31,
            LegacyElementClass::ActorNpcSoldier,
            Some("Expected"),
            Some(LegacyLocalAiKind::Enemy),
            None,
        );
        let context =
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &metadata, 31);
        with_reader(&[], |reader| {
            let error = context
                .read_actor_script_members(reader, "Serialized")
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("initialized mission script class")
            );

            let error = context
                .read_local_ai(reader, 31, LegacyElementClass::ActorNpcCivilian)
                .unwrap_err();
            assert!(error.to_string().contains("initialized mission class"));
        });
    }
}
