//! Decoder for fast-grid data in original-game v48 saves.
//!
//! The stream contains no patch, gate, script-object, or sector counts.
//! The spatial grid walks mission-created arrays instead. Decoding therefore
//! requires the same ordered topology produced while loading the mission.
//! Treating bytes as self-describing here would silently shift every later
//! save section when the wrong mission data is supplied.

use super::read_helpers::DEFAULT_BULK_LIMIT;
use super::read_helpers::{hex16, reserve};
use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyRead, LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::elements::LegacyElementClass;
use super::payload_base::{
    LegacyElementBaseDecode, LegacyElementRef, LegacyFxPayload, LegacyPayloadLimits, LegacyPoint2,
};
use super::payload_vm::{LegacyVmMemberDecoder, LegacyVmMemberSection};

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
            occupants_per_container: DEFAULT_BULK_LIMIT,
            static_repulsive_points: 1_000_000,
        }
    }
}

/// Identity of the patch-owned effect element, when the mission constructed one.
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
        /// `None` means no associated script, so the original game does
        /// not serialize member variables.
        associated_class: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyGateTopology {
    /// Door serialization writes lock and PC-authorisation state.
    Door,
    /// Plain and jump gates use base gate serialization, whose
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

/// Exact ordered mission topology consumed by fast-grid serialization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyGridTopology {
    /// Normal layer order, then patch order within each layer.
    pub patches: Vec<LegacyPatchTopology>,
    /// Full gate order, including byte-less jump gates.
    pub gates: Vec<LegacyGateTopology>,
    /// Full script-object order, including entries skipped because
    /// they are not sectors.
    pub script_objects: Vec<LegacyScriptObjectTopology>,
    /// Full sector order. Only the three special kinds serialize.
    pub sectors: Vec<LegacySectorTopology>,
}

pub trait LegacyGridDecodeContext {
    fn read_sector_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        class_name: &str,
    ) -> LegacyResult<LegacyVmMemberSection>;
}

impl LegacyGridDecodeContext for LegacyVmMemberDecoder<'_> {
    fn read_sector_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        class_name: &str,
    ) -> LegacyResult<LegacyVmMemberSection> {
        self.read_class_members(reader, class_name)
    }
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

/// Decode context for one patch: its mission topology entry and limits.
#[derive(Clone, Copy)]
pub struct LegacyPatchDecode<'a> {
    pub topology: &'a LegacyPatchTopology,
    pub limits: &'a LegacyGridLimits,
    pub payload_limits: &'a LegacyPayloadLimits,
}

/// Field declaration order is wire order (`topology` consumes no bytes).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyPatchDecode<'_>,
    fingerprint = FINGERPRINT_PATCH,
    expected = "patch fingerprint"
)]
pub struct LegacyPatchState {
    #[legacy(value = ctx.topology.clone())]
    pub topology: LegacyPatchTopology,
    pub active: bool,
    /// Four obsolete lock booleans are skipped by the Original. They are
    /// opaque compiler-era bytes, not authoritative boolean values.
    #[legacy(bytes)]
    pub obsolete_lock_bytes: [u8; 4],
    pub locked: bool,
    #[legacy(read = read_occupants(reader, ctx.limits))]
    pub occupants: Vec<LegacyElementRef>,
    pub active_now: bool,
    pub applied_now: bool,
    pub in_transition_now: bool,
    #[legacy(read = read_patch_fx(reader, ctx))]
    pub fx: Option<LegacyFxPayload>,
    pub display_doors: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(fingerprint = FINGERPRINT_DOOR, expected = "door fingerprint")]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyLayeredRepulsivePoint {
    #[legacy(flatten)]
    pub point: LegacyRepulsivePoint,
    pub layer: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
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
            reader.read_signature("fingerprint", FINGERPRINT_GRID, "spatial-grid fingerprint")?;

            let mut patches = Vec::new();
            reserve(reader, &mut patches, topology.patches.len(), "patches")?;
            for (index, patch_topology) in topology.patches.iter().enumerate() {
                patches.push(reader.scope_indexed("patches", index, |reader| {
                    LegacyPatchState::read(
                        reader,
                        &LegacyPatchDecode {
                            topology: patch_topology,
                            limits,
                            payload_limits,
                        },
                    )
                })?);
            }

            let mut gates = Vec::new();
            reserve(reader, &mut gates, topology.gates.len(), "gates")?;
            for (index, topology) in topology.gates.iter().enumerate() {
                gates.push(match topology {
                    LegacyGateTopology::Door => {
                        LegacyGateState::Door(reader.scope_indexed("gates", index, |reader| {
                            LegacyDoorState::read(reader, &())
                        })?)
                    }
                    LegacyGateTopology::Stateless => LegacyGateState::Stateless,
                });
            }

            let mut script_sectors = Vec::new();
            for (index, script_object) in topology.script_objects.iter().enumerate() {
                let LegacyScriptObjectTopology::Sector { associated_class } = script_object else {
                    continue;
                };
                script_sectors.push(reader.scope_indexed("script_objects", index, |reader| {
                    reader.read_signature(
                        "fingerprint",
                        FINGERPRINT_SCRIPT_SECTOR,
                        "script-sector fingerprint",
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
                })?);
            }

            let mut special_sectors = Vec::new();
            for (index, sector) in topology.sectors.iter().enumerate() {
                let state = match sector {
                    LegacySectorTopology::NullOrOrdinary => continue,
                    LegacySectorTopology::Door => {
                        reader.read_signature(
                            "sector_door.fingerprint",
                            FINGERPRINT_DOOR_SECTOR,
                            "door-sector fingerprint",
                        )?;
                        LegacySpecialSectorState::Door {
                            sector_index: index,
                            active: reader.read_bool(format!("sectors[{index}].active"))?,
                        }
                    }
                    LegacySectorTopology::Building => {
                        reader.scope_indexed("sectors", index, |reader| {
                            reader.scope("building", |reader| {
                                reader.read_signature(
                                    "fingerprint",
                                    FINGERPRINT_BUILDING_SECTOR,
                                    "building-sector fingerprint",
                                )?;
                                Ok(LegacySpecialSectorState::Building {
                                    sector_index: index,
                                    occupants: read_occupants(reader, limits)?,
                                    arrow_reserve: reader.read_bool("arrow_reserve")?,
                                })
                            })
                        })?
                    }
                    LegacySectorTopology::Lift => {
                        reader.scope_indexed("sectors", index, |reader| {
                            reader.scope("lift", |reader| {
                                reader.read_signature(
                                    "fingerprint",
                                    FINGERPRINT_LIFT_SECTOR,
                                    "lift-sector fingerprint",
                                )?;
                                Ok(LegacySpecialSectorState::Lift {
                                    sector_index: index,
                                    occupants_pc: reader.read_u16("occupants_pc")?,
                                    occupants: reader.read_u16("occupants")?,
                                    occupied_upwards: reader.read_bool("occupied_upwards")?,
                                    occupied_downwards: reader.read_bool("occupied_downwards")?,
                                    wait_time: reader.read_u32("wait_time")?,
                                })
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
            let static_repulsive_points =
                reader.read_list("static_repulsive_points", point_count, |reader, item| {
                    LegacyLayeredRepulsivePoint::read_field(reader, item, &())
                })?;

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
        if let Some(fx) = patch.fx
            && fx.class != LegacyElementClass::Fx
        {
            return Err(reader.invalid_value(
                offset,
                format!("topology.patches[{index}].fx.class"),
                format_args!("{:?}", fx.class),
                "effect element (the concrete type owned by a patch)",
            ));
        }
    }
    Ok(())
}

/// The patch-owned effect exists exactly when the mission constructed one.
fn read_patch_fx(
    reader: &mut LegacyReader<'_>,
    ctx: &LegacyPatchDecode<'_>,
) -> LegacyResult<Option<LegacyFxPayload>> {
    ctx.topology
        .fx
        .map(|identity| {
            LegacyFxPayload::read_field(
                reader,
                "fx",
                &LegacyElementBaseDecode {
                    limits: ctx.payload_limits,
                    expected_creation_order: Some(identity.creation_order),
                    expected_class: Some(identity.class),
                },
            )
        })
        .transpose()
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
    reader.read_list("occupants", count, |reader, item| {
        LegacyElementRef::read_field(reader, item, &())
    })
}

#[cfg(test)]
mod tests {

    use super::*;

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

    use crate::legacy_save::test_support::with_reader;

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
        bytes.push(0);
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
