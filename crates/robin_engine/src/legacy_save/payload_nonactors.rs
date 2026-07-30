//! V48 payload readers for the non-actor element leaves used by shipped saves.
//!
//! These readers deliberately mirror the call order in each Original
//! `Serialize` method. In particular, Scroll writes its leaf state and script
//! members before calling Object, while Target writes its leaf state, script
//! members, and linked FX list before calling FX.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::elements::LegacyElementClass;
use super::payload_base::{
    LegacyDecodedSection, LegacyElementPayloadBase, LegacyElementRef, LegacyFxPayload,
    LegacyPayloadLimits, LegacyPoint2, read_element_ref,
};

const FINGERPRINT_OBJECT: [u8; 16] = hex16("90062155c12beef1e93d3c32cb21776f");
const FINGERPRINT_SCROLL: [u8; 16] = hex16("b02a77c4c704497d4cf06c506b5166e7");
const FINGERPRINT_TARGET: [u8; 16] = hex16("6554b7c74493712f6dbb7269f195aac7");
const FINGERPRINT_FX_MASKED: [u8; 16] = hex16("40b36826668c188dd5344e4b4c74c8e3");

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

/// Mission-initialized VM metadata required by Scroll and Target payloads.
///
/// Whether an element has an instantiated script class is not encoded in the
/// save. The context must therefore inspect the loaded mission element and
/// either consume the compiled class's ordered member schema or return `None`.
pub trait LegacyNonActorPayloadDecodeContext {
    fn read_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<Option<LegacyDecodedSection>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyNonActorPayloadLimits {
    pub target_linked_fxs: usize,
}

impl Default for LegacyNonActorPayloadLimits {
    fn default() -> Self {
        Self {
            target_linked_fxs: 65_535,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyRepulsivePointPayload {
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

impl LegacyRepulsivePointPayload {
    /// Read `RHRepulsivePoint::Serialize`.
    ///
    /// Retail Windows saves contain four pairs of `DOUBLE`s here. The Linux
    /// i386 port writes the same geometry as `FLOAT`s, so this is one of the
    /// few leaf-payload sites whose byte width genuinely depends on producer.
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
    ) -> LegacyResult<Self> {
        let position = match abi_profile {
            LegacySaveAbiProfile::RetailWindowsX86V48 => read_point2_f64(reader, "position")?,
            LegacySaveAbiProfile::PortLinuxI386V48 => read_point2_f32(reader, "position")?,
        };
        let concave = reader.read_bool("concave")?;
        let limit_left = match abi_profile {
            LegacySaveAbiProfile::RetailWindowsX86V48 => read_point2_f64(reader, "limit_left")?,
            LegacySaveAbiProfile::PortLinuxI386V48 => read_point2_f32(reader, "limit_left")?,
        };
        let limit_right = match abi_profile {
            LegacySaveAbiProfile::RetailWindowsX86V48 => read_point2_f64(reader, "limit_right")?,
            LegacySaveAbiProfile::PortLinuxI386V48 => read_point2_f32(reader, "limit_right")?,
        };
        Self::read_tail(reader, position, concave, limit_left, limit_right)
    }

    fn read_tail(
        reader: &mut LegacyReader<'_>,
        position: LegacyPoint2,
        concave: bool,
        limit_left: LegacyPoint2,
        limit_right: LegacyPoint2,
    ) -> LegacyResult<Self> {
        Ok(Self {
            position,
            concave,
            limit_left,
            limit_right,
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
pub struct LegacyObjectPayload {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub terminate: bool,
    pub register_number: u16,
    pub quantity: u16,
    pub animation: u32,
    pub object_type: u32,
    pub associated_action: u32,
    pub repulsive_point: LegacyRepulsivePointPayload,
    pub belongs_to_beggar: bool,
    pub taken: bool,
    pub element: LegacyElementPayloadBase,
    pub end_offset: u64,
}

pub fn read_object_payload(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
    limits: &LegacyPayloadLimits,
    expected_creation_order: u32,
    expected_class: LegacyElementClass,
) -> LegacyResult<LegacyObjectPayload> {
    reader.scope("object", |reader| {
        let start_offset = reader.offset();
        reader.read_signature(
            "fingerprint",
            FINGERPRINT_OBJECT,
            "MD5(\"RHElementObject\")",
        )?;
        let terminate = reader.read_bool("terminate")?;
        let register_number = reader.read_u16("register_number")?;
        let quantity = reader.read_u16("quantity")?;
        let animation = reader.read_u32("animation")?;
        let object_type = reader.read_u32("object_type")?;
        let associated_action = reader.read_u32("associated_action")?;
        let repulsive_point = reader.scope("repulsive_point", |reader| {
            LegacyRepulsivePointPayload::read(reader, abi_profile)
        })?;
        let belongs_to_beggar = reader.read_bool("belongs_to_beggar")?;
        let taken = reader.read_bool("taken")?;
        let element = reader.scope("element", |reader| {
            LegacyElementPayloadBase::read(
                reader,
                limits,
                Some(expected_creation_order),
                Some(expected_class),
            )
        })?;
        let end_offset = reader.offset();
        Ok(LegacyObjectPayload {
            abi_profile,
            start_offset,
            terminate,
            register_number,
            quantity,
            animation,
            object_type,
            associated_action,
            repulsive_point,
            belongs_to_beggar,
            taken,
            element,
            end_offset,
        })
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyBonusPayload {
    pub class: LegacyElementClass,
    pub object: LegacyObjectPayload,
}

/// Bonus subclasses do not override `Serialize`; every v48 bonus variant uses
/// exactly the Object payload.
pub fn read_bonus_payload(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
    limits: &LegacyPayloadLimits,
    expected_creation_order: u32,
    expected_class: LegacyElementClass,
) -> LegacyResult<LegacyBonusPayload> {
    if !is_bonus_class(expected_class) {
        let offset = reader.offset();
        return Err(reader.invalid_value(
            offset,
            "class",
            format_args!("{expected_class:?}"),
            "one of the RHCLASSID_BONUS_* variants",
        ));
    }
    Ok(LegacyBonusPayload {
        class: expected_class,
        object: read_object_payload(
            reader,
            abi_profile,
            limits,
            expected_creation_order,
            expected_class,
        )?,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyScrollPayload {
    pub start_offset: u64,
    pub status: u32,
    pub script_hourglass_timeout: u32,
    pub script_members: Option<LegacyDecodedSection>,
    pub object: LegacyObjectPayload,
    pub end_offset: u64,
}

pub fn read_scroll_payload(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
    limits: &LegacyPayloadLimits,
    context: &dyn LegacyNonActorPayloadDecodeContext,
    expected_creation_order: u32,
) -> LegacyResult<LegacyScrollPayload> {
    reader.scope("scroll", |reader| {
        let start_offset = reader.offset();
        reader.read_signature(
            "fingerprint",
            FINGERPRINT_SCROLL,
            "MD5(\"RHElementScroll\")",
        )?;
        let status = reader.read_u32("status")?;
        let script_hourglass_timeout = reader.read_u32("script_hourglass_timeout")?;
        let script_members = reader.scope("script_members", |reader| {
            context.read_script_members(reader, expected_creation_order, LegacyElementClass::Scroll)
        })?;
        let object = read_object_payload(
            reader,
            abi_profile,
            limits,
            expected_creation_order,
            LegacyElementClass::Scroll,
        )?;
        let end_offset = reader.offset();
        Ok(LegacyScrollPayload {
            start_offset,
            status,
            script_hourglass_timeout,
            script_members,
            object,
            end_offset,
        })
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyTargetPayload {
    pub start_offset: u64,
    pub animation: u32,
    pub progression: u32,
    pub script_members: Option<LegacyDecodedSection>,
    pub linked_fxs: Vec<LegacyElementRef>,
    pub fx: LegacyFxPayload,
    pub end_offset: u64,
}

pub fn read_target_payload(
    reader: &mut LegacyReader<'_>,
    limits: &LegacyPayloadLimits,
    leaf_limits: &LegacyNonActorPayloadLimits,
    context: &dyn LegacyNonActorPayloadDecodeContext,
    expected_creation_order: u32,
) -> LegacyResult<LegacyTargetPayload> {
    reader.scope("target", |reader| {
        let start_offset = reader.offset();
        reader.read_signature(
            "fingerprint",
            FINGERPRINT_TARGET,
            "MD5(\"RHElementTarget\")",
        )?;
        let animation = reader.read_u32("animation")?;
        let progression = reader.read_u32("progression")?;
        let script_members = reader.scope("script_members", |reader| {
            context.read_script_members(reader, expected_creation_order, LegacyElementClass::Target)
        })?;
        let linked_count =
            reader.read_count_u32("linked_fxs.count", leaf_limits.target_linked_fxs)?;
        let mut linked_fxs = Vec::new();
        let allocation_offset = reader.offset();
        linked_fxs
            .try_reserve_exact(linked_count)
            .map_err(|_| reader.allocation_error(allocation_offset, "linked_fxs", linked_count))?;
        for index in 0..linked_count {
            linked_fxs.push(read_element_ref(reader, format!("linked_fxs[{index}]"))?);
        }
        let fx = reader.scope("fx", |reader| {
            LegacyFxPayload::read(
                reader,
                limits,
                Some(expected_creation_order),
                Some(LegacyElementClass::Target),
            )
        })?;
        let end_offset = reader.offset();
        Ok(LegacyTargetPayload {
            start_offset,
            animation,
            progression,
            script_members,
            linked_fxs,
            fx,
            end_offset,
        })
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyStandaloneFxPayload {
    pub start_offset: u64,
    pub fx: LegacyFxPayload,
    pub end_offset: u64,
}

pub fn read_fx_payload(
    reader: &mut LegacyReader<'_>,
    limits: &LegacyPayloadLimits,
    expected_creation_order: u32,
) -> LegacyResult<LegacyStandaloneFxPayload> {
    reader.scope("fx_leaf", |reader| {
        let start_offset = reader.offset();
        let fx = LegacyFxPayload::read(
            reader,
            limits,
            Some(expected_creation_order),
            Some(LegacyElementClass::Fx),
        )?;
        let end_offset = reader.offset();
        Ok(LegacyStandaloneFxPayload {
            start_offset,
            fx,
            end_offset,
        })
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyStandaloneFxMaskedPayload {
    pub start_offset: u64,
    pub animation_speed: f32,
    pub element: LegacyElementPayloadBase,
    pub end_offset: u64,
}

pub fn read_fx_masked_payload(
    reader: &mut LegacyReader<'_>,
    limits: &LegacyPayloadLimits,
    expected_creation_order: u32,
) -> LegacyResult<LegacyStandaloneFxMaskedPayload> {
    reader.scope("fx_masked_leaf", |reader| {
        let start_offset = reader.offset();
        reader.read_signature(
            "fingerprint",
            FINGERPRINT_FX_MASKED,
            "MD5(\"RHElementFXMasked\")",
        )?;
        let animation_speed = reader.read_f32("animation_speed")?;
        let element = reader.scope("element", |reader| {
            LegacyElementPayloadBase::read(
                reader,
                limits,
                Some(expected_creation_order),
                Some(LegacyElementClass::FxMasked),
            )
        })?;
        let end_offset = reader.offset();
        Ok(LegacyStandaloneFxMaskedPayload {
            start_offset,
            animation_speed,
            element,
            end_offset,
        })
    })
}

fn read_point2_f32(
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

fn read_point2_f64(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyPoint2> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyPoint2 {
            x: read_f64(reader, "x")? as f32,
            y: read_f64(reader, "y")? as f32,
        })
    })
}

fn read_f64(reader: &mut LegacyReader<'_>, field: impl std::fmt::Display) -> LegacyResult<f64> {
    let mut bytes = [0; 8];
    reader.read_bytes(field, &mut bytes)?;
    Ok(f64::from_le_bytes(bytes))
}

fn is_bonus_class(class: LegacyElementClass) -> bool {
    matches!(
        class,
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
            | LegacyElementClass::BonusSwordOfTheState
    )
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::legacy_io::LegacyIoErrorKind;
    use crate::sbfile::{SB_FILE_READ, SbFile};

    struct NoScript;

    impl LegacyNonActorPayloadDecodeContext for NoScript {
        fn read_script_members(
            &self,
            _reader: &mut LegacyReader<'_>,
            _creation_order: u32,
            _class: LegacyElementClass,
        ) -> LegacyResult<Option<LegacyDecodedSection>> {
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

    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f64(bytes: &mut Vec<u8>, value: f64) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_repulsive_tail(bytes: &mut Vec<u8>) {
        for value in [10.0, 11.0, 12.0, 13.0] {
            push_f32(bytes, value);
        }
        push_u32(bytes, 14);
        bytes.extend_from_slice(&[1, 0, 1, 0]);
    }

    #[test]
    fn repulsive_point_honors_windows_double_and_linux_float_boundaries() {
        let mut windows = Vec::new();
        push_f64(&mut windows, 1.25);
        push_f64(&mut windows, 2.5);
        windows.push(1);
        for value in [3.0, 4.0, 5.0, 6.0] {
            push_f64(&mut windows, value);
        }
        push_repulsive_tail(&mut windows);

        let mut linux = Vec::new();
        for value in [1.25, 2.5] {
            push_f32(&mut linux, value);
        }
        linux.push(1);
        for value in [3.0, 4.0, 5.0, 6.0] {
            push_f32(&mut linux, value);
        }
        push_repulsive_tail(&mut linux);

        for (bytes, abi) in [
            (&windows, LegacySaveAbiProfile::RetailWindowsX86V48),
            (&linux, LegacySaveAbiProfile::PortLinuxI386V48),
        ] {
            with_reader(bytes, |reader| {
                let payload = LegacyRepulsivePointPayload::read(reader, abi).unwrap();
                assert_eq!(payload.position, LegacyPoint2 { x: 1.25, y: 2.5 });
                assert_eq!(payload.limit_left, LegacyPoint2 { x: 3.0, y: 4.0 });
                assert_eq!(payload.limit_right, LegacyPoint2 { x: 5.0, y: 6.0 });
                assert!(payload.concave);
                assert_eq!(payload.id, 14);
                assert_eq!(reader.offset(), bytes.len() as u64);
            });
        }
        assert_eq!(windows.len() - linux.len(), 24);
    }

    #[test]
    fn target_invokes_script_callback_before_linked_fx_list() {
        struct MarkerScript;
        impl LegacyNonActorPayloadDecodeContext for MarkerScript {
            fn read_script_members(
                &self,
                reader: &mut LegacyReader<'_>,
                creation_order: u32,
                class: LegacyElementClass,
            ) -> LegacyResult<Option<LegacyDecodedSection>> {
                assert_eq!(creation_order, 77);
                assert_eq!(class, LegacyElementClass::Target);
                assert_eq!(reader.read_u32("marker")?, 0xfeed_beef);
                Ok(None)
            }
        }

        let mut bytes = FINGERPRINT_TARGET.to_vec();
        push_u32(&mut bytes, 1);
        push_u32(&mut bytes, 2);
        push_u32(&mut bytes, 0xfeed_beef);
        push_u32(&mut bytes, 1);
        push_u32(&mut bytes, 123);
        // Stop at the parent FX signature; the failure offset proves all leaf
        // fields and the context-owned script bytes were consumed in order.
        bytes.extend_from_slice(&[0; 16]);

        with_reader(&bytes, |reader| {
            let error = read_target_payload(
                reader,
                &LegacyPayloadLimits::default(),
                &LegacyNonActorPayloadLimits::default(),
                &MarkerScript,
                77,
            )
            .unwrap_err();
            assert_eq!(error.offset, 36);
            assert_eq!(error.field, "target.fx.fingerprint");
        });
    }

    #[test]
    fn target_rejects_link_count_before_allocation_or_parent_parse() {
        let mut bytes = FINGERPRINT_TARGET.to_vec();
        push_u32(&mut bytes, 1);
        push_u32(&mut bytes, 2);
        push_u32(&mut bytes, 3);

        with_reader(&bytes, |reader| {
            let error = read_target_payload(
                reader,
                &LegacyPayloadLimits::default(),
                &LegacyNonActorPayloadLimits {
                    target_linked_fxs: 2,
                },
                &NoScript,
                77,
            )
            .unwrap_err();
            assert_eq!(error.offset, 24);
            assert_eq!(error.field, "target.linked_fxs.count");
            assert!(matches!(
                error.kind,
                LegacyIoErrorKind::CountLimit {
                    count: 3,
                    maximum: 2
                }
            ));
        });
    }

    #[test]
    fn scroll_reports_its_own_signature_without_entering_object_payload() {
        with_reader(&[0; 16], |reader| {
            let error = read_scroll_payload(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacyPayloadLimits::default(),
                &NoScript,
                12,
            )
            .unwrap_err();
            assert_eq!(error.offset, 0);
            assert_eq!(error.field, "scroll.fingerprint");
        });
    }

    #[test]
    fn bonus_reader_rejects_non_bonus_class_without_consuming_bytes() {
        with_reader(&[0xaa], |reader| {
            let error = read_bonus_payload(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacyPayloadLimits::default(),
                1,
                LegacyElementClass::Object,
            )
            .unwrap_err();
            assert_eq!(error.offset, 0);
            assert_eq!(error.field, "class");
            assert_eq!(reader.offset(), 0);
        });
    }
}
