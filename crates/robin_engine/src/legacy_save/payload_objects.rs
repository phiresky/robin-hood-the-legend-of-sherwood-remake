//! Original v48 phase-two payloads for objects, projectiles, and mobile scenery.
//!
//! The apparent C++ inheritance hierarchy is not enough to decode these
//! records. Several leaf serializers write state before calling their parent,
//! while `RHElementWasp` deliberately calls `RHElementObject::Serialize`
//! despite inheriting from `RHElementProjectile`. The readers below mirror the
//! exact `Serialize` call order.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::elements::LegacyElementClass;
use super::payload_base::{
    LegacyElementRef, LegacyMobilePayload, LegacyOpaquePointer32, LegacyPayloadDecodeContext,
    LegacyPayloadLimits, LegacyPoint2, LegacyPoint3, LegacySectorRef, read_element_ref,
    read_sector_ref,
};
use super::payload_nonactors::{LegacyObjectPayload, read_object_payload};

const NULL_U32: u32 = u32::MAX;

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
            trajectory_points: 65_535,
            net_victims: 65_535,
        }
    }
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
    limits: &LegacyObjectPayloadLimits,
    base_limits: &LegacyPayloadLimits,
    context: &dyn LegacyPayloadDecodeContext,
    creation_order: u32,
    class: LegacyElementClass,
) -> LegacyResult<LegacyObjectItemPayload> {
    reader.scope(format!("object_item.{class:?}"), |reader| {
        Ok(match class {
            LegacyElementClass::Object => LegacyObjectItemPayload::Object(read_object_payload(
                reader,
                abi_profile,
                base_limits,
                creation_order,
                class,
            )?),
            LegacyElementClass::Arrow => LegacyObjectItemPayload::Arrow(LegacyArrowPayload::read(
                reader,
                abi_profile,
                limits,
                base_limits,
                creation_order,
            )?),
            LegacyElementClass::Apple => LegacyObjectItemPayload::Apple(LegacyApplePayload {
                projectile: LegacyProjectilePayload::read(
                    reader,
                    abi_profile,
                    limits,
                    base_limits,
                    creation_order,
                    class,
                )?,
            }),
            LegacyElementClass::Purse => LegacyObjectItemPayload::Purse(LegacyPursePayload::read(
                reader,
                abi_profile,
                limits,
                base_limits,
                creation_order,
            )?),
            LegacyElementClass::Stone => LegacyObjectItemPayload::Stone(LegacyStonePayload {
                projectile: LegacyProjectilePayload::read(
                    reader,
                    abi_profile,
                    limits,
                    base_limits,
                    creation_order,
                    class,
                )?,
            }),
            LegacyElementClass::WaspNest => {
                LegacyObjectItemPayload::WaspNest(LegacyWaspNestPayload::read(
                    reader,
                    abi_profile,
                    limits,
                    base_limits,
                    creation_order,
                )?)
            }
            LegacyElementClass::Wasp => LegacyObjectItemPayload::Wasp(LegacyWaspPayload::read(
                reader,
                abi_profile,
                base_limits,
                creation_order,
            )?),
            LegacyElementClass::Net => LegacyObjectItemPayload::Net(LegacyNetPayload::read(
                reader,
                abi_profile,
                limits,
                base_limits,
                creation_order,
            )?),
            LegacyElementClass::Coin => LegacyObjectItemPayload::Coin(LegacyCoinPayload::read(
                reader,
                abi_profile,
                limits,
                base_limits,
                creation_order,
            )?),
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
            LegacyElementClass::Mobile => {
                audit_abi(abi_profile);
                LegacyObjectItemPayload::Mobile(LegacyMobilePayload::read(
                    reader,
                    base_limits,
                    context,
                    creation_order,
                )?)
            }
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyProjectilePayload {
    pub flying: bool,
    pub dive: bool,
    pub magic_bullet: bool,
    pub frame_count: u16,
    pub trajectory_origin_map: LegacyPoint2,
    /// Raw Win32/i386 pointer bytes written by `CHECKVAR(pSector)`. They are
    /// not authoritative; `trajectory_origin_sector` follows after the
    /// audited two-byte struct padding.
    pub trajectory_origin_sector_pointer: LegacyOpaquePointer32,
    pub trajectory_origin_level: u16,
    pub trajectory_origin_padding: [u8; 2],
    pub trajectory_origin_sector: LegacySectorRef,
    pub flight_direction: u16,
    pub start: LegacyPoint3,
    pub end: LegacyPoint3,
    pub shooter: LegacyElementRef,
    pub trajectory: Vec<LegacyTrajectoryPoint>,
    pub object: LegacyObjectPayload,
}

impl LegacyProjectilePayload {
    fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyObjectPayloadLimits,
        base_limits: &LegacyPayloadLimits,
        creation_order: u32,
        class: LegacyElementClass,
    ) -> LegacyResult<Self> {
        reader.scope("projectile", |reader| {
            audit_abi(abi_profile);
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_PROJECTILE,
                "RHElementProjectile fingerprint",
            )?;
            let flying = reader.read_bool("flying")?;
            let dive = reader.read_bool("dive")?;
            let magic_bullet = reader.read_bool("magic_bullet")?;
            let frame_count = reader.read_u16("frame_count")?;
            let trajectory_origin_map = read_point2(reader, "trajectory_origin_map")?;
            let trajectory_origin_sector_pointer =
                LegacyOpaquePointer32(reader.read_u32("trajectory_origin_sector_pointer")?);
            let trajectory_origin_level = reader.read_u16("trajectory_origin_level")?;
            let mut trajectory_origin_padding = [0; 2];
            reader.read_bytes("trajectory_origin_padding", &mut trajectory_origin_padding)?;
            let trajectory_origin_sector = read_sector_ref(reader, "trajectory_origin_sector")?;
            let flight_direction = reader.read_u16("flight_direction")?;
            let start = read_point3(reader, "start")?;
            let end = read_point3(reader, "end")?;
            let shooter = read_element_ref(reader, "shooter")?;
            let count = reader.read_count_u32("trajectory.count", limits.trajectory_points)?;
            let mut trajectory = Vec::new();
            reserve(reader, &mut trajectory, count, "trajectory")?;
            for index in 0..count {
                trajectory.push(reader.scope(format!("trajectory[{index}]"), |reader| {
                    Ok(LegacyTrajectoryPoint {
                        time: reader.read_u16("time")?,
                        bounce: reader.read_bool("bounce")?,
                        material: reader.read_u32("material")?,
                        position: read_point3(reader, "position")?,
                    })
                })?);
            }
            let object =
                read_object_payload(reader, abi_profile, base_limits, creation_order, class)?;
            Ok(Self {
                flying,
                dive,
                magic_bullet,
                frame_count,
                trajectory_origin_map,
                trajectory_origin_sector_pointer,
                trajectory_origin_level,
                trajectory_origin_padding,
                trajectory_origin_sector,
                flight_direction,
                start,
                end,
                shooter,
                trajectory,
                object,
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyTrajectoryPoint {
    pub time: u16,
    pub bounce: bool,
    pub material: u32,
    pub position: LegacyPoint3,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyArrowPayload {
    pub projectile: LegacyProjectilePayload,
    pub bow: Option<LegacyBowPayload>,
    pub flat_shot: bool,
    pub falling: bool,
    pub falling_direction: u8,
    pub last_sector: u8,
    pub last_azimuth: i16,
    pub play_impact: bool,
}

impl LegacyArrowPayload {
    fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyObjectPayloadLimits,
        base_limits: &LegacyPayloadLimits,
        creation_order: u32,
    ) -> LegacyResult<Self> {
        reader.scope("arrow", |reader| {
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_ARROW,
                "RHElementArrow fingerprint",
            )?;
            let projectile = LegacyProjectilePayload::read(
                reader,
                abi_profile,
                limits,
                base_limits,
                creation_order,
                LegacyElementClass::Arrow,
            )?;
            let has_bow = reader.read_bool("has_bow")?;
            let bow = if has_bow {
                let raw = reader.read_u32("bow_profile")?;
                Some(LegacyBowPayload {
                    profile: (raw != NULL_U32).then_some(raw),
                })
            } else {
                None
            };
            Ok(Self {
                projectile,
                bow,
                flat_shot: reader.read_bool("flat_shot")?,
                falling: reader.read_bool("falling")?,
                falling_direction: reader.read_u8("falling_direction")?,
                last_sector: reader.read_u8("last_sector")?,
                last_azimuth: reader.read_i16("last_azimuth")?,
                play_impact: reader.read_bool("play_impact")?,
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyBowPayload {
    /// Index in `RHProfileManager::mvectorShootProfile`, or null for a
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPursePayload {
    pub number_of_coins: u16,
    pub projectile: LegacyProjectilePayload,
}

impl LegacyPursePayload {
    fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyObjectPayloadLimits,
        base_limits: &LegacyPayloadLimits,
        creation_order: u32,
    ) -> LegacyResult<Self> {
        reader.scope("purse", |reader| {
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_PURSE,
                "RHElementPurse fingerprint",
            )?;
            let number_of_coins = reader.read_u16("number_of_coins")?;
            let projectile = LegacyProjectilePayload::read(
                reader,
                abi_profile,
                limits,
                base_limits,
                creation_order,
                LegacyElementClass::Purse,
            )?;
            Ok(Self {
                number_of_coins,
                projectile,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyWaspNestPayload {
    pub projectile: LegacyProjectilePayload,
    pub flying_wasp_count: u32,
}

impl LegacyWaspNestPayload {
    fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyObjectPayloadLimits,
        base_limits: &LegacyPayloadLimits,
        creation_order: u32,
    ) -> LegacyResult<Self> {
        reader.scope("wasp_nest", |reader| {
            // RHElementWaspNest::Serialize has no ValidateStream call.
            let projectile = LegacyProjectilePayload::read(
                reader,
                abi_profile,
                limits,
                base_limits,
                creation_order,
                LegacyElementClass::WaspNest,
            )?;
            let flying_wasp_count = reader.read_u32("flying_wasp_count")?;
            Ok(Self {
                projectile,
                flying_wasp_count,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyWaspPayload {
    pub nest: LegacyElementRef,
    pub victim: LegacyElementRef,
    pub stinging: bool,
    pub timeout: u32,
    pub movement: LegacyPoint3,
    pub object: LegacyObjectPayload,
}

impl LegacyWaspPayload {
    fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        base_limits: &LegacyPayloadLimits,
        creation_order: u32,
    ) -> LegacyResult<Self> {
        reader.scope("wasp", |reader| {
            reader.read_signature("fingerprint", FINGERPRINT_WASP, "RHElementWasp fingerprint")?;
            let nest = read_element_ref(reader, "nest")?;
            let victim = read_element_ref(reader, "victim")?;
            let stinging = reader.read_bool("stinging")?;
            let timeout = reader.read_u32("timeout")?;
            let movement = read_point3(reader, "movement")?;
            // Intentional Original behavior: wasps inherit Projectile but
            // serialize only their Object base.
            let object = read_object_payload(
                reader,
                abi_profile,
                base_limits,
                creation_order,
                LegacyElementClass::Wasp,
            )?;
            Ok(Self {
                nest,
                victim,
                stinging,
                timeout,
                movement,
                object,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyNetPayload {
    pub projectile: LegacyProjectilePayload,
    pub victims: Vec<LegacyElementRef>,
    pub time_until_unfolding: u32,
    pub crumpled: bool,
    pub was_flying: bool,
}

impl LegacyNetPayload {
    fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyObjectPayloadLimits,
        base_limits: &LegacyPayloadLimits,
        creation_order: u32,
    ) -> LegacyResult<Self> {
        reader.scope("net", |reader| {
            // RHElementNet::Serialize has no ValidateStream call.
            let projectile = LegacyProjectilePayload::read(
                reader,
                abi_profile,
                limits,
                base_limits,
                creation_order,
                LegacyElementClass::Net,
            )?;
            let count = read_bounded_u16(reader, "victims.count", limits.net_victims)?;
            let mut victims = Vec::new();
            reserve(reader, &mut victims, count, "victims")?;
            for index in 0..count {
                victims.push(read_element_ref(reader, format!("victims[{index}]"))?);
            }
            Ok(Self {
                projectile,
                victims,
                time_until_unfolding: reader.read_u32("time_until_unfolding")?,
                crumpled: reader.read_bool("crumpled")?,
                was_flying: reader.read_bool("was_flying")?,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyCoinPayload {
    pub source_purse: LegacyElementRef,
    pub projectile: LegacyProjectilePayload,
}

impl LegacyCoinPayload {
    fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyObjectPayloadLimits,
        base_limits: &LegacyPayloadLimits,
        creation_order: u32,
    ) -> LegacyResult<Self> {
        reader.scope("coin", |reader| {
            // RHElementCoin::Serialize has no ValidateStream call and stores
            // its leaf reference before invoking Projectile.
            let source_purse = read_element_ref(reader, "source_purse")?;
            let projectile = LegacyProjectilePayload::read(
                reader,
                abi_profile,
                limits,
                base_limits,
                creation_order,
                LegacyElementClass::Coin,
            )?;
            Ok(Self {
                source_purse,
                projectile,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyAlePayload {
    pub object: LegacyObjectPayload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySpyCapePayload {
    pub object: LegacyObjectPayload,
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

fn read_bounded_u16(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display + Copy,
    maximum: usize,
) -> LegacyResult<usize> {
    let offset = reader.offset();
    let raw = reader.read_u16(field)?;
    let count = usize::from(raw);
    if count > maximum {
        return Err(reader.invalid_value(
            offset,
            field,
            count,
            "item count within the caller-supplied limit",
        ));
    }
    Ok(count)
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

fn audit_abi(abi_profile: LegacySaveAbiProfile) {
    debug_assert!(abi_profile.is_little_endian());
    debug_assert_eq!(LegacySaveAbiProfile::BOOL_WIDTH, 1);
    debug_assert_eq!(LegacySaveAbiProfile::WORD_WIDTH, 2);
    debug_assert_eq!(LegacySaveAbiProfile::LONG_WIDTH, 4);
    debug_assert_eq!(LegacySaveAbiProfile::ENUM_WIDTH, 4);
    debug_assert_eq!(LegacySaveAbiProfile::FLOAT_WIDTH, 4);
    debug_assert_eq!(LegacySaveAbiProfile::POINTER_PLACEHOLDER_WIDTH, 4);
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::sbfile::{SB_FILE_READ, SbFile};

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut fixture = NamedTempFile::new().unwrap();
        fixture.write_all(bytes).unwrap();
        fixture.flush().unwrap();
        let path = fixture.path().to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
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
        with_reader(&bytes, |reader| {
            let error = LegacyProjectilePayload::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &limits,
                &LegacyPayloadLimits::default(),
                1,
                LegacyElementClass::Stone,
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
        with_reader(&bytes, |reader| {
            let error = LegacyProjectilePayload::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacyObjectPayloadLimits::default(),
                &LegacyPayloadLimits::default(),
                1,
                LegacyElementClass::Arrow,
            )
            .unwrap_err();
            assert_eq!(error.offset, 0);
            assert!(error.field.ends_with("fingerprint"));
            assert!(
                error
                    .to_string()
                    .contains("RHElementProjectile fingerprint")
            );
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
        with_reader(&bytes, |reader| {
            let error = LegacyCoinPayload::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacyObjectPayloadLimits::default(),
                &LegacyPayloadLimits::default(),
                7,
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
        with_reader(&bytes, |reader| {
            let error = LegacyPursePayload::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacyObjectPayloadLimits::default(),
                &LegacyPayloadLimits::default(),
                7,
            )
            .unwrap_err();
            assert_eq!(error.offset, 0);
            assert!(error.field.ends_with("fingerprint"));
            assert!(error.to_string().contains("RHElementPurse fingerprint"));
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
        with_reader(&bytes, |reader| {
            let error = LegacyWaspPayload::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacyPayloadLimits::default(),
                1,
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
            let error = read_bounded_u16(reader, "victims.count", 1).unwrap_err();
            assert_eq!(error.offset, 0);
            assert_eq!(error.field, "victims.count");
            assert!(error.to_string().contains("caller-supplied limit"));
        });
    }
}
