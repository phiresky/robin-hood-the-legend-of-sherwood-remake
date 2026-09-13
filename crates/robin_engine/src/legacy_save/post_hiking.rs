//! Original v48 post-grid hiking-guide and trajectory sections.
//!
//! The original game writes no hiking path or waypoint counts. It walks the
//! mission-created paths, validates every waypoint, and conditionally
//! serializes VM members for script waypoints. Consequently this grammar
//! requires the exact mission waypoint topology and compiled script schema.
//!
//! The following projectile element is an engine-owned helper, not an
//! element from the phase-one envelope. Historical Original builds left its
//! inherited class ID uninitialized. Its reader therefore preserves the raw
//! ID and deliberately does not use normal class dispatch. Field declaration
//! order is wire order.

use super::read_helpers::DEFAULT_BULK_LIMIT;
use super::read_helpers::{hex16, reserve};
use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyRead, LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::payload_base::{
    LegacyElementRef, LegacyOpaquePointer32, LegacyPayloadLimits, LegacyPoint2, LegacyPoint3,
    LegacySectorRef, LegacySpriteDecode, LegacySpritePayload, read_element_ref,
};
use super::payload_nonactors::LegacyRepulsivePointPayload;
use super::payload_objects::LegacyTrajectoryPoint;
use super::payload_vm::{LegacyVmMemberDecoder, LegacyVmMemberSection};

const FINGERPRINT_HIKING_GUIDE: [u8; 16] = hex16("9999481162bbcc8461c8d924d0ec24cb");
const FINGERPRINT_WAYPOINT: [u8; 16] = hex16("fdf47609dbe10dab3ebc801e5ca5286a");
const FINGERPRINT_PROJECTILE: [u8; 16] = hex16("6933dbbc6be435f3249e7cf1c994fb2d");
const FINGERPRINT_OBJECT: [u8; 16] = hex16("90062155c12beef1e93d3c32cb21776f");
const FINGERPRINT_ELEMENT: [u8; 16] = hex16("7730a5b25924f7a72c4926ef69f7700f");
// Sprite and position-interface signatures are verified by the shared
// payload_base readers; the byte fixtures below still need them.
#[cfg(test)]
const FINGERPRINT_SPRITE: [u8; 16] = hex16("ef8f9051c70a8eb993b6101ac4210ca5");
#[cfg(test)]
const FINGERPRINT_POSITION: [u8; 16] = hex16("f41fe85b168584aa52b8bb352f8b593a");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPostHikingLimits {
    pub hiking_paths: usize,
    pub hiking_waypoints: usize,
    pub trajectory_points: usize,
    pub sprite_animation_replacements: usize,
}

impl Default for LegacyPostHikingLimits {
    fn default() -> Self {
        Self {
            hiking_paths: DEFAULT_BULK_LIMIT,
            hiking_waypoints: DEFAULT_BULK_LIMIT,
            trajectory_points: DEFAULT_BULK_LIMIT,
            sprite_animation_replacements: LegacyPayloadLimits::default()
                .sprite_animation_replacements,
        }
    }
}

/// Mission-authored shape of the hiking-path collection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyHikingGuideTopology {
    pub paths: Vec<LegacyHikingPathTopology>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyHikingPathTopology {
    pub waypoints: Vec<LegacyWaypointTopology>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyWaypointTopology {
    /// `Some` exactly when both global script serialization is enabled and
    /// the mission waypoint is marked as a script command.
    pub script_class: Option<String>,
}

pub trait LegacyHikingGuideDecodeContext {
    fn read_waypoint_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        path_index: usize,
        waypoint_index: usize,
        script_class: &str,
    ) -> LegacyResult<LegacyVmMemberSection>;
}

impl LegacyHikingGuideDecodeContext for LegacyVmMemberDecoder<'_> {
    fn read_waypoint_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        _path_index: usize,
        _waypoint_index: usize,
        script_class: &str,
    ) -> LegacyResult<LegacyVmMemberSection> {
        self.read_class_members(reader, script_class)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyHikingGuideState {
    pub start_offset: u64,
    pub paths: Vec<LegacyHikingPathState>,
    pub end_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyHikingPathState {
    pub waypoints: Vec<LegacyWaypointState>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyWaypointState {
    pub start_offset: u64,
    pub script_members: Option<LegacyVmMemberSection>,
    pub end_offset: u64,
}

// Hand-written: the shape comes from mission topology and script members are
// decoded through the mission context.
impl LegacyHikingGuideState {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        topology: &LegacyHikingGuideTopology,
        limits: &LegacyPostHikingLimits,
        context: &dyn LegacyHikingGuideDecodeContext,
    ) -> LegacyResult<Self> {
        reader.scope("hiking_guide", |reader| {
            let start_offset = reader.offset();
            validate_hiking_topology(reader, topology, limits)?;
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_HIKING_GUIDE,
                "hiking-guide fingerprint",
            )?;

            let mut paths = Vec::new();
            reserve(reader, &mut paths, topology.paths.len(), "paths")?;
            for (path_index, path) in topology.paths.iter().enumerate() {
                paths.push(reader.scope_indexed("paths", path_index, |reader| {
                    let mut waypoints = Vec::new();
                    reserve(reader, &mut waypoints, path.waypoints.len(), "waypoints")?;
                    for (waypoint_index, waypoint) in path.waypoints.iter().enumerate() {
                        waypoints.push(reader.scope_indexed(
                            "waypoints",
                            waypoint_index,
                            |reader| {
                                let start_offset = reader.offset();
                                reader.read_signature(
                                    "fingerprint",
                                    FINGERPRINT_WAYPOINT,
                                    "waypoint fingerprint",
                                )?;
                                let script_members =
                                    if let Some(script_class) = &waypoint.script_class {
                                        Some(reader.scope("script_members", |reader| {
                                            context.read_waypoint_script_members(
                                                reader,
                                                path_index,
                                                waypoint_index,
                                                script_class,
                                            )
                                        })?)
                                    } else {
                                        None
                                    };
                                Ok(LegacyWaypointState {
                                    start_offset,
                                    script_members,
                                    end_offset: reader.offset(),
                                })
                            },
                        )?);
                    }
                    Ok(LegacyHikingPathState { waypoints })
                })?);
            }

            Ok(Self {
                start_offset,
                paths,
                end_offset: reader.offset(),
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyProjectileTrajectorySection {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub projectile: LegacyStandaloneProjectilePayload,
    pub jumper: LegacyElementRef,
    pub jumped: LegacyElementRef,
    pub end_offset: u64,
}

impl LegacyProjectileTrajectorySection {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyPostHikingLimits,
    ) -> LegacyResult<Self> {
        reader.scope("projectile_trajectory", |reader| {
            Ok(Self {
                abi_profile,
                start_offset: reader.offset(),
                projectile: LegacyStandaloneProjectilePayload::read_field(
                    reader,
                    "projectile",
                    limits,
                )?,
                jumper: read_element_ref(reader, "jumper")?,
                jumped: read_element_ref(reader, "jumped")?,
                end_offset: reader.offset(),
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyPostHikingLimits,
    fingerprint = FINGERPRINT_PROJECTILE,
    expected = "projectile-element fingerprint"
)]
pub struct LegacyStandaloneProjectilePayload {
    pub flying: bool,
    pub dive: bool,
    pub magic_bullet: bool,
    pub frame_count: u16,
    pub trajectory_origin_map: LegacyPoint2,
    pub trajectory_origin_sector_pointer: LegacyOpaquePointer32,
    pub trajectory_origin_level: u16,
    #[legacy(bytes)]
    pub trajectory_origin_padding: [u8; 2],
    pub trajectory_origin_sector: LegacySectorRef,
    pub flight_direction: u16,
    pub start: LegacyPoint3,
    pub end: LegacyPoint3,
    pub shooter: LegacyElementRef,
    #[legacy(count_u32 = ctx.trajectory_points)]
    pub trajectory: Vec<LegacyTrajectoryPoint>,
    pub object: LegacyStandaloneObjectPayload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyPostHikingLimits)]
pub struct LegacyStandaloneObjectPayload {
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(fingerprint = FINGERPRINT_OBJECT, expected = "object-element fingerprint")]
    pub terminate: bool,
    pub register_number: u16,
    pub quantity: u16,
    pub animation: u32,
    pub object_type: u32,
    pub associated_action: u32,
    // This standalone trajectory helper is still serialized by
    // object element, whose retail stream uses narrow geometry.
    #[legacy(read = LegacyRepulsivePointPayload::read_field(
        reader,
        "repulsive_point",
        &LegacySaveAbiProfile::PortLinuxI386V48,
    ))]
    pub repulsive_point: LegacyRepulsivePointPayload,
    pub belongs_to_beggar: bool,
    pub taken: bool,
    pub element: LegacyStandaloneElementPayload,
    #[legacy(offset)]
    pub end_offset: u64,
}

/// Inherited element data for the engine-owned helper.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyPostHikingLimits,
    fingerprint = FINGERPRINT_ELEMENT,
    expected = "element fingerprint"
)]
pub struct LegacyStandaloneElementPayload {
    pub creation_order: u32,
    pub outline_colors: [u16; 5],
    pub current_outline: u32,
    pub outline_width: u16,
    pub custom_minimap_dot: u16,
    pub active: bool,
    pub position_map_delayed: bool,
    pub position_delayed: bool,
    /// Opaque because older original-game initialization did not initialize it.
    #[legacy(name = "class_id")]
    pub raw_class_id: u16,
    pub delayed_map_position: LegacyPoint2,
    pub delayed_position: LegacyPoint3,
    pub in_honolulu: bool,
    pub index_in_elements_list: u16,
    pub blipped: bool,
    pub unreachable: bool,
    #[legacy(read = LegacySpritePayload::read_field(
        reader,
        "sprite",
        &LegacySpriteDecode {
            animation_replacements: ctx.sprite_animation_replacements,
            sprite_fingerprint: "sprite fingerprint",
            position_fingerprint: "position-interface fingerprint",
        },
    ))]
    pub sprite: LegacySpritePayload,
}

fn validate_hiking_topology(
    reader: &mut LegacyReader<'_>,
    topology: &LegacyHikingGuideTopology,
    limits: &LegacyPostHikingLimits,
) -> LegacyResult<()> {
    let offset = reader.offset();
    if topology.paths.len() > limits.hiking_paths {
        return Err(reader.invalid_value(
            offset,
            "topology.paths",
            topology.paths.len(),
            "path count within the caller-supplied limit",
        ));
    }
    let mut waypoint_count = 0_usize;
    for (path_index, path) in topology.paths.iter().enumerate() {
        waypoint_count = waypoint_count
            .checked_add(path.waypoints.len())
            .ok_or_else(|| {
                reader.invalid_value(
                    offset,
                    format!("topology.paths[{path_index}].waypoints"),
                    "usize overflow",
                    "total waypoint count representable by the host",
                )
            })?;
        if waypoint_count > limits.hiking_waypoints {
            return Err(reader.invalid_value(
                offset,
                format!("topology.paths[{path_index}].waypoints"),
                waypoint_count,
                "total waypoint count within the caller-supplied limit",
            ));
        }
        for (waypoint_index, waypoint) in path.waypoints.iter().enumerate() {
            if waypoint.script_class.as_deref() == Some("") {
                return Err(reader.invalid_value(
                    offset,
                    format!(
                        "topology.paths[{path_index}].waypoints[{waypoint_index}].script_class"
                    ),
                    "empty string",
                    "non-empty compiled VM class name",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::legacy_save::test_support::{push_f32, push_u16, push_u32};

    use super::*;

    struct NoScripts;

    impl LegacyHikingGuideDecodeContext for NoScripts {
        fn read_waypoint_script_members(
            &self,
            reader: &mut LegacyReader<'_>,
            _path_index: usize,
            _waypoint_index: usize,
            script_class: &str,
        ) -> LegacyResult<LegacyVmMemberSection> {
            let offset = reader.offset();
            Err(reader.invalid_value(
                offset,
                "script_class",
                script_class,
                "no scripted waypoint in this test topology",
            ))
        }
    }

    use crate::legacy_save::test_support::with_reader;

    fn empty_topology() -> LegacyHikingGuideTopology {
        LegacyHikingGuideTopology { paths: Vec::new() }
    }

    fn push_point2(bytes: &mut Vec<u8>) {
        push_f32(bytes, 0.0);
        push_f32(bytes, 0.0);
    }

    fn push_point3(bytes: &mut Vec<u8>) {
        push_point2(bytes);
        push_f32(bytes, 0.0);
    }

    fn push_box2(bytes: &mut Vec<u8>) {
        push_point2(bytes);
        push_point2(bytes);
        bytes.push(0);
    }

    fn push_position(bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&FINGERPRINT_POSITION);
        for _ in 0..5 {
            push_u32(bytes, 0);
        }
        push_u16(bytes, 0);
        push_u16(bytes, 0);
        bytes.push(0);
        push_u16(bytes, 0);
        push_u16(bytes, 0);
        push_f32(bytes, 0.0);
        bytes.extend_from_slice(&[0; 5]);
        bytes.push(0);
        bytes.extend_from_slice(&[0; 2]);
        push_u16(bytes, 0);
        push_f32(bytes, 0.0);
        bytes.push(0);
        push_u16(bytes, u16::MAX);
        push_u16(bytes, u16::MAX);
        push_u16(bytes, u16::MAX);
        push_u16(bytes, u16::MAX);
        push_u32(bytes, u32::MAX);
        push_point3(bytes);
        push_point2(bytes);
        push_point2(bytes);
        push_point3(bytes);
        push_point2(bytes);
        push_point2(bytes);
        push_point2(bytes);
        push_point2(bytes);
        push_point3(bytes);
        push_point3(bytes);
        push_point2(bytes);
        push_point2(bytes);
        push_point3(bytes);
        push_box2(bytes);
        push_box2(bytes);
    }

    fn push_sprite(bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&FINGERPRINT_SPRITE);
        for _ in 0..5 {
            push_u16(bytes, 0);
        }
        push_u32(bytes, 0);
        bytes.extend_from_slice(&[0; 3]);
        push_f32(bytes, 0.0);
        push_u32(bytes, 0);
        bytes.push(0);
        push_u32(bytes, u32::MAX);
        for _ in 0..4 {
            push_u16(bytes, 0);
        }
        push_u32(bytes, 0);
        push_box2(bytes);
        push_u32(bytes, 0);
        push_position(bytes);
    }

    fn push_element(bytes: &mut Vec<u8>, raw_class_id: u16) {
        bytes.extend_from_slice(&FINGERPRINT_ELEMENT);
        push_u32(bytes, 23);
        for _ in 0..5 {
            push_u16(bytes, 0);
        }
        push_u32(bytes, 0);
        push_u16(bytes, 0);
        push_u16(bytes, 0);
        bytes.extend_from_slice(&[0; 3]);
        push_u16(bytes, raw_class_id);
        push_point2(bytes);
        push_point3(bytes);
        bytes.push(0);
        push_u16(bytes, 0);
        bytes.extend_from_slice(&[0; 2]);
        push_sprite(bytes);
    }

    fn push_repulsive_point(bytes: &mut Vec<u8>, _abi_profile: LegacySaveAbiProfile) {
        // The standalone trajectory helper is serialized as an object element,
        // whose retail stream keeps the ordinary narrow geometry — both
        // audited producer ABIs write the 8-byte point form here.
        let point_width = 8;
        bytes.resize(bytes.len() + point_width, 0);
        bytes.push(0);
        bytes.resize(bytes.len() + point_width * 2, 0);
        for _ in 0..4 {
            push_f32(bytes, 0.0);
        }
        push_u32(bytes, 0);
        bytes.extend_from_slice(&[0; 4]);
    }

    fn minimal_trajectory(abi_profile: LegacySaveAbiProfile, raw_class_id: u16) -> Vec<u8> {
        let mut bytes = FINGERPRINT_PROJECTILE.to_vec();
        bytes.extend_from_slice(&[0; 3]);
        push_u16(&mut bytes, 0);
        push_point2(&mut bytes);
        push_u32(&mut bytes, 0x1234_5678);
        push_u16(&mut bytes, 0);
        bytes.extend_from_slice(&[0xaa, 0x55]);
        push_u16(&mut bytes, u16::MAX);
        push_u16(&mut bytes, 0);
        push_point3(&mut bytes);
        push_point3(&mut bytes);
        push_u32(&mut bytes, u32::MAX);
        push_u32(&mut bytes, 0);

        bytes.extend_from_slice(&FINGERPRINT_OBJECT);
        bytes.push(0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_repulsive_point(&mut bytes, abi_profile);
        bytes.extend_from_slice(&[0; 2]);
        push_element(&mut bytes, raw_class_id);

        push_u32(&mut bytes, 12);
        push_u32(&mut bytes, u32::MAX);
        bytes
    }

    #[test]
    fn hiking_topology_drives_exact_waypoint_count() {
        let topology = LegacyHikingGuideTopology {
            paths: vec![
                LegacyHikingPathTopology {
                    waypoints: vec![LegacyWaypointTopology { script_class: None }],
                },
                LegacyHikingPathTopology {
                    waypoints: vec![
                        LegacyWaypointTopology { script_class: None },
                        LegacyWaypointTopology { script_class: None },
                    ],
                },
            ],
        };
        let mut bytes = FINGERPRINT_HIKING_GUIDE.to_vec();
        for _ in 0..3 {
            bytes.extend_from_slice(&FINGERPRINT_WAYPOINT);
        }
        bytes.extend_from_slice(&0xfeed_beef_u32.to_le_bytes());

        let state = with_reader(&bytes, |reader| {
            let state = LegacyHikingGuideState::read(
                reader,
                &topology,
                &LegacyPostHikingLimits::default(),
                &NoScripts,
            )
            .unwrap();
            assert_eq!(reader.offset(), 64);
            state
        });
        assert_eq!(state.start_offset, 0);
        assert_eq!(state.end_offset, 64);
        assert_eq!(state.paths[1].waypoints.len(), 2);
    }

    #[test]
    fn empty_hiking_guide_is_abi_independent() {
        for _abi_profile in [
            LegacySaveAbiProfile::PortLinuxI386V48,
            LegacySaveAbiProfile::RetailWindowsX86V48,
        ] {
            let state = with_reader(&FINGERPRINT_HIKING_GUIDE, |reader| {
                LegacyHikingGuideState::read(
                    reader,
                    &empty_topology(),
                    &LegacyPostHikingLimits::default(),
                    &NoScripts,
                )
            })
            .unwrap();
            assert_eq!(state.end_offset, 16);
        }
    }

    #[test]
    fn standalone_trajectory_decodes_both_abis_and_preserves_opaque_class() {
        for abi_profile in [
            LegacySaveAbiProfile::PortLinuxI386V48,
            LegacySaveAbiProfile::RetailWindowsX86V48,
        ] {
            let bytes = minimal_trajectory(abi_profile, 0xdead);
            let state = with_reader(&bytes, |reader| {
                LegacyProjectileTrajectorySection::read(
                    reader,
                    abi_profile,
                    &LegacyPostHikingLimits::default(),
                )
            })
            .unwrap();
            assert_eq!(state.end_offset as usize, bytes.len());
            assert_eq!(state.projectile.object.element.raw_class_id, 0xdead);
            assert_eq!(state.projectile.object.element.creation_order, 23);
            assert_eq!(state.jumper, LegacyElementRef(Some(12)));
            assert_eq!(state.jumped, LegacyElementRef(None));
        }
    }

    #[test]
    fn malformed_waypoint_fingerprint_stops_at_exact_signature() {
        let topology = LegacyHikingGuideTopology {
            paths: vec![LegacyHikingPathTopology {
                waypoints: vec![LegacyWaypointTopology { script_class: None }],
            }],
        };
        let mut bytes = FINGERPRINT_HIKING_GUIDE.to_vec();
        let mut bad = FINGERPRINT_WAYPOINT;
        bad[3] ^= 0xff;
        bytes.extend_from_slice(&bad);
        with_reader(&bytes, |reader| {
            let error = LegacyHikingGuideState::read(
                reader,
                &topology,
                &LegacyPostHikingLimits::default(),
                &NoScripts,
            )
            .unwrap_err();
            assert_eq!(error.offset, 16);
            assert!(error.field.ends_with("waypoints[0].fingerprint"));
            assert_eq!(reader.offset(), 32);
        });
    }

    #[test]
    fn trajectory_count_limit_fails_before_allocation_for_both_abis() {
        for abi_profile in [
            LegacySaveAbiProfile::PortLinuxI386V48,
            LegacySaveAbiProfile::RetailWindowsX86V48,
        ] {
            let mut bytes = FINGERPRINT_PROJECTILE.to_vec();
            bytes.extend_from_slice(&[0; 3]);
            push_u16(&mut bytes, 0);
            push_point2(&mut bytes);
            push_u32(&mut bytes, 0);
            push_u16(&mut bytes, 0);
            bytes.extend_from_slice(&[0; 2]);
            push_u16(&mut bytes, u16::MAX);
            push_u16(&mut bytes, 0);
            push_point3(&mut bytes);
            push_point3(&mut bytes);
            push_u32(&mut bytes, u32::MAX);
            push_u32(&mut bytes, 2);
            let limits = LegacyPostHikingLimits {
                trajectory_points: 1,
                ..LegacyPostHikingLimits::default()
            };

            with_reader(&bytes, |reader| {
                let error = LegacyProjectileTrajectorySection::read(reader, abi_profile, &limits)
                    .unwrap_err();
                assert_eq!(error.offset, 69);
                assert!(error.field.ends_with("trajectory.count"));
                assert_eq!(reader.offset(), 73);
            });
        }
    }

    #[test]
    fn topology_limit_rejection_does_not_consume_stream() {
        let topology = LegacyHikingGuideTopology {
            paths: vec![LegacyHikingPathTopology {
                waypoints: vec![LegacyWaypointTopology { script_class: None }],
            }],
        };
        let limits = LegacyPostHikingLimits {
            hiking_waypoints: 0,
            ..LegacyPostHikingLimits::default()
        };
        with_reader(&[], |reader| {
            let error =
                LegacyHikingGuideState::read(reader, &topology, &limits, &NoScripts).unwrap_err();
            assert!(error.field.contains("topology.paths[0].waypoints"));
            assert_eq!(error.offset, 0);
            assert_eq!(reader.offset(), 0);
        });
    }
}
