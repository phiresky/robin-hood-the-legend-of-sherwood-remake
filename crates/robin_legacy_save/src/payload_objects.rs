//! Original v48 phase-two payloads for objects, projectiles, and mobile scenery.
//!
//! The apparent object hierarchy is not enough to decode these
//! records. Several leaf serializers write state before calling their parent,
//! while the wasp element deliberately uses object serialization
//! despite being projectile elements. The readers below mirror the
//! exact `Serialize` call order: field declaration order is wire order.

use super::read_helpers::DEFAULT_BULK_LIMIT;
use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyRead, LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::elements::LegacyElementClass;
use super::payload_base::{
    LegacyElementRef, LegacyMobilePayload, LegacyOpaquePointer32, LegacyPayloadDecodeContext,
    LegacyPoint2, LegacyPoint3, LegacySectorRef, read_nullable_u32_ref,
};
use super::payload_dispatch::LegacyElementPayloadLimits;
use super::payload_nonactors::{LegacyObjectDecode, LegacyObjectPayload, read_object_payload};

const FINGERPRINT_PROJECTILE: [u8; 16] = [
    0x69, 0x33, 0xdb, 0xbc, 0x6b, 0xe4, 0x35, 0xf3, 0x24, 0x9e, 0x7c, 0xf1, 0xc9, 0x94, 0xfb, 0x2d,
];
const FINGERPRINT_ARROW: [u8; 16] = [
    0x03, 0x10, 0x5b, 0x49, 0x6c, 0x3f, 0x67, 0xae, 0x87, 0x9d, 0xd3, 0x72, 0xbe, 0x1b, 0x60, 0x16,
];
const FINGERPRINT_PURSE: [u8; 16] = [
    0xe6, 0x7b, 0x40, 0x33, 0xb3, 0xde, 0x85, 0x76, 0x83, 0x22, 0x51, 0xd8, 0x8d, 0x7b, 0x26, 0x19,
];
const FINGERPRINT_WASP: [u8; 16] = [
    0x09, 0x0f, 0xa7, 0xd8, 0xfc, 0x82, 0x0a, 0x50, 0x98, 0x2c, 0x07, 0xc4, 0x42, 0xee, 0x5d, 0x7b,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyObjectPayloadLimits {
    pub trajectory_points: usize,
    pub net_victims: usize,
}

impl Default for LegacyObjectPayloadLimits {
    fn default() -> Self {
        Self {
            trajectory_points: DEFAULT_BULK_LIMIT,
            net_victims: DEFAULT_BULK_LIMIT,
        }
    }
}

/// Decode context for projectile leaves: the embedded Object identity plus
/// the projectile-specific limits.
#[derive(Clone, Copy)]
pub struct LegacyProjectileDecode<'a> {
    pub object: LegacyObjectDecode<'a>,
    pub limits: &'a LegacyObjectPayloadLimits,
}

/// Complete phase-two payload for any object/item class handled by this
/// module. The enum keeps classes with identical inherited grammars distinct,
/// which makes conversion and diagnostics independent of the numeric class ID.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LegacyObjectItemPayload {
    Object(LegacyObjectPayload),
    Arrow(LegacyArrowPayload),
    Apple(LegacyApplePayload),
    Purse(LegacyPursePayload),
    Stone(LegacyStonePayload),
    WaspNest(LegacyWaspNestPayload),
    Wasp(LegacyWaspPayload),
    Net(LegacyNetPayload),
    Coin(LegacyCoinPayload),
    Ale(LegacyAlePayload),
    SpyCape(LegacySpyCapePayload),
    Mobile(LegacyMobilePayload),
}

/// Dispatch one complete phase-two payload using the class from its phase-one
/// envelope.
pub fn read_object_item_payload(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
    payload_limits: &LegacyElementPayloadLimits,
    context: &dyn LegacyPayloadDecodeContext,
    creation_order: u32,
    class: LegacyElementClass,
) -> LegacyResult<LegacyObjectItemPayload> {
    let base_limits = &payload_limits.base;
    // Every leaf passes its own concrete class down to the Object parent.
    let object = LegacyObjectDecode {
        abi_profile,
        limits: base_limits,
        creation_order,
        class,
    };
    let projectile = LegacyProjectileDecode {
        object,
        limits: &payload_limits.objects,
    };
    reader.scope(format!("object_item.{class:?}"), |reader| {
        Ok(match class {
            LegacyElementClass::Object => LegacyObjectItemPayload::Object(read_object_payload(
                reader,
                abi_profile,
                base_limits,
                creation_order,
                class,
            )?),
            LegacyElementClass::Arrow => LegacyObjectItemPayload::Arrow(
                LegacyArrowPayload::read_field(reader, "arrow", &projectile)?,
            ),
            LegacyElementClass::Apple => LegacyObjectItemPayload::Apple(LegacyApplePayload {
                projectile: LegacyProjectilePayload::read_field(reader, "projectile", &projectile)?,
            }),
            LegacyElementClass::Purse => LegacyObjectItemPayload::Purse(
                LegacyPursePayload::read_field(reader, "purse", &projectile)?,
            ),
            LegacyElementClass::Stone => LegacyObjectItemPayload::Stone(LegacyStonePayload {
                projectile: LegacyProjectilePayload::read_field(reader, "projectile", &projectile)?,
            }),
            LegacyElementClass::WaspNest => LegacyObjectItemPayload::WaspNest(
                LegacyWaspNestPayload::read_field(reader, "wasp_nest", &projectile)?,
            ),
            LegacyElementClass::Wasp => LegacyObjectItemPayload::Wasp(
                LegacyWaspPayload::read_field(reader, "wasp", &object)?,
            ),
            LegacyElementClass::Net => LegacyObjectItemPayload::Net(LegacyNetPayload::read_field(
                reader,
                "net",
                &projectile,
            )?),
            LegacyElementClass::Coin => LegacyObjectItemPayload::Coin(
                LegacyCoinPayload::read_field(reader, "coin", &projectile)?,
            ),
            LegacyElementClass::Ale => LegacyObjectItemPayload::Ale(LegacyAlePayload {
                object: read_object_payload(
                    reader,
                    abi_profile,
                    base_limits,
                    creation_order,
                    class,
                )?,
            }),
            LegacyElementClass::SpyCape => LegacyObjectItemPayload::SpyCape(LegacySpyCapePayload {
                object: read_object_payload(
                    reader,
                    abi_profile,
                    base_limits,
                    creation_order,
                    class,
                )?,
            }),
            LegacyElementClass::Mobile => LegacyObjectItemPayload::Mobile(
                LegacyMobilePayload::read(reader, base_limits, context, creation_order)?,
            ),
            _ => {
                let offset = reader.offset();
                return Err(reader.invalid_value(
                    offset,
                    "class",
                    format_args!("{class:?}"),
                    "object, projectile, item, or mobile class handled by payload_objects",
                ));
            }
        })
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyProjectileDecode<'_>,
    fingerprint = FINGERPRINT_PROJECTILE,
    expected = "projectile-element fingerprint"
)]
pub struct LegacyProjectilePayload {
    pub flying: bool,
    pub dive: bool,
    pub magic_bullet: bool,
    pub frame_count: u16,
    pub trajectory_origin_map: LegacyPoint2,
    /// Raw Win32/i386 reference bytes written for the sector. They are
    /// not authoritative; `trajectory_origin_sector` follows after the
    /// audited two-byte struct padding.
    pub trajectory_origin_sector_pointer: LegacyOpaquePointer32,
    pub trajectory_origin_level: u16,
    #[legacy(bytes)]
    pub trajectory_origin_padding: [u8; 2],
    pub trajectory_origin_sector: LegacySectorRef,
    pub flight_direction: u16,
    pub start: LegacyPoint3,
    pub end: LegacyPoint3,
    pub shooter: LegacyElementRef,
    #[legacy(count_u32 = ctx.limits.trajectory_points)]
    pub trajectory: Vec<LegacyTrajectoryPoint>,
    #[legacy(read = LegacyObjectPayload::read_field(reader, "object", &ctx.object))]
    pub object: LegacyObjectPayload,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyTrajectoryPoint {
    pub time: u16,
    pub bounce: bool,
    pub material: u32,
    pub position: LegacyPoint3,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyProjectileDecode<'_>,
    fingerprint = FINGERPRINT_ARROW,
    expected = "arrow-element fingerprint"
)]
pub struct LegacyArrowPayload {
    pub projectile: LegacyProjectilePayload,
    #[legacy(read = read_bow(reader))]
    pub bow: Option<LegacyBowPayload>,
    pub flat_shot: bool,
    pub falling: bool,
    pub falling_direction: u8,
    pub last_sector: u8,
    pub last_azimuth: i16,
    pub play_impact: bool,
}

fn read_bow(reader: &mut LegacyReader<'_>) -> LegacyResult<Option<LegacyBowPayload>> {
    if !reader.read_bool("has_bow")? {
        return Ok(None);
    }
    Ok(Some(LegacyBowPayload {
        profile: read_nullable_u32_ref(reader, "bow_profile")?,
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyBowPayload {
    /// Index in the shoot-profile collection, or null for a
    /// structurally present default-constructed bow.
    pub profile: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyApplePayload {
    pub projectile: LegacyProjectilePayload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyStonePayload {
    pub projectile: LegacyProjectilePayload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyProjectileDecode<'_>,
    fingerprint = FINGERPRINT_PURSE,
    expected = "purse-element fingerprint"
)]
pub struct LegacyPursePayload {
    pub number_of_coins: u16,
    pub projectile: LegacyProjectilePayload,
}

/// Wasp-nest serialization has no stream-validation call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyProjectileDecode<'_>)]
pub struct LegacyWaspNestPayload {
    pub projectile: LegacyProjectilePayload,
    pub flying_wasp_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyObjectDecode<'_>,
    fingerprint = FINGERPRINT_WASP,
    expected = "wasp-element fingerprint"
)]
pub struct LegacyWaspPayload {
    pub nest: LegacyElementRef,
    pub victim: LegacyElementRef,
    pub stinging: bool,
    pub timeout: u32,
    pub movement: LegacyPoint3,
    /// Intentional Original behavior: wasps inherit Projectile but
    /// serialize only their Object base.
    pub object: LegacyObjectPayload,
}

/// Net serialization has no stream-validation call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyProjectileDecode<'_>)]
pub struct LegacyNetPayload {
    pub projectile: LegacyProjectilePayload,
    #[legacy(count_u16 = ctx.limits.net_victims)]
    pub victims: Vec<LegacyElementRef>,
    pub time_until_unfolding: u32,
    pub crumpled: bool,
    pub was_flying: bool,
}

/// Coin serialization has no stream-validation call and stores its leaf
/// reference before invoking Projectile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyProjectileDecode<'_>)]
pub struct LegacyCoinPayload {
    pub source_purse: LegacyElementRef,
    pub projectile: LegacyProjectilePayload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyAlePayload {
    pub object: LegacyObjectPayload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySpyCapePayload {
    pub object: LegacyObjectPayload,
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::legacy_save::payload_base::LegacyPayloadLimits;

    use crate::legacy_save::test_support::with_reader;

    fn object_decode(
        base_limits: &LegacyPayloadLimits,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyObjectDecode<'_> {
        LegacyObjectDecode {
            abi_profile: LegacySaveAbiProfile::PortLinuxI386V48,
            limits: base_limits,
            creation_order,
            class,
        }
    }

    fn projectile_decode<'a>(
        limits: &'a LegacyObjectPayloadLimits,
        base_limits: &'a LegacyPayloadLimits,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyProjectileDecode<'a> {
        LegacyProjectileDecode {
            object: object_decode(base_limits, creation_order, class),
            limits,
        }
    }

    #[test]
    fn trajectory_count_is_rejected_before_payload_allocation() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FINGERPRINT_PROJECTILE);
        bytes.extend_from_slice(&[0, 0, 0]);
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&0x1234_5678_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&[0xa4, 0xd5]);
        bytes.extend_from_slice(&u16::MAX.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        let limits = LegacyObjectPayloadLimits {
            trajectory_points: 1,
            ..LegacyObjectPayloadLimits::default()
        };
        let base_limits = LegacyPayloadLimits::default();
        with_reader(&bytes, |reader| {
            let error = LegacyProjectilePayload::read_field(
                reader,
                "projectile",
                &projectile_decode(&limits, &base_limits, 1, LegacyElementClass::Stone),
            )
            .unwrap_err();
            assert_eq!(error.offset, 69);
            assert!(error.field.ends_with("trajectory.count"));
            assert!(error.to_string().contains("caller-supplied limit"));
            assert_eq!(reader.offset(), 73);
        });
    }

    #[test]
    fn projectile_fingerprint_error_stops_at_the_signature() {
        let mut bytes = FINGERPRINT_PROJECTILE;
        bytes[4] ^= 0xff;
        let limits = LegacyObjectPayloadLimits::default();
        let base_limits = LegacyPayloadLimits::default();
        with_reader(&bytes, |reader| {
            let error = LegacyProjectilePayload::read_field(
                reader,
                "projectile",
                &projectile_decode(&limits, &base_limits, 1, LegacyElementClass::Arrow),
            )
            .unwrap_err();
            assert_eq!(error.offset, 0);
            assert!(error.field.ends_with("fingerprint"));
            assert!(error.to_string().contains("projectile-element fingerprint"));
            assert_eq!(reader.offset(), 16);
        });
    }

    #[test]
    fn coin_consumes_source_purse_before_projectile_parent() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&42_u32.to_le_bytes());
        let mut bad_projectile_fingerprint = FINGERPRINT_PROJECTILE;
        bad_projectile_fingerprint[0] ^= 0xff;
        bytes.extend_from_slice(&bad_projectile_fingerprint);
        let limits = LegacyObjectPayloadLimits::default();
        let base_limits = LegacyPayloadLimits::default();
        with_reader(&bytes, |reader| {
            let error = LegacyCoinPayload::read_field(
                reader,
                "coin",
                &projectile_decode(&limits, &base_limits, 7, LegacyElementClass::Coin),
            )
            .unwrap_err();
            assert_eq!(error.offset, 4);
            assert!(error.field.ends_with("projectile.fingerprint"));
            assert_eq!(reader.offset(), 20);
        });
    }

    #[test]
    fn purse_leaf_fingerprint_precedes_its_coin_count_and_parent() {
        let mut bytes = FINGERPRINT_PURSE;
        bytes[15] ^= 0xff;
        let limits = LegacyObjectPayloadLimits::default();
        let base_limits = LegacyPayloadLimits::default();
        with_reader(&bytes, |reader| {
            let error = LegacyPursePayload::read_field(
                reader,
                "purse",
                &projectile_decode(&limits, &base_limits, 7, LegacyElementClass::Purse),
            )
            .unwrap_err();
            assert_eq!(error.offset, 0);
            assert!(error.field.ends_with("fingerprint"));
            assert!(error.to_string().contains("purse-element fingerprint"));
            assert_eq!(reader.offset(), 16);
        });
    }

    #[test]
    fn wasp_consumes_three_dimensional_movement_before_object_parent() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FINGERPRINT_WASP);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 12]);
        bytes.extend_from_slice(&[0; 16]);
        let base_limits = LegacyPayloadLimits::default();
        with_reader(&bytes, |reader| {
            let error = LegacyWaspPayload::read_field(
                reader,
                "wasp",
                &object_decode(&base_limits, 1, LegacyElementClass::Wasp),
            )
            .unwrap_err();
            assert_eq!(error.offset, 41);
            assert!(error.field.ends_with("object.fingerprint"));
            assert_eq!(reader.offset(), 57);
        });
    }

    #[test]
    fn net_victim_count_has_a_strict_u16_limit() {
        with_reader(&2_u16.to_le_bytes(), |reader| {
            let error = reader.read_count_u16("victims.count", 1).unwrap_err();
            assert_eq!(error.offset, 0);
            assert_eq!(error.field, "victims.count");
            assert!(error.to_string().contains("caller-supplied limit"));
        });
    }
}
