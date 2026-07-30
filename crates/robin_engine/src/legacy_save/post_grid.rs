//! Decoder for `RHFastFindGrid::Serialize` in Original v48 saves.
//!
//! The stream contains no patch, gate, script-object, or sector counts.
//! `RHFastFindGrid` walks mission-created arrays instead. Decoding therefore
//! requires the same ordered topology produced while loading the mission.
//! Treating bytes as self-describing here would silently shift every later
//! save section when the wrong mission data is supplied.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::elements::LegacyElementClass;
use super::payload_base::{
    LegacyElementRef, LegacyFxPayload, LegacyPayloadLimits, LegacyPoint2, read_element_ref,
};
use super::payload_vm::LegacyVmMemberSection;

const fn hex16(hex: &str) -> [u8; 16] {
    let bytes = hex.as_bytes();
    let mut result = [0; 16];
    let mut index = 0;
    while index < 16 {
        result[index] = (hex_digit(bytes[index * 2]) << 4) | hex_digit(bytes[index * 2 + 1]);
        index += 1;
    }
    result
}

const fn hex_digit(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        _ => panic!("invalid fingerprint hex"),
    }
}

const FINGERPRINT_GRID: [u8; 16] = hex16("109f51840f1e3a0b2ef324915c42722f");
const FINGERPRINT_PATCH: [u8; 16] = hex16("607a13790e707c89e2654c43fa3862db");
const FINGERPRINT_DOOR: [u8; 16] = hex16("ac52c6241393fc1f57eb19891d60378e");
const FINGERPRINT_SCRIPT_SECTOR: [u8; 16] = hex16("977b4f52068b314e5c86f1fe9fb83e2b");
const FINGERPRINT_DOOR_SECTOR: [u8; 16] = hex16("9b6b9e747bf5f510fd1cf51c449132cf");
const FINGERPRINT_BUILDING_SECTOR: [u8; 16] = hex16("b72119432f59b53c935a2e311fd7733d");
const FINGERPRINT_LIFT_SECTOR: [u8; 16] = hex16("2356a100488554c198fa3cc3c745cdcb");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyGridLimits {
    pub occupants_per_container: usize,
    pub static_repulsive_points: usize,
}

impl Default for LegacyGridLimits {
    fn default() -> Self {
        Self {
            occupants_per_container: 65_535,
            static_repulsive_points: 1_000_000,
        }
    }
}

/// Identity of the patch-owned `RHElementFX`, when the mission constructed one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPatchFxTopology {
    pub creation_order: u32,
    pub class: LegacyElementClass,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPatchTopology {
    pub layer: u16,
    pub index_in_layer: u16,
    pub fx: Option<LegacyPatchFxTopology>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyScriptObjectTopology {
    NonSector,
    Sector {
        /// `None` means `mbScriptAssociated == false`, so the Original does
        /// not invoke `VMCore::SerializeMemberVariable`.
        associated_class: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyGateTopology {
    /// `RHDoor::Serialize` writes lock and PC-authorisation state.
    Door,
    /// Plain `RHGate`/`RHGateJump` dispatches to `RHGate::Serialize`, whose
    /// v48 implementation deliberately writes no bytes.
    Stateless,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacySectorTopology {
    NullOrOrdinary,
    Door,
    Building,
    Lift,
}

/// Exact ordered mission topology consumed by `RHFastFindGrid::Serialize`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyGridTopology {
    /// Normal layer order, then patch order within each layer.
    pub patches: Vec<LegacyPatchTopology>,
    /// Full `marrayGates` order, including byte-less jump gates.
    pub gates: Vec<LegacyGateTopology>,
    /// Full `marrayScriptObjects` order, including entries skipped because
    /// `IsSector()` is false.
    pub script_objects: Vec<LegacyScriptObjectTopology>,
    /// Full `marraySectors` order. Only the three special kinds serialize.
    pub sectors: Vec<LegacySectorTopology>,
}

pub trait LegacyGridDecodeContext {
    fn read_sector_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        class_name: &str,
    ) -> LegacyResult<LegacyVmMemberSection>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyFastFindGridState {
    pub start_offset: u64,
    pub abi_profile: LegacySaveAbiProfile,
    pub patches: Vec<LegacyPatchState>,
    pub gates: Vec<LegacyGateState>,
    pub script_sectors: Vec<LegacyScriptSectorState>,
    pub special_sectors: Vec<LegacySpecialSectorState>,
    pub static_repulsive_points: Vec<LegacyLayeredRepulsivePoint>,
    pub end_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPatchState {
    pub topology: LegacyPatchTopology,
    pub active: bool,
    /// Four obsolete lock booleans are skipped by the Original. They are
    /// opaque compiler-era bytes, not authoritative boolean values.
    pub obsolete_lock_bytes: [u8; 4],
    pub locked: bool,
    pub occupants: Vec<LegacyElementRef>,
    pub active_now: bool,
    pub applied_now: bool,
    pub in_transition_now: bool,
    pub fx: Option<LegacyFxPayload>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyDoorState {
    pub locked_pc: bool,
    pub locked_npc_villain: bool,
    pub locked_npc_civilian: bool,
    pub unlockable: bool,
    pub special_authorisation_pc: bool,
    pub authorised_pc_direct: u16,
    pub authorised_pc_indirect: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyGateState {
    Door(LegacyDoorState),
    Stateless,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyScriptSectorState {
    pub script_object_index: usize,
    pub occupants: Vec<LegacyElementRef>,
    pub script_members: Option<LegacyVmMemberSection>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LegacySpecialSectorState {
    Door {
        sector_index: usize,
        active: bool,
    },
    Building {
        sector_index: usize,
        occupants: Vec<LegacyElementRef>,
        arrow_reserve: bool,
    },
    Lift {
        sector_index: usize,
        occupants_pc: u16,
        occupants: u16,
        occupied_upwards: bool,
        occupied_downwards: bool,
        wait_time: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyLayeredRepulsivePoint {
    pub point: LegacyRepulsivePoint,
    pub layer: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyRepulsivePoint {
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

impl LegacyFastFindGridState {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        topology: &LegacyGridTopology,
        limits: &LegacyGridLimits,
        payload_limits: &LegacyPayloadLimits,
        context: &dyn LegacyGridDecodeContext,
    ) -> LegacyResult<Self> {
        reader.scope("fast_find_grid", |reader| {
            validate_topology(reader, topology)?;
            let start_offset = reader.offset();
            reader.read_signature("fingerprint", FINGERPRINT_GRID, "MD5(\"RHFastFindGrid\")")?;

            let mut patches = Vec::new();
            reserve(reader, &mut patches, topology.patches.len(), "patches")?;
            for (index, patch_topology) in topology.patches.iter().enumerate() {
                patches.push(reader.scope(format!("patches[{index}]"), |reader| {
                    read_patch(reader, patch_topology, limits, payload_limits)
                })?);
            }

            let mut gates = Vec::new();
            reserve(reader, &mut gates, topology.gates.len(), "gates")?;
            for (index, topology) in topology.gates.iter().enumerate() {
                gates.push(match topology {
                    LegacyGateTopology::Door => {
                        LegacyGateState::Door(reader.scope(format!("gates[{index}]"), read_door)?)
                    }
                    LegacyGateTopology::Stateless => LegacyGateState::Stateless,
                });
            }

            let mut script_sectors = Vec::new();
            for (index, script_object) in topology.script_objects.iter().enumerate() {
                let LegacyScriptObjectTopology::Sector { associated_class } = script_object else {
                    continue;
                };
                script_sectors.push(reader.scope(
                    format!("script_objects[{index}]"),
                    |reader| {
                        reader.read_signature(
                            "fingerprint",
                            FINGERPRINT_SCRIPT_SECTOR,
                            "MD5(\"RHSectorScript\")",
                        )?;
                        let occupants = read_occupants(reader, limits)?;
                        let script_members = associated_class
                            .as_deref()
                            .map(|class_name| {
                                reader.scope("script_members", |reader| {
                                    context.read_sector_script_members(reader, class_name)
                                })
                            })
                            .transpose()?;
                        Ok(LegacyScriptSectorState {
                            script_object_index: index,
                            occupants,
                            script_members,
                        })
                    },
                )?);
            }

            let mut special_sectors = Vec::new();
            for (index, sector) in topology.sectors.iter().enumerate() {
                let state = match sector {
                    LegacySectorTopology::NullOrOrdinary => continue,
                    LegacySectorTopology::Door => {
                        reader.read_signature(
                            "sector_door.fingerprint",
                            FINGERPRINT_DOOR_SECTOR,
                            "MD5(\"RHSectorDoor\")",
                        )?;
                        LegacySpecialSectorState::Door {
                            sector_index: index,
                            active: reader.read_bool(format!("sectors[{index}].active"))?,
                        }
                    }
                    LegacySectorTopology::Building => {
                        reader.scope(format!("sectors[{index}].building"), |reader| {
                            reader.read_signature(
                                "fingerprint",
                                FINGERPRINT_BUILDING_SECTOR,
                                "MD5(\"RHSectorBuilding\")",
                            )?;
                            Ok(LegacySpecialSectorState::Building {
                                sector_index: index,
                                occupants: read_occupants(reader, limits)?,
                                arrow_reserve: reader.read_bool("arrow_reserve")?,
                            })
                        })?
                    }
                    LegacySectorTopology::Lift => {
                        reader.scope(format!("sectors[{index}].lift"), |reader| {
                            reader.read_signature(
                                "fingerprint",
                                FINGERPRINT_LIFT_SECTOR,
                                "MD5(\"RHSectorLift\")",
                            )?;
                            Ok(LegacySpecialSectorState::Lift {
                                sector_index: index,
                                occupants_pc: reader.read_u16("occupants_pc")?,
                                occupants: reader.read_u16("occupants")?,
                                occupied_upwards: reader.read_bool("occupied_upwards")?,
                                occupied_downwards: reader.read_bool("occupied_downwards")?,
                                wait_time: reader.read_u32("wait_time")?,
                            })
                        })?
                    }
                };
                special_sectors.push(state);
            }

            let point_count = reader.read_count_u32(
                "static_repulsive_points.count",
                limits.static_repulsive_points,
            )?;
            let mut static_repulsive_points = Vec::new();
            reserve(
                reader,
                &mut static_repulsive_points,
                point_count,
                "static_repulsive_points",
            )?;
            for index in 0..point_count {
                static_repulsive_points.push(reader.scope(
                    format!("static_repulsive_points[{index}]"),
                    |reader| {
                        Ok(LegacyLayeredRepulsivePoint {
                            point: read_repulsive_point(reader)?,
                            layer: reader.read_u16("layer")?,
                        })
                    },
                )?);
            }

            let end_offset = reader.offset();
            Ok(Self {
                start_offset,
                abi_profile,
                patches,
                gates,
                script_sectors,
                special_sectors,
                static_repulsive_points,
                end_offset,
            })
        })
    }
}

fn validate_topology(
    reader: &mut LegacyReader<'_>,
    topology: &LegacyGridTopology,
) -> LegacyResult<()> {
    let offset = reader.offset();
    for (index, pair) in topology.patches.windows(2).enumerate() {
        let previous = (&pair[0].layer, &pair[0].index_in_layer);
        let current = (&pair[1].layer, &pair[1].index_in_layer);
        if current <= previous {
            return Err(reader.invalid_value(
                offset,
                format!("topology.patches[{}]", index + 1),
                format_args!("layer={}, index={}", current.0, current.1),
                "strict normal-layer and patch-index order without duplicates",
            ));
        }
    }
    for (index, patch) in topology.patches.iter().enumerate() {
        if let Some(fx) = patch.fx {
            if fx.class != LegacyElementClass::Fx {
                return Err(reader.invalid_value(
                    offset,
                    format!("topology.patches[{index}].fx.class"),
                    format_args!("{:?}", fx.class),
                    "RHElementFX (the concrete type owned by RHPatch)",
                ));
            }
        }
    }
    Ok(())
}

fn read_patch(
    reader: &mut LegacyReader<'_>,
    topology: &LegacyPatchTopology,
    limits: &LegacyGridLimits,
    payload_limits: &LegacyPayloadLimits,
) -> LegacyResult<LegacyPatchState> {
    reader.read_signature("fingerprint", FINGERPRINT_PATCH, "MD5(\"RHPatch\")")?;
    let active = reader.read_bool("active")?;
    let mut obsolete_lock_bytes = [0; 4];
    reader.read_bytes("obsolete_lock_bytes", &mut obsolete_lock_bytes)?;
    let locked = reader.read_bool("locked")?;
    let occupants = read_occupants(reader, limits)?;
    let active_now = reader.read_bool("active_now")?;
    let applied_now = reader.read_bool("applied_now")?;
    let in_transition_now = reader.read_bool("in_transition_now")?;
    let fx = topology
        .fx
        .map(|identity| {
            reader.scope("fx", |reader| {
                LegacyFxPayload::read(
                    reader,
                    payload_limits,
                    Some(identity.creation_order),
                    Some(identity.class),
                )
            })
        })
        .transpose()?;
    Ok(LegacyPatchState {
        topology: topology.clone(),
        active,
        obsolete_lock_bytes,
        locked,
        occupants,
        active_now,
        applied_now,
        in_transition_now,
        fx,
    })
}

fn read_door(reader: &mut LegacyReader<'_>) -> LegacyResult<LegacyDoorState> {
    reader.read_signature("fingerprint", FINGERPRINT_DOOR, "MD5(\"RHDoor\")")?;
    Ok(LegacyDoorState {
        locked_pc: reader.read_bool("locked_pc")?,
        locked_npc_villain: reader.read_bool("locked_npc_villain")?,
        locked_npc_civilian: reader.read_bool("locked_npc_civilian")?,
        unlockable: reader.read_bool("unlockable")?,
        special_authorisation_pc: reader.read_bool("special_authorisation_pc")?,
        authorised_pc_direct: reader.read_u16("authorised_pc_direct")?,
        authorised_pc_indirect: reader.read_u16("authorised_pc_indirect")?,
    })
}

fn read_occupants(
    reader: &mut LegacyReader<'_>,
    limits: &LegacyGridLimits,
) -> LegacyResult<Vec<LegacyElementRef>> {
    let count_offset = reader.offset();
    let count = reader.read_u16("occupants.count")? as usize;
    if count > limits.occupants_per_container {
        return Err(reader.invalid_value(
            count_offset,
            "occupants.count",
            count,
            "occupant count within the caller-supplied limit",
        ));
    }
    let mut occupants = Vec::new();
    reserve(reader, &mut occupants, count, "occupants")?;
    for index in 0..count {
        occupants.push(read_element_ref(reader, format!("occupants[{index}]"))?);
    }
    Ok(occupants)
}

fn read_repulsive_point(reader: &mut LegacyReader<'_>) -> LegacyResult<LegacyRepulsivePoint> {
    Ok(LegacyRepulsivePoint {
        position: read_point2(reader, "position")?,
        concave: reader.read_bool("concave")?,
        limit_left: read_point2(reader, "limit_left")?,
        limit_right: read_point2(reader, "limit_right")?,
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
    use crate::sbfile::{SB_FILE_READ, SbFile};

    struct NoScripts;

    impl LegacyGridDecodeContext for NoScripts {
        fn read_sector_script_members(
            &self,
            reader: &mut LegacyReader<'_>,
            class_name: &str,
        ) -> LegacyResult<LegacyVmMemberSection> {
            let offset = reader.offset();
            Err(reader.invalid_value(
                offset,
                "script_class",
                class_name,
                "a script decoder supplied by the mission context",
            ))
        }
    }

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut temporary = NamedTempFile::new().unwrap();
        temporary.write_all(bytes).unwrap();
        temporary.flush().unwrap();
        let mut file = SbFile::open(temporary.path().to_str().unwrap(), SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    fn empty_topology() -> LegacyGridTopology {
        LegacyGridTopology {
            patches: Vec::new(),
            gates: Vec::new(),
            script_objects: Vec::new(),
            sectors: Vec::new(),
        }
    }

    fn read(bytes: &[u8], topology: &LegacyGridTopology) -> LegacyResult<LegacyFastFindGridState> {
        with_reader(bytes, |reader| {
            LegacyFastFindGridState::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                topology,
                &LegacyGridLimits::default(),
                &LegacyPayloadLimits::default(),
                &NoScripts,
            )
        })
    }

    #[test]
    fn decodes_minimal_topology_for_both_audited_abis_to_exact_boundary() {
        let mut bytes = FINGERPRINT_GRID.to_vec();
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        for abi_profile in [
            LegacySaveAbiProfile::PortLinuxI386V48,
            LegacySaveAbiProfile::RetailWindowsX86V48,
        ] {
            let state = with_reader(&bytes, |reader| {
                LegacyFastFindGridState::read(
                    reader,
                    abi_profile,
                    &empty_topology(),
                    &LegacyGridLimits::default(),
                    &LegacyPayloadLimits::default(),
                    &NoScripts,
                )
            })
            .unwrap();
            assert_eq!(state.abi_profile, abi_profile);
            assert_eq!(state.start_offset, 0);
            assert_eq!(state.end_offset, 20);
            assert!(state.patches.is_empty());
            assert!(state.static_repulsive_points.is_empty());
        }
    }

    #[test]
    fn patch_preserves_opaque_skip_bytes_and_optional_fx_is_topology_driven() {
        let mut bytes = FINGERPRINT_GRID.to_vec();
        bytes.extend_from_slice(&FINGERPRINT_PATCH);
        bytes.extend_from_slice(&[1, 0xde, 0xad, 0xbe, 0xef, 1]);
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&[1, 0, 1]);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        let mut topology = empty_topology();
        topology.patches.push(LegacyPatchTopology {
            layer: 0,
            index_in_layer: 0,
            fx: None,
        });

        let state = read(&bytes, &topology).unwrap();
        assert_eq!(
            state.patches[0].obsolete_lock_bytes,
            [0xde, 0xad, 0xbe, 0xef]
        );
        assert!(state.patches[0].fx.is_none());

        topology.patches[0].fx = Some(LegacyPatchFxTopology {
            creation_order: 7,
            class: LegacyElementClass::Fx,
        });
        let error = read(&bytes, &topology).unwrap_err();
        assert!(error.field.contains("patches[0].fx.fingerprint"));
    }

    #[test]
    fn associated_sector_requires_mission_script_schema_decoder() {
        let mut bytes = FINGERPRINT_GRID.to_vec();
        bytes.extend_from_slice(&FINGERPRINT_SCRIPT_SECTOR);
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        let mut topology = empty_topology();
        topology
            .script_objects
            .push(LegacyScriptObjectTopology::Sector {
                associated_class: Some("AlarmZone".to_owned()),
            });
        let error = read(&bytes, &topology).unwrap_err();
        assert!(error.field.contains("script_members.script_class"));
        assert!(error.to_string().contains("AlarmZone"));
    }

    #[test]
    fn rejects_inconsistent_patch_topology_before_consuming_stream() {
        let mut topology = empty_topology();
        topology.patches = vec![
            LegacyPatchTopology {
                layer: 1,
                index_in_layer: 0,
                fx: None,
            },
            LegacyPatchTopology {
                layer: 0,
                index_in_layer: 4,
                fx: None,
            },
        ];
        let error = read(&[], &topology).unwrap_err();
        assert!(error.field.contains("topology.patches[1]"));
        assert_eq!(error.offset, 0);
    }
}
