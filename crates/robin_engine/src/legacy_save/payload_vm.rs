//! Compiled-script-guided decoding of Original v48 VM member state.
//!
//! `VMCore::SerializeMemberVariable` does not put a schema in the save. It
//! walks the bound SCB class's member list in declaration order. Ordinary
//! members are serialized as one opaque `SLONG`; native members delegate to
//! serializers registered by `RHEngine` for `Actor`, `Scroll`, and
//! `Location`. Consequently an RHSG payload cannot be decoded without the
//! exact mission SCB used to create it.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};
use crate::scb::{MemberVariable, ScbFile, TypeTag};

use super::payload_base::{
    LegacyContextValue, LegacyDecodedSection, LegacyElementRef, LegacyNamedValue, LegacyPoint2,
    LegacySectorRef,
};

const NULL_U32: u32 = u32::MAX;
const NULL_U16: u16 = u16::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyVmDecodeLimits {
    pub members_per_class: usize,
}

impl Default for LegacyVmDecodeLimits {
    fn default() -> Self {
        Self {
            members_per_class: 65_535,
        }
    }
}

/// Save-relevant projection of an SCB class.
///
/// Keeping this owned and serializable makes the otherwise implicit decoding
/// contract inspectable in save-import diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyVmClassSchema {
    pub class_name: String,
    pub heap_size: u32,
    pub members: Vec<LegacyVmMemberSchema>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyVmMemberSchema {
    pub name: String,
    pub address: u32,
    pub kind: LegacyVmMemberKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyVmMemberKind {
    /// The Original writes an `SLONG` for every tag except `NativeType`.
    /// `tag` is retained for later bool/int/float interpretation.
    Raw32 {
        tag: TypeTag,
    },
    ActorRef,
    ScrollRef,
    Location,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyVmMemberState {
    pub schema: LegacyVmMemberSchema,
    pub value: LegacyVmMemberValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LegacyVmMemberValue {
    /// Exact little-endian bits from the Original `SLONG` slot.
    Raw32 {
        bits: u32,
    },
    ActorRef(LegacyElementRef),
    ScrollRef(LegacyElementRef),
    Location(Option<LegacyVmLocation>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyVmLocation {
    /// Serialized but unused by `SerializeLocation`; preserve it for parity
    /// diagnostics instead of assuming that old saves always contain false.
    pub legacy_dummy: bool,
    pub position: LegacyPoint2,
    pub layer: u16,
    pub active: bool,
    pub sector: LegacySectorRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyVmMemberSection {
    pub class_name: String,
    pub members: Vec<LegacyVmMemberState>,
}

impl LegacyVmMemberSection {
    /// Adapt the typed result to the payload context's common diagnostic form.
    pub fn into_decoded_section(self) -> LegacyDecodedSection {
        let fields = self
            .members
            .into_iter()
            .map(|member| LegacyNamedValue {
                name: member.schema.name,
                value: member.value.into_context_value(),
            })
            .collect();
        LegacyDecodedSection {
            schema: format!("vm_class:{}", self.class_name),
            fields,
        }
    }
}

impl LegacyVmMemberValue {
    fn into_context_value(self) -> LegacyContextValue {
        match self {
            Self::Raw32 { bits } => LegacyContextValue::U32(bits),
            Self::ActorRef(reference) | Self::ScrollRef(reference) => {
                LegacyContextValue::ElementRef(reference)
            }
            Self::Location(None) => LegacyContextValue::Reference {
                kind: "Location".to_owned(),
                id: None,
            },
            Self::Location(Some(location)) => LegacyContextValue::Struct(vec![
                LegacyNamedValue {
                    name: "legacy_dummy".to_owned(),
                    value: LegacyContextValue::Bool(location.legacy_dummy),
                },
                LegacyNamedValue {
                    name: "position".to_owned(),
                    value: LegacyContextValue::Struct(vec![
                        LegacyNamedValue {
                            name: "x".to_owned(),
                            value: LegacyContextValue::F32(location.position.x),
                        },
                        LegacyNamedValue {
                            name: "y".to_owned(),
                            value: LegacyContextValue::F32(location.position.y),
                        },
                    ]),
                },
                LegacyNamedValue {
                    name: "layer".to_owned(),
                    value: LegacyContextValue::U32(u32::from(location.layer)),
                },
                LegacyNamedValue {
                    name: "active".to_owned(),
                    value: LegacyContextValue::Bool(location.active),
                },
                LegacyNamedValue {
                    name: "sector".to_owned(),
                    value: LegacyContextValue::Reference {
                        kind: "Sector".to_owned(),
                        id: location.sector.0.map(u32::from),
                    },
                },
            ]),
        }
    }
}

/// Adapter over the mission SCB used by payload decode contexts.
pub struct LegacyVmMemberDecoder<'a> {
    scb: &'a ScbFile,
    limits: LegacyVmDecodeLimits,
}

impl<'a> LegacyVmMemberDecoder<'a> {
    pub fn new(scb: &'a ScbFile, limits: LegacyVmDecodeLimits) -> Self {
        Self { scb, limits }
    }

    pub fn with_default_limits(scb: &'a ScbFile) -> Self {
        Self::new(scb, LegacyVmDecodeLimits::default())
    }

    pub fn read_class_members(
        &self,
        reader: &mut LegacyReader<'_>,
        class_name: &str,
    ) -> LegacyResult<LegacyVmMemberSection> {
        let schema = self.schema_for_class(reader, class_name)?;
        let mut members = Vec::new();
        let reserve_offset = reader.offset();
        members
            .try_reserve_exact(schema.members.len())
            .map_err(|_| {
                reader.allocation_error(
                    reserve_offset,
                    format!("class {class_name}.members"),
                    schema.members.len(),
                )
            })?;

        for member in &schema.members {
            let value = reader.scope(format!("member {}", member.name), |reader| {
                read_member_value(reader, &member.kind)
            })?;
            members.push(LegacyVmMemberState {
                schema: member.clone(),
                value,
            });
        }

        Ok(LegacyVmMemberSection {
            class_name: schema.class_name,
            members,
        })
    }

    pub fn read_decoded_section(
        &self,
        reader: &mut LegacyReader<'_>,
        class_name: &str,
    ) -> LegacyResult<LegacyDecodedSection> {
        self.read_class_members(reader, class_name)
            .map(LegacyVmMemberSection::into_decoded_section)
    }

    fn schema_for_class(
        &self,
        reader: &mut LegacyReader<'_>,
        class_name: &str,
    ) -> LegacyResult<LegacyVmClassSchema> {
        let schema_offset = reader.offset();
        let class = self
            .scb
            .classes
            .iter()
            .find(|class| class.class_name == class_name)
            .ok_or_else(|| {
                reader.invalid_value(
                    schema_offset,
                    "script_class",
                    class_name,
                    "a class present in the mission SCB",
                )
            })?;

        if class.member_variables.len() > self.limits.members_per_class {
            return Err(reader.invalid_value(
                schema_offset,
                format!("class {class_name}.member_count"),
                class.member_variables.len(),
                "member count within the caller-supplied limit",
            ));
        }
        let heap_size = u32::try_from(class.size_of_member_variables).map_err(|_| {
            reader.invalid_value(
                schema_offset,
                format!("class {class_name}.heap_size"),
                class.size_of_member_variables,
                "a non-negative 32-bit VM heap size",
            )
        })?;

        let mut members = Vec::new();
        members
            .try_reserve_exact(class.member_variables.len())
            .map_err(|_| {
                reader.allocation_error(
                    schema_offset,
                    format!("class {class_name}.member_schema"),
                    class.member_variables.len(),
                )
            })?;
        for (index, member) in class.member_variables.iter().enumerate() {
            members.push(project_member_schema(
                reader,
                schema_offset,
                class_name,
                heap_size,
                index,
                member,
            )?);
        }

        Ok(LegacyVmClassSchema {
            class_name: class.class_name.clone(),
            heap_size,
            members,
        })
    }
}

fn project_member_schema(
    reader: &mut LegacyReader<'_>,
    error_offset: u64,
    class_name: &str,
    heap_size: u32,
    index: usize,
    member: &MemberVariable,
) -> LegacyResult<LegacyVmMemberSchema> {
    let address = u32::try_from(member.address).map_err(|_| {
        reader.invalid_value(
            error_offset,
            format!("class {class_name}.members[{index}].address"),
            member.address,
            "a non-negative four-byte slot within the class VM heap",
        )
    })?;
    let end = address.checked_add(4).ok_or_else(|| {
        reader.invalid_value(
            error_offset,
            format!("class {class_name}.members[{index}].address"),
            address,
            "a non-negative four-byte slot within the class VM heap",
        )
    })?;
    if end > heap_size {
        return Err(reader.invalid_value(
            error_offset,
            format!("class {class_name}.members[{index}].address"),
            address,
            "a non-negative four-byte slot within the class VM heap",
        ));
    }

    let kind = if member.ty.tag != TypeTag::NativeType {
        LegacyVmMemberKind::Raw32 { tag: member.ty.tag }
    } else {
        match member.ty.native_type_name.as_str() {
            "Actor" => LegacyVmMemberKind::ActorRef,
            "Scroll" => LegacyVmMemberKind::ScrollRef,
            "Location" => LegacyVmMemberKind::Location,
            native_type => {
                return Err(reader.invalid_value(
                    error_offset,
                    format!("class {class_name}.members[{index}].native_type"),
                    native_type,
                    "Actor, Scroll, or Location (the RHEngine-registered serializers)",
                ));
            }
        }
    };

    Ok(LegacyVmMemberSchema {
        name: member.name.clone(),
        address,
        kind,
    })
}

fn read_member_value(
    reader: &mut LegacyReader<'_>,
    kind: &LegacyVmMemberKind,
) -> LegacyResult<LegacyVmMemberValue> {
    match kind {
        LegacyVmMemberKind::Raw32 { .. } => Ok(LegacyVmMemberValue::Raw32 {
            bits: reader.read_u32("raw_bits")?,
        }),
        LegacyVmMemberKind::ActorRef => Ok(LegacyVmMemberValue::ActorRef(read_element_ref(
            reader, "actor",
        )?)),
        LegacyVmMemberKind::ScrollRef => Ok(LegacyVmMemberValue::ScrollRef(read_element_ref(
            reader, "scroll",
        )?)),
        LegacyVmMemberKind::Location => read_location(reader).map(LegacyVmMemberValue::Location),
    }
}

fn read_element_ref(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
) -> LegacyResult<LegacyElementRef> {
    let creation_order = reader.read_u32(field)?;
    Ok(LegacyElementRef(
        (creation_order != NULL_U32).then_some(creation_order),
    ))
}

fn read_location(reader: &mut LegacyReader<'_>) -> LegacyResult<Option<LegacyVmLocation>> {
    if !reader.read_bool("initialized")? {
        return Ok(None);
    }

    let legacy_dummy = reader.read_bool("legacy_dummy")?;
    let position = LegacyPoint2 {
        x: reader.read_f32("x")?,
        y: reader.read_f32("y")?,
    };
    let layer = reader.read_u16("layer")?;
    let active = reader.read_bool("active")?;
    let sector_index = reader.read_u16("sector")?;
    let sector = LegacySectorRef((sector_index != NULL_U16).then_some(sector_index));
    Ok(Some(LegacyVmLocation {
        legacy_dummy,
        position,
        layer,
        active,
        sector,
    }))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::sbfile::{SB_FILE_READ, SbFile};
    use crate::scb::{ClassEntry, ScType};

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut fixture = NamedTempFile::new().unwrap();
        fixture.write_all(bytes).unwrap();
        fixture.flush().unwrap();
        let path = fixture.path().to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    fn member(name: &str, address: i32, tag: TypeTag, native_type_name: &str) -> MemberVariable {
        MemberVariable {
            ty: ScType {
                tag,
                native_type_name: native_type_name.to_owned(),
            },
            name: name.to_owned(),
            address,
        }
    }

    fn scb(heap_size: i32, members: Vec<MemberVariable>) -> ScbFile {
        ScbFile {
            version: 1.5,
            classes: vec![ClassEntry {
                source_file: "fixture.scs".to_owned(),
                class_name: "Fixture".to_owned(),
                size_of_member_variables: heap_size,
                member_variables: members,
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        }
    }

    #[test]
    fn decodes_members_in_scb_order_with_registered_native_semantics() {
        let schema = scb(
            16,
            vec![
                member("counter", 8, TypeTag::Int, ""),
                member("target", 0, TypeTag::NativeType, "Actor"),
                member("camera", 4, TypeTag::NativeType, "Location"),
                member("scroll", 12, TypeTag::NativeType, "Scroll"),
            ],
        );
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x89ab_cdef_u32.to_le_bytes());
        bytes.extend_from_slice(&42_u32.to_le_bytes());
        bytes.push(1);
        bytes.push(1);
        bytes.extend_from_slice(&1.25_f32.to_le_bytes());
        bytes.extend_from_slice(&(-9.5_f32).to_le_bytes());
        bytes.extend_from_slice(&3_u16.to_le_bytes());
        bytes.push(1);
        bytes.extend_from_slice(&77_u16.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());

        with_reader(&bytes, |reader| {
            let decoded = LegacyVmMemberDecoder::with_default_limits(&schema)
                .read_class_members(reader, "Fixture")
                .unwrap();
            assert_eq!(decoded.members.len(), 4);
            assert_eq!(
                decoded.members[0].value,
                LegacyVmMemberValue::Raw32 { bits: 0x89ab_cdef }
            );
            assert_eq!(
                decoded.members[1].value,
                LegacyVmMemberValue::ActorRef(LegacyElementRef(Some(42)))
            );
            assert_eq!(
                decoded.members[2].value,
                LegacyVmMemberValue::Location(Some(LegacyVmLocation {
                    legacy_dummy: true,
                    position: LegacyPoint2 { x: 1.25, y: -9.5 },
                    layer: 3,
                    active: true,
                    sector: LegacySectorRef(Some(77)),
                }))
            );
            assert_eq!(
                decoded.members[3].value,
                LegacyVmMemberValue::ScrollRef(LegacyElementRef(None))
            );
            assert_eq!(reader.offset(), bytes.len() as u64);
        });
    }

    #[test]
    fn null_location_consumes_only_its_initialized_byte() {
        let schema = scb(
            8,
            vec![
                member("location", 0, TypeTag::NativeType, "Location"),
                member("after", 4, TypeTag::Bool, ""),
            ],
        );
        let bytes = [0, 0x78, 0x56, 0x34, 0x12];
        with_reader(&bytes, |reader| {
            let decoded = LegacyVmMemberDecoder::with_default_limits(&schema)
                .read_class_members(reader, "Fixture")
                .unwrap();
            assert_eq!(
                decoded.members[0].value,
                LegacyVmMemberValue::Location(None)
            );
            assert_eq!(
                decoded.members[1].value,
                LegacyVmMemberValue::Raw32 { bits: 0x1234_5678 }
            );
            assert_eq!(reader.offset(), bytes.len() as u64);
        });
    }

    #[test]
    fn rejects_absent_classes_and_unregistered_native_types() {
        let missing_schema = scb(0, Vec::new());
        with_reader(&[], |reader| {
            let error = LegacyVmMemberDecoder::with_default_limits(&missing_schema)
                .read_class_members(reader, "Missing")
                .unwrap_err();
            assert!(error.to_string().contains("present in the mission SCB"));
        });

        let bad_native = scb(4, vec![member("door", 0, TypeTag::NativeType, "Door")]);
        with_reader(&[], |reader| {
            let error = LegacyVmMemberDecoder::with_default_limits(&bad_native)
                .read_class_members(reader, "Fixture")
                .unwrap_err();
            assert!(error.to_string().contains("Actor, Scroll, or Location"));
        });
    }

    #[test]
    fn rejects_member_addresses_outside_the_compiled_heap() {
        for address in [-1, 2] {
            let schema = scb(4, vec![member("bad", address, TypeTag::Int, "")]);
            with_reader(&[], |reader| {
                let error = LegacyVmMemberDecoder::with_default_limits(&schema)
                    .read_class_members(reader, "Fixture")
                    .unwrap_err();
                assert!(error.to_string().contains("four-byte slot"));
            });
        }
    }
}
