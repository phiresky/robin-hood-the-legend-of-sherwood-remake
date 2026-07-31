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

use super::LegacySaveAbiProfile;
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyMissionPayloadMetadata {
    /// Original `RHEngine::mulNumberOfCreatedStaticElements`. Creation
    /// orders below this boundary must resolve to initialized mission
    /// metadata; orders at or above it were reconstructed by the phase-one
    /// dynamic-element switch.
    pub static_creation_order_boundary: u32,
    pub by_creation_order: BTreeMap<u32, LegacyElementPayloadMetadata>,
}

impl Default for LegacyMissionPayloadMetadata {
    fn default() -> Self {
        Self {
            // A synthetic/default context has no proven dynamic boundary.
            // Treat every order as static so missing metadata remains a hard
            // error instead of silently enabling dynamic-class rules.
            static_creation_order_boundary: u32::MAX,
            by_creation_order: BTreeMap::new(),
        }
    }
}

/// Independent hard limits applied by the typed payload readers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyMissionPayloadDecodeLimits {
    pub sequences: LegacySequencePayloadLimits,
    pub local_ai: LegacyLocalAiLimits,
    pub vm: LegacyVmDecodeLimits,
}

/// Borrowed decoder context for the complete phase-two element stream.
pub struct LegacyMissionPayloadDecodeContext<'a> {
    scb: &'a ScbFile,
    metadata: &'a LegacyMissionPayloadMetadata,
    limits: LegacyMissionPayloadDecodeLimits,
    abi_profile: LegacySaveAbiProfile,
}

impl<'a> LegacyMissionPayloadDecodeContext<'a> {
    pub fn new(
        scb: &'a ScbFile,
        metadata: &'a LegacyMissionPayloadMetadata,
        limits: LegacyMissionPayloadDecodeLimits,
        abi_profile: LegacySaveAbiProfile,
    ) -> Self {
        Self {
            scb,
            metadata,
            limits,
            abi_profile,
        }
    }

    pub fn with_default_limits(
        scb: &'a ScbFile,
        metadata: &'a LegacyMissionPayloadMetadata,
    ) -> Self {
        Self::new(
            scb,
            metadata,
            LegacyMissionPayloadDecodeLimits::default(),
            LegacySaveAbiProfile::PortLinuxI386V48,
        )
    }

    pub fn with_default_limits_for_abi(
        scb: &'a ScbFile,
        metadata: &'a LegacyMissionPayloadMetadata,
        abi_profile: LegacySaveAbiProfile,
    ) -> Self {
        Self::new(
            scb,
            metadata,
            LegacyMissionPayloadDecodeLimits::default(),
            abi_profile,
        )
    }

    fn metadata_for<'r>(
        &self,
        reader: &mut LegacyReader<'r>,
        creation_order: u32,
    ) -> LegacyResult<&LegacyElementPayloadMetadata> {
        self.metadata
            .by_creation_order
            .get(&creation_order)
            .ok_or_else(|| {
                let offset = reader.offset();
                reader.invalid_value(
                    offset,
                    "creation_order",
                    creation_order,
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
        let metadata = self.metadata_for(reader, creation_order)?;
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

    fn is_dynamic(&self, creation_order: u32) -> bool {
        creation_order >= self.metadata.static_creation_order_boundary
    }

    fn validate_dynamic_pc(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
        callback: &'static str,
    ) -> LegacyResult<()> {
        if !self.is_dynamic(creation_order) || class != LegacyElementClass::ActorPc {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "creation_order",
                format_args!("{creation_order} ({class:?}) at {callback} callback"),
                "a dynamic ActorPc supported by Original's phase-one load switch",
            ));
        }
        Ok(())
    }

    fn validate_actor_owner(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<&LegacyElementPayloadMetadata> {
        let metadata = self.validate_identity(reader, creation_order, class)?;
        if !is_actor_class(class) {
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
    fn mobile_sprite_count(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        maximum: usize,
    ) -> LegacyResult<usize> {
        let metadata =
            self.validate_identity(reader, creation_order, LegacyElementClass::Mobile)?;
        if metadata.class != LegacyElementClass::Mobile {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "element_class",
                format_args!("{:?}", metadata.class),
                "Mobile at the mobile-sprite callback",
            ));
        }
        let count = metadata.mobile_sprite_count.ok_or_else(|| {
            let offset = reader.offset();
            reader.invalid_value(
                offset,
                "mobile_sprite_count",
                "missing",
                "the sprite count from the initialized mission mobile",
            )
        })?;
        if count > maximum {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "mobile_sprite_count",
                count,
                "a count within the caller-supplied decode limit",
            ));
        }
        Ok(count)
    }

    fn read_actor_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
        script_class: &str,
    ) -> LegacyResult<LegacyVmMemberSection> {
        if self.is_dynamic(creation_order) {
            self.validate_dynamic_pc(reader, creation_order, class, "actor VM-member")?;
            // Original phase one constructs dynamic PCs from their campaign
            // description, then RHElementActor::Serialize reads the
            // authoritative script-class string, binds it, and immediately
            // serializes that class's members. No mission-static lookup is
            // involved for this case.
            return LegacyVmMemberDecoder::new(self.scb, self.limits.vm)
                .read_class_members(reader, script_class);
        }

        let metadata = self.validate_actor_owner(reader, creation_order, class)?;
        let bound_class = metadata.script_class.as_deref().unwrap_or(script_class);
        // RHElementActor::Serialize overwrites mstrScriptClass from the save,
        // but an already initialized mission actor does not Bind that name
        // again. IsScripted() and SerializeMemberVariable() consequently use
        // the actor's existing VM binding. This is observable after BeamMe:
        // the serialized name can belong to the previous PC slot while the
        // member bytes retain the freshly initialized slot's live schema.
        // Conversely, when the freshly initialized mission actor has no live
        // binding, Original binds the serialized name before reading members,
        // so that name is the schema source.
        LegacyVmMemberDecoder::new(self.scb, self.limits.vm).read_class_members(reader, bound_class)
    }

    fn read_inline_sequence(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<LegacyInlineSequence> {
        if self.is_dynamic(creation_order) {
            self.validate_dynamic_pc(reader, creation_order, class, "inline-sequence")?;
        } else {
            self.validate_actor_owner(reader, creation_order, class)?;
        }
        LegacyInlineSequence::read(reader, &self.limits.sequences)
    }

    fn read_local_ai(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<Box<LegacyLocalAiPayload>> {
        if self.is_dynamic(creation_order) {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "creation_order",
                format_args!("{creation_order} ({class:?})"),
                "a static NPC; Original's dynamic load switch cannot construct NPCs",
            ));
        }
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
                abi_profile: self.abi_profile,
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
        if self.is_dynamic(creation_order) {
            if class == LegacyElementClass::Scroll {
                // Original constructs a dynamic RHElementScroll with its
                // default constructor. Phase one does not instantiate a
                // script class, and Scroll::Serialize writes no class name,
                // so IsClassInstanciate() is false and no member bytes exist.
                return Ok(None);
            }
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset,
                "creation_order",
                format_args!("{creation_order} ({class:?})"),
                "a static Target/Scroll or a dynamic Scroll",
            ));
        }
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
            static_creation_order_boundary: creation_order.saturating_add(1),
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
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &actor_metadata);
        with_reader(&0x1234_5678_u32.to_le_bytes(), |reader| {
            let section = actor
                .read_actor_script_members(
                    reader,
                    7,
                    LegacyElementClass::ActorNpcSoldier,
                    "Fixture",
                )
                .unwrap();
            assert_eq!(
                section.members[0].value,
                LegacyVmMemberValue::Raw32 { bits: 0x1234_5678 }
            );
        });

        let scroll_metadata = metadata(9, LegacyElementClass::Scroll, Some("Fixture"), None, None);
        let scroll =
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &scroll_metadata);
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
        let context = LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &metadata);
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
        let context = LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &metadata);
        with_reader(&[], |reader| {
            assert_eq!(context.mobile_sprite_count(reader, 13, 4).unwrap(), 3);
            assert!(context.mobile_sprite_count(reader, 13, 2).is_err());
        });
    }

    #[test]
    fn rejects_absent_owner_and_required_shape_metadata() {
        let schema = scb();
        let absent = LegacyMissionPayloadMetadata::default();
        let absent_context =
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &absent);
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
            LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &mobile_metadata);
        with_reader(&[], |reader| {
            assert!(mobile.mobile_sprite_count(reader, 22, 8).is_err());
        });

        let npc_metadata = metadata(23, LegacyElementClass::ActorNpcSoldier, None, None, None);
        let npc = LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &npc_metadata);
        with_reader(&[], |reader| {
            let error = npc
                .read_local_ai(reader, 23, LegacyElementClass::ActorNpcSoldier)
                .unwrap_err();
            assert!(error.to_string().contains("local_ai_kind"));
        });
    }

    #[test]
    fn static_actor_uses_live_binding_when_serialized_class_name_differs() {
        let schema = scb();
        let metadata = metadata(
            31,
            LegacyElementClass::ActorNpcSoldier,
            Some("Fixture"),
            Some(LegacyLocalAiKind::Enemy),
            None,
        );
        let context = LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &metadata);
        with_reader(&0x1020_3040_u32.to_le_bytes(), |reader| {
            let members = context
                .read_actor_script_members(
                    reader,
                    31,
                    LegacyElementClass::ActorNpcSoldier,
                    "Serialized",
                )
                .unwrap();
            assert_eq!(
                members.members[0].value,
                LegacyVmMemberValue::Raw32 { bits: 0x1020_3040 }
            );

            let error = context
                .read_local_ai(reader, 31, LegacyElementClass::ActorNpcCivilian)
                .unwrap_err();
            assert!(error.to_string().contains("initialized mission class"));
        });
    }

    #[test]
    fn dynamic_pc_uses_serialized_script_class_without_static_metadata() {
        let schema = scb();
        let metadata = LegacyMissionPayloadMetadata {
            static_creation_order_boundary: 40,
            by_creation_order: BTreeMap::new(),
        };
        let context = LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &metadata);
        with_reader(&0x1020_3040_u32.to_le_bytes(), |reader| {
            let members = context
                .read_actor_script_members(reader, 40, LegacyElementClass::ActorPc, "Fixture")
                .unwrap();
            assert_eq!(
                members.members[0].value,
                LegacyVmMemberValue::Raw32 { bits: 0x1020_3040 }
            );
        });
    }

    #[test]
    fn dynamic_scroll_has_no_implicit_script_members_and_other_shapes_stay_strict() {
        let schema = scb();
        let metadata = LegacyMissionPayloadMetadata {
            static_creation_order_boundary: 50,
            by_creation_order: BTreeMap::new(),
        };
        let context = LegacyMissionPayloadDecodeContext::with_default_limits(&schema, &metadata);
        with_reader(&[], |reader| {
            assert_eq!(
                context
                    .read_script_members(reader, 50, LegacyElementClass::Scroll)
                    .unwrap(),
                None
            );
            assert!(
                context
                    .read_script_members(reader, 51, LegacyElementClass::Target)
                    .unwrap_err()
                    .to_string()
                    .contains("dynamic Scroll")
            );
            assert!(
                context
                    .read_local_ai(reader, 52, LegacyElementClass::ActorNpcSoldier)
                    .unwrap_err()
                    .to_string()
                    .contains("cannot construct NPCs")
            );
            assert!(
                context
                    .read_actor_script_members(
                        reader,
                        53,
                        LegacyElementClass::ActorNpcSoldier,
                        "Fixture",
                    )
                    .unwrap_err()
                    .to_string()
                    .contains("dynamic ActorPc")
            );
        });
    }
}
