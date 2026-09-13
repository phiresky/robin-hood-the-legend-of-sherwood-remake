//! V48 payload readers for the non-actor element leaves used by shipped saves.
//!
//! These readers deliberately mirror the call order in each Original
//! `Serialize` method. In particular, Scroll writes its leaf state and script
//! members before calling Object, while Target writes its leaf state, script
//! members, and linked FX list before calling FX. Field declaration order is
//! wire order.

use super::read_helpers::DEFAULT_BULK_LIMIT;
use super::read_helpers::hex16;
use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyRead, LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::elements::LegacyElementClass;
use super::payload_base::{
    LegacyElementBaseDecode, LegacyElementPayloadBase, LegacyElementRef, LegacyFxPayload,
    LegacyPayloadLimits, LegacyPoint2,
};
use super::payload_vm::LegacyVmMemberSection;

const FINGERPRINT_OBJECT: [u8; 16] = hex16("90062155c12beef1e93d3c32cb21776f");
const FINGERPRINT_SCROLL: [u8; 16] = hex16("b02a77c4c704497d4cf06c506b5166e7");
const FINGERPRINT_TARGET: [u8; 16] = hex16("6554b7c74493712f6dbb7269f195aac7");
const FINGERPRINT_FX_MASKED: [u8; 16] = hex16("40b36826668c188dd5344e4b4c74c8e3");

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
    ) -> LegacyResult<Option<LegacyVmMemberSection>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyNonActorPayloadLimits {
    pub target_linked_fxs: usize,
}

impl Default for LegacyNonActorPayloadLimits {
    fn default() -> Self {
        Self {
            target_linked_fxs: DEFAULT_BULK_LIMIT,
        }
    }
}

/// Context: the producer ABI, which decides the geometry width.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacySaveAbiProfile)]
pub struct LegacyRepulsivePointPayload {
    #[legacy(read = read_abi_point2(reader, "position", *ctx))]
    pub position: LegacyPoint2,
    pub concave: bool,
    #[legacy(read = read_abi_point2(reader, "limit_left", *ctx))]
    pub limit_left: LegacyPoint2,
    #[legacy(read = read_abi_point2(reader, "limit_right", *ctx))]
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
    /// Read a repulsive-point payload.
    ///
    /// Retail Windows saves contain four pairs of double-precision values here. The Linux
    /// i386 port writes the same geometry in single precision, so this is one of the
    /// few leaf-payload sites whose byte width genuinely depends on producer.
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
    ) -> LegacyResult<Self> {
        <Self as LegacyRead<_>>::read(reader, &abi_profile)
    }
}

fn read_abi_point2(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    abi_profile: LegacySaveAbiProfile,
) -> LegacyResult<LegacyPoint2> {
    match abi_profile {
        LegacySaveAbiProfile::RetailWindowsX86V48 => reader.scope(field, |reader| {
            Ok(LegacyPoint2 {
                x: reader.read_f64("x")? as f32,
                y: reader.read_f64("y")? as f32,
            })
        }),
        LegacySaveAbiProfile::PortLinuxI386V48 => LegacyPoint2::read_field(reader, field, &()),
    }
}

/// Decode context for the Object payload and the leaves that embed it.
#[derive(Clone, Copy)]
pub struct LegacyObjectDecode<'a> {
    pub abi_profile: LegacySaveAbiProfile,
    pub limits: &'a LegacyPayloadLimits,
    pub creation_order: u32,
    pub class: LegacyElementClass,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyObjectDecode<'_>)]
pub struct LegacyObjectPayload {
    #[legacy(value = ctx.abi_profile)]
    pub abi_profile: LegacySaveAbiProfile,
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(fingerprint = FINGERPRINT_OBJECT, expected = "object-element fingerprint")]
    pub terminate: bool,
    pub register_number: u16,
    pub quantity: u16,
    pub animation: u32,
    pub object_type: u32,
    pub associated_action: u32,
    // Retail object payloads use the narrow geometry layout here.
    // The wide Windows compatibility form is specific to the Human
    // serializer; object elements use the ordinary serializer.
    #[legacy(read = LegacyRepulsivePointPayload::read_field(
        reader,
        "repulsive_point",
        &LegacySaveAbiProfile::PortLinuxI386V48,
    ))]
    pub repulsive_point: LegacyRepulsivePointPayload,
    pub belongs_to_beggar: bool,
    pub taken: bool,
    #[legacy(read = LegacyElementPayloadBase::read_field(
        reader,
        "element",
        &LegacyElementBaseDecode {
            limits: ctx.limits,
            expected_creation_order: Some(ctx.creation_order),
            expected_class: Some(ctx.class),
        },
    ))]
    pub element: LegacyElementPayloadBase,
    #[legacy(offset)]
    pub end_offset: u64,
}

pub fn read_object_payload(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
    limits: &LegacyPayloadLimits,
    expected_creation_order: u32,
    expected_class: LegacyElementClass,
) -> LegacyResult<LegacyObjectPayload> {
    LegacyObjectPayload::read_field(
        reader,
        "object",
        &LegacyObjectDecode {
            abi_profile,
            limits,
            creation_order: expected_creation_order,
            class: expected_class,
        },
    )
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
    pub script_members: Option<LegacyVmMemberSection>,
    pub object: LegacyObjectPayload,
    pub end_offset: u64,
}

// Hand-written: script members are decoded through the mission context.
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
            "scroll-element fingerprint",
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
    pub script_members: Option<LegacyVmMemberSection>,
    pub linked_fxs: Vec<LegacyElementRef>,
    pub fx: LegacyFxPayload,
    pub end_offset: u64,
}

// Hand-written: script members are decoded through the mission context.
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
            "target-element fingerprint",
        )?;
        let animation = reader.read_u32("animation")?;
        let progression = reader.read_u32("progression")?;
        let script_members = reader.scope("script_members", |reader| {
            context.read_script_members(reader, expected_creation_order, LegacyElementClass::Target)
        })?;
        let linked_count =
            reader.read_count_u32("linked_fxs.count", leaf_limits.target_linked_fxs)?;
        let linked_fxs = reader.read_list("linked_fxs", linked_count, |reader, item| {
            LegacyElementRef::read_field(reader, item, &())
        })?;
        let fx = LegacyFxPayload::read_field(
            reader,
            "fx",
            &LegacyElementBaseDecode {
                limits,
                expected_creation_order: Some(expected_creation_order),
                expected_class: Some(LegacyElementClass::Target),
            },
        )?;
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyElementBaseDecode<'_>)]
pub struct LegacyStandaloneFxPayload {
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(flatten)]
    pub fx: LegacyFxPayload,
    #[legacy(offset)]
    pub end_offset: u64,
}

pub fn read_fx_payload(
    reader: &mut LegacyReader<'_>,
    limits: &LegacyPayloadLimits,
    expected_creation_order: u32,
) -> LegacyResult<LegacyStandaloneFxPayload> {
    LegacyStandaloneFxPayload::read_field(
        reader,
        "fx_leaf",
        &LegacyElementBaseDecode {
            limits,
            expected_creation_order: Some(expected_creation_order),
            expected_class: Some(LegacyElementClass::Fx),
        },
    )
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyElementBaseDecode<'_>)]
pub struct LegacyStandaloneFxMaskedPayload {
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(
        fingerprint = FINGERPRINT_FX_MASKED,
        expected = "masked-effect-element fingerprint"
    )]
    pub animation_speed: f32,
    pub element: LegacyElementPayloadBase,
    #[legacy(offset)]
    pub end_offset: u64,
}

pub fn read_fx_masked_payload(
    reader: &mut LegacyReader<'_>,
    limits: &LegacyPayloadLimits,
    expected_creation_order: u32,
) -> LegacyResult<LegacyStandaloneFxMaskedPayload> {
    LegacyStandaloneFxMaskedPayload::read_field(
        reader,
        "fx_masked_leaf",
        &LegacyElementBaseDecode {
            limits,
            expected_creation_order: Some(expected_creation_order),
            expected_class: Some(LegacyElementClass::FxMasked),
        },
    )
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
    use crate::legacy_save::test_support::{push_f32, push_u32};

    use super::*;
    use crate::legacy_io::LegacyIoErrorKind;

    struct NoScript;

    impl LegacyNonActorPayloadDecodeContext for NoScript {
        fn read_script_members(
            &self,
            _reader: &mut LegacyReader<'_>,
            _creation_order: u32,
            _class: LegacyElementClass,
        ) -> LegacyResult<Option<LegacyVmMemberSection>> {
            Ok(None)
        }
    }

    use crate::legacy_save::test_support::with_reader;

    fn push_f64(bytes: &mut Vec<u8>, value: f64) {
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
    fn windows_repulsive_point_truncation_reports_the_scoped_component() {
        let bytes = [0; 12];
        with_reader(&bytes, |reader| {
            let error = LegacyRepulsivePointPayload::read(
                reader,
                LegacySaveAbiProfile::RetailWindowsX86V48,
            )
            .unwrap_err();
            assert_eq!(error.offset, 8);
            assert_eq!(error.field, "position.y");
        });
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
            ) -> LegacyResult<Option<LegacyVmMemberSection>> {
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
