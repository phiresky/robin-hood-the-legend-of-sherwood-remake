//! Strict Original v48 engine-tail decoding after `RHTitbits::Serialize`.
//!
//! This is the final byte range written by `RHEngine::Serialize`:
//!
//! 1. optional mission-global VM members;
//! 2. engine script globals;
//! 3. timer and camera sequence-element references;
//! 4. `RHArtificialIntelligence::SerializeAllAI`;
//! 5. `RHPathFinder::Serialize`;
//! 6. the dead-PC reference and `RHMissionStat`;
//! 7. the pending shield danger point/reference, followed by EOF.
//!
//! Neither global AI nor Pathfinder stores its mission-sized shape. The
//! caller must supply the seek-point, archery-sector, and path-graph topology
//! created by the exact mission data. No boundary scanning or inferred count
//! is used.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::payload_base::{
    LegacyElementRef, LegacyPoint2, LegacyPoint3, LegacySequenceElementRef, read_element_ref,
    read_sequence_element_ref,
};
use super::payload_vm::{LegacyVmMemberDecoder, LegacyVmMemberSection};

const FINGERPRINT_GLOBAL_AI: [u8; 16] = hex16("cfc60d0e2673c85bb064bb0ce0d46f99");
const FINGERPRINT_SEEK_POINT: [u8; 16] = hex16("1d8f13888a44ed97abc70ec98d7132a1");
const FINGERPRINT_ARCHERY_SECTOR: [u8; 16] = hex16("91449b8fa703552a40004516743c9e83");
const FINGERPRINT_PATHFINDER: [u8; 16] = hex16("899c5131be364a32c1f14c26fd308ac3");
const FINGERPRINT_MISSION_STAT: [u8; 16] = hex16("959b4584dd5ff50e9dc33b6e995d4437");

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPostTailLimits {
    pub script_globals: usize,
    pub timer_sequence_elements: usize,
    pub path_requests: usize,
    pub mission_pc_names: usize,
    pub wide_string_code_units: usize,
    pub seek_points: usize,
    pub archery_sectors: usize,
    pub archery_points_per_sector: usize,
    pub path_graph_layers: usize,
    pub path_graph_areas_per_layer: usize,
}

impl Default for LegacyPostTailLimits {
    fn default() -> Self {
        Self {
            script_globals: 65_535,
            timer_sequence_elements: 65_535,
            path_requests: 65_535,
            mission_pc_names: 65_535,
            wide_string_code_units: 4_096,
            seek_points: 65_535,
            archery_sectors: 65_535,
            archery_points_per_sector: 65_535,
            path_graph_layers: 65_535,
            path_graph_areas_per_layer: 65_535,
        }
    }
}

/// Mission-created shape omitted from the v48 save stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPostTailTopology {
    /// SCB class bound to the engine-global VM object. `None` means the
    /// Original's global script serialization option was disabled, so this
    /// section occupies zero bytes.
    pub global_script_class: Option<String>,
    /// One entry per `RHArtificialIntelligence::marraySeekPoints` element.
    pub seek_point_count: usize,
    /// Number of `RHPointArchery` entries in each mission-created
    /// `RHSectorArchery`, in stable array order.
    pub archery_sector_point_counts: Vec<usize>,
    /// Number of graph areas in every `RHPathFinder` layer, in stable order.
    pub path_graph_area_counts: Vec<usize>,
    /// Exact size of the containing save file. The decoder requires the
    /// shield reference to end at this offset.
    pub eof_offset: u64,
}

pub trait LegacyPostTailDecodeContext {
    fn read_global_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        script_class: &str,
    ) -> LegacyResult<LegacyVmMemberSection>;
}

impl LegacyPostTailDecodeContext for LegacyVmMemberDecoder<'_> {
    fn read_global_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        script_class: &str,
    ) -> LegacyResult<LegacyVmMemberSection> {
        self.read_class_members(reader, script_class)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyEnginePostTitbitsTail {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub global_script_members: Option<LegacyVmMemberSection>,
    pub script_globals: LegacyScriptGlobals,
    pub timers: LegacyTimerSequenceState,
    pub global_ai: LegacyGlobalAiState,
    pub pathfinder: LegacyPathfinderState,
    pub dead_pc: LegacyElementRef,
    pub mission_statistics: LegacyMissionStatistics,
    pub shield: LegacyPendingShieldState,
    pub end_offset: u64,
}

impl LegacyEnginePostTitbitsTail {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        topology: &LegacyPostTailTopology,
        limits: &LegacyPostTailLimits,
        context: &dyn LegacyPostTailDecodeContext,
    ) -> LegacyResult<Self> {
        reader.scope("post_titbits_tail", |reader| {
            audit_abi(abi_profile);
            validate_topology(reader, topology, limits)?;
            let start_offset = reader.offset();

            let global_script_members = topology
                .global_script_class
                .as_deref()
                .map(|script_class| {
                    reader.scope("global_script_members", |reader| {
                        context.read_global_script_members(reader, script_class)
                    })
                })
                .transpose()?;
            let script_globals = LegacyScriptGlobals::read(reader, limits)?;
            let timers = LegacyTimerSequenceState::read(reader, limits)?;
            let global_ai = LegacyGlobalAiState::read(reader, abi_profile, topology)?;
            let pathfinder = LegacyPathfinderState::read(reader, topology, limits)?;
            let dead_pc = read_element_ref(reader, "dead_pc")?;
            let mission_statistics = LegacyMissionStatistics::read(reader, limits)?;
            let shield = LegacyPendingShieldState::read(reader)?;

            let end_offset = reader.offset();
            if end_offset != topology.eof_offset {
                return Err(reader.invalid_value(
                    end_offset,
                    "eof",
                    end_offset,
                    "exact caller-supplied save-stream EOF offset",
                ));
            }

            Ok(Self {
                abi_profile,
                start_offset,
                global_script_members,
                script_globals,
                timers,
                global_ai,
                pathfinder,
                dead_pc,
                mission_statistics,
                shield,
                end_offset,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyScriptGlobals {
    pub start_offset: u64,
    /// Exact two's-complement bits of the Original `int` array.
    pub values: Vec<i32>,
    pub end_offset: u64,
}

impl LegacyScriptGlobals {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyPostTailLimits) -> LegacyResult<Self> {
        reader.scope("script_globals", |reader| {
            let start_offset = reader.offset();
            let count = reader.read_count_u32("count", limits.script_globals)?;
            let mut values = Vec::new();
            reserve(reader, &mut values, count, "values")?;
            for index in 0..count {
                values.push(reader.read_i32(format!("values[{index}]"))?);
            }
            Ok(Self {
                start_offset,
                values,
                end_offset: reader.offset(),
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyTimerSequenceState {
    pub start_offset: u64,
    pub timer_elements: Vec<LegacySequenceElementRef>,
    pub camera_element: Option<LegacySequenceElementRef>,
    pub end_offset: u64,
}

impl LegacyTimerSequenceState {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyPostTailLimits) -> LegacyResult<Self> {
        reader.scope("timer_sequences", |reader| {
            let start_offset = reader.offset();
            let count =
                reader.read_count_u32("timer_elements.count", limits.timer_sequence_elements)?;
            let mut timer_elements = Vec::new();
            reserve(reader, &mut timer_elements, count, "timer_elements")?;
            for index in 0..count {
                timer_elements.push(read_sequence_element_ref(
                    reader,
                    format!("timer_elements[{index}]"),
                )?);
            }
            let camera_element = if reader.read_bool("camera.present")? {
                Some(read_sequence_element_ref(reader, "camera.element")?)
            } else {
                None
            };
            Ok(Self {
                start_offset,
                timer_elements,
                camera_element,
                end_offset: reader.offset(),
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySeekPointStatus {
    pub frame_when_fully_interesting: u32,
    pub last_calculated_interest: u8,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyArcherySectorState {
    pub start_offset: u64,
    pub number_of_owners: u16,
    pub point_owners: Vec<LegacyElementRef>,
    pub end_offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyTimeT32(pub i32);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyGlobalAiState {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub stupid_soldiers_cheat: bool,
    pub seek_points: Vec<LegacySeekPointStatus>,
    pub archery_sectors: Vec<LegacyArcherySectorState>,
    pub green_alert_soldiers: u16,
    pub yellow_alert_soldiers: u16,
    pub red_alert_soldiers: u16,
    pub overall_alert_status: i32,
    pub overall_villain_alert_status: i32,
    /// Both supported producers are audited 32-bit builds. Their serialized
    /// `time_t` is a signed four-byte value, independent of the Rust host.
    pub saved_random_seed: LegacyTimeT32,
    pub end_offset: u64,
}

impl LegacyGlobalAiState {
    fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        topology: &LegacyPostTailTopology,
    ) -> LegacyResult<Self> {
        reader.scope("global_ai", |reader| {
            let start_offset = reader.offset();
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_GLOBAL_AI,
                "MD5(\"RHArtificialIntelligence\")",
            )?;
            let stupid_soldiers_cheat = reader.read_bool("stupid_soldiers_cheat")?;

            let mut seek_points = Vec::new();
            reserve(
                reader,
                &mut seek_points,
                topology.seek_point_count,
                "seek_points",
            )?;
            for index in 0..topology.seek_point_count {
                seek_points.push(reader.scope(format!("seek_points[{index}]"), |reader| {
                    reader.read_signature(
                        "fingerprint",
                        FINGERPRINT_SEEK_POINT,
                        "MD5(\"RHSeekPoint\")",
                    )?;
                    Ok(LegacySeekPointStatus {
                        frame_when_fully_interesting: reader
                            .read_u32("frame_when_fully_interesting")?,
                        last_calculated_interest: reader.read_u8("last_calculated_interest")?,
                        locked: reader.read_bool("locked")?,
                    })
                })?);
            }

            let mut archery_sectors = Vec::new();
            reserve(
                reader,
                &mut archery_sectors,
                topology.archery_sector_point_counts.len(),
                "archery_sectors",
            )?;
            for (sector_index, &point_count) in
                topology.archery_sector_point_counts.iter().enumerate()
            {
                archery_sectors.push(reader.scope(
                    format!("archery_sectors[{sector_index}]"),
                    |reader| {
                        let start_offset = reader.offset();
                        reader.read_signature(
                            "fingerprint",
                            FINGERPRINT_ARCHERY_SECTOR,
                            "MD5(\"RHSectorArchery\")",
                        )?;
                        let number_of_owners = reader.read_u16("number_of_owners")?;
                        let mut point_owners = Vec::new();
                        reserve(reader, &mut point_owners, point_count, "point_owners")?;
                        for point_index in 0..point_count {
                            point_owners.push(read_element_ref(
                                reader,
                                format!("point_owners[{point_index}]"),
                            )?);
                        }
                        Ok(LegacyArcherySectorState {
                            start_offset,
                            number_of_owners,
                            point_owners,
                            end_offset: reader.offset(),
                        })
                    },
                )?);
            }

            Ok(Self {
                abi_profile,
                start_offset,
                stupid_soldiers_cheat,
                seek_points,
                archery_sectors,
                green_alert_soldiers: reader.read_u16("green_alert_soldiers")?,
                yellow_alert_soldiers: reader.read_u16("yellow_alert_soldiers")?,
                red_alert_soldiers: reader.read_u16("red_alert_soldiers")?,
                overall_alert_status: reader.read_i32("overall_alert_status")?,
                overall_villain_alert_status: reader.read_i32("overall_villain_alert_status")?,
                saved_random_seed: LegacyTimeT32(read_time_t32(reader, abi_profile)?),
                end_offset: reader.offset(),
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPathRequest {
    pub action: i32,
    pub reverse: bool,
    pub use_first_point: bool,
    pub tolerance: f32,
    pub speed: u8,
    pub area: u16,
    pub half_diagonal_index: u16,
    pub layer: u16,
    pub sector: u16,
    pub goal: LegacyPoint2,
    pub source: LegacyPoint2,
    pub actor: LegacyElementRef,
    pub antagonist: LegacyElementRef,
    pub sequence_element: LegacySequenceElementRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPathfinderState {
    pub start_offset: u64,
    /// The writer always emits false after excluding an ignored front request.
    pub do_not_ignore_next_path: bool,
    pub requests: Vec<LegacyPathRequest>,
    /// Mutable graph state in mission layer/area order.
    pub layer_area_states: Vec<Vec<u32>>,
    pub end_offset: u64,
}

impl LegacyPathfinderState {
    fn read(
        reader: &mut LegacyReader<'_>,
        topology: &LegacyPostTailTopology,
        limits: &LegacyPostTailLimits,
    ) -> LegacyResult<Self> {
        reader.scope("pathfinder", |reader| {
            let start_offset = reader.offset();
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_PATHFINDER,
                "MD5(\"RHPathFinder\")",
            )?;
            let do_not_ignore_next_path = reader.read_bool("do_not_ignore_next_path")?;
            let count = read_count_u16(reader, "requests.count", limits.path_requests)?;
            let mut requests = Vec::new();
            reserve(reader, &mut requests, count, "requests")?;
            for index in 0..count {
                requests.push(reader.scope(format!("requests[{index}]"), |reader| {
                    Ok(LegacyPathRequest {
                        action: reader.read_i32("action")?,
                        reverse: reader.read_bool("reverse")?,
                        use_first_point: reader.read_bool("use_first_point")?,
                        tolerance: reader.read_f32("tolerance")?,
                        speed: reader.read_u8("speed")?,
                        area: reader.read_u16("area")?,
                        half_diagonal_index: reader.read_u16("half_diagonal_index")?,
                        layer: reader.read_u16("layer")?,
                        sector: reader.read_u16("sector")?,
                        goal: read_point2(reader, "goal")?,
                        source: read_point2(reader, "source")?,
                        actor: read_element_ref(reader, "actor")?,
                        antagonist: read_element_ref(reader, "antagonist")?,
                        sequence_element: read_sequence_element_ref(reader, "sequence_element")?,
                    })
                })?);
            }

            let mut layer_area_states = Vec::new();
            reserve(
                reader,
                &mut layer_area_states,
                topology.path_graph_area_counts.len(),
                "layer_area_states",
            )?;
            for (layer_index, &area_count) in topology.path_graph_area_counts.iter().enumerate() {
                layer_area_states.push(reader.scope(
                    format!("layer_area_states[{layer_index}]"),
                    |reader| {
                        let mut states = Vec::new();
                        reserve(reader, &mut states, area_count, "states")?;
                        for area_index in 0..area_count {
                            states.push(reader.read_u32(format!("states[{area_index}]"))?);
                        }
                        Ok(states)
                    },
                )?);
            }

            Ok(Self {
                start_offset,
                do_not_ignore_next_path,
                requests,
                layer_area_states,
                end_offset: reader.offset(),
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyMissionStatistics {
    pub start_offset: u64,
    pub collected_money: u32,
    pub bonus_money: u32,
    pub soldier_money: u32,
    pub living_soldier_count: u32,
    pub total_soldier_count: u32,
    pub new_peasant_count: u32,
    pub killed_peasant_count: u32,
    pub killed_allied_count: u32,
    pub added_score: u32,
    pub pc_names: Vec<String>,
    pub end_offset: u64,
}

impl LegacyMissionStatistics {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyPostTailLimits) -> LegacyResult<Self> {
        reader.scope("mission_statistics", |reader| {
            let start_offset = reader.offset();
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_MISSION_STAT,
                "MD5(\"RHMissionStat\")",
            )?;
            let collected_money = reader.read_u32("collected_money")?;
            let bonus_money = reader.read_u32("bonus_money")?;
            let soldier_money = reader.read_u32("soldier_money")?;
            let living_soldier_count = reader.read_u32("living_soldier_count")?;
            let total_soldier_count = reader.read_u32("total_soldier_count")?;
            let new_peasant_count = reader.read_u32("new_peasant_count")?;
            let killed_peasant_count = reader.read_u32("killed_peasant_count")?;
            let killed_allied_count = reader.read_u32("killed_allied_count")?;
            let added_score = reader.read_u32("added_score")?;
            let count = reader.read_count_u32("pc_names.count", limits.mission_pc_names)?;
            let mut pc_names = Vec::new();
            reserve(reader, &mut pc_names, count, "pc_names")?;
            for index in 0..count {
                pc_names.push(reader.read_wide_string(
                    format!("pc_names[{index}]"),
                    limits.wide_string_code_units,
                )?);
            }
            Ok(Self {
                start_offset,
                collected_money,
                bonus_money,
                soldier_money,
                living_soldier_count,
                total_soldier_count,
                new_peasant_count,
                killed_peasant_count,
                killed_allied_count,
                added_score,
                pc_names,
                end_offset: reader.offset(),
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyPendingShieldState {
    pub start_offset: u64,
    pub danger_point: LegacyPoint3,
    pub protected_pc: LegacyElementRef,
    pub end_offset: u64,
}

impl LegacyPendingShieldState {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        reader.scope("shield", |reader| {
            let start_offset = reader.offset();
            Ok(Self {
                start_offset,
                danger_point: read_point3(reader, "danger_point")?,
                protected_pc: read_element_ref(reader, "protected_pc")?,
                end_offset: reader.offset(),
            })
        })
    }
}

fn validate_topology(
    reader: &mut LegacyReader<'_>,
    topology: &LegacyPostTailTopology,
    limits: &LegacyPostTailLimits,
) -> LegacyResult<()> {
    let offset = reader.offset();
    validate_topology_count(
        reader,
        offset,
        "topology.seek_point_count",
        topology.seek_point_count,
        limits.seek_points,
    )?;
    validate_topology_count(
        reader,
        offset,
        "topology.archery_sector_count",
        topology.archery_sector_point_counts.len(),
        limits.archery_sectors,
    )?;
    for (index, &count) in topology.archery_sector_point_counts.iter().enumerate() {
        validate_topology_count(
            reader,
            offset,
            format!("topology.archery_sector_point_counts[{index}]"),
            count,
            limits.archery_points_per_sector,
        )?;
    }
    validate_topology_count(
        reader,
        offset,
        "topology.path_graph_layer_count",
        topology.path_graph_area_counts.len(),
        limits.path_graph_layers,
    )?;
    for (index, &count) in topology.path_graph_area_counts.iter().enumerate() {
        validate_topology_count(
            reader,
            offset,
            format!("topology.path_graph_area_counts[{index}]"),
            count,
            limits.path_graph_areas_per_layer,
        )?;
    }
    if topology.eof_offset < offset {
        return Err(reader.invalid_value(
            offset,
            "topology.eof_offset",
            topology.eof_offset,
            "an EOF offset at or after the tail start",
        ));
    }
    Ok(())
}

fn validate_topology_count(
    reader: &mut LegacyReader<'_>,
    offset: u64,
    field: impl std::fmt::Display,
    count: usize,
    maximum: usize,
) -> LegacyResult<()> {
    if count > maximum {
        return Err(reader.invalid_value(
            offset,
            field,
            count,
            "mission topology count within the caller-supplied limit",
        ));
    }
    Ok(())
}

fn read_time_t32(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
) -> LegacyResult<i32> {
    match abi_profile {
        LegacySaveAbiProfile::RetailWindowsX86V48 | LegacySaveAbiProfile::PortLinuxI386V48 => {
            reader.read_i32("saved_random_seed")
        }
    }
}

fn read_count_u16(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display + Copy,
    maximum: usize,
) -> LegacyResult<usize> {
    let offset = reader.offset();
    let count = reader.read_u16(field)? as usize;
    if count > maximum {
        return Err(reader.invalid_value(
            offset,
            field,
            count,
            "count within the caller-supplied limit",
        ));
    }
    Ok(count)
}

fn read_point2(
    reader: &mut LegacyReader<'_>,
    field: impl Into<String>,
) -> LegacyResult<LegacyPoint2> {
    reader.scope(field, |reader| {
        Ok(LegacyPoint2 {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
        })
    })
}

fn read_point3(
    reader: &mut LegacyReader<'_>,
    field: impl Into<String>,
) -> LegacyResult<LegacyPoint3> {
    reader.scope(field, |reader| {
        Ok(LegacyPoint3 {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
            z: reader.read_f32("z")?,
        })
    })
}

fn reserve<T>(
    reader: &mut LegacyReader<'_>,
    values: &mut Vec<T>,
    count: usize,
    field: impl std::fmt::Display,
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
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::legacy_io::LegacyIoErrorKind;
    use crate::sbfile::{SB_FILE_READ, SbFile};

    struct NoVm;

    impl LegacyPostTailDecodeContext for NoVm {
        fn read_global_script_members(
            &self,
            reader: &mut LegacyReader<'_>,
            _script_class: &str,
        ) -> LegacyResult<LegacyVmMemberSection> {
            let offset = reader.offset();
            Err(reader.invalid_value(
                offset,
                "global_script_members",
                "unexpected callback",
                "no VM members when global_script_class is absent",
            ))
        }
    }

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn minimal_tail() -> Vec<u8> {
        let mut bytes = Vec::new();
        push_u32(&mut bytes, 0); // script globals
        push_u32(&mut bytes, 0); // timer elements
        bytes.push(0); // camera absent

        bytes.extend_from_slice(&FINGERPRINT_GLOBAL_AI);
        bytes.push(0); // stupid soldiers
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 2);
        push_u16(&mut bytes, 3);
        push_i32(&mut bytes, 4);
        push_i32(&mut bytes, 5);
        push_i32(&mut bytes, -6); // 32-bit time_t

        bytes.extend_from_slice(&FINGERPRINT_PATHFINDER);
        bytes.push(0); // do not ignore
        push_u16(&mut bytes, 0); // requests

        push_u32(&mut bytes, u32::MAX); // dead PC
        bytes.extend_from_slice(&FINGERPRINT_MISSION_STAT);
        for value in 10..19 {
            push_u32(&mut bytes, value);
        }
        push_u32(&mut bytes, 1); // PC names
        push_u16(&mut bytes, 2);
        push_u16(&mut bytes, b'R' as u16);
        push_u16(&mut bytes, b'H' as u16);

        push_f32(&mut bytes, 1.0);
        push_f32(&mut bytes, 2.0);
        push_f32(&mut bytes, 3.0);
        push_u32(&mut bytes, u32::MAX); // protected PC
        bytes
    }

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut temporary = NamedTempFile::new().unwrap();
        temporary.write_all(bytes).unwrap();
        temporary.flush().unwrap();
        let mut file = SbFile::open(temporary.path().to_str().unwrap(), SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    fn empty_topology(eof_offset: u64) -> LegacyPostTailTopology {
        LegacyPostTailTopology {
            global_script_class: None,
            seek_point_count: 0,
            archery_sector_point_counts: Vec::new(),
            path_graph_area_counts: Vec::new(),
            eof_offset,
        }
    }

    #[test]
    fn decodes_minimal_tail_for_both_v48_abis() {
        let bytes = minimal_tail();
        for abi in [
            LegacySaveAbiProfile::RetailWindowsX86V48,
            LegacySaveAbiProfile::PortLinuxI386V48,
        ] {
            with_reader(&bytes, |reader| {
                let tail = LegacyEnginePostTitbitsTail::read(
                    reader,
                    abi,
                    &empty_topology(bytes.len() as u64),
                    &LegacyPostTailLimits::default(),
                    &NoVm,
                )
                .unwrap();
                assert_eq!(tail.end_offset, bytes.len() as u64);
                assert_eq!(tail.global_ai.saved_random_seed, LegacyTimeT32(-6));
                assert_eq!(tail.global_ai.green_alert_soldiers, 1);
                assert_eq!(tail.mission_statistics.pc_names, ["RH"]);
                assert_eq!(
                    tail.shield.danger_point,
                    LegacyPoint3 {
                        x: 1.0,
                        y: 2.0,
                        z: 3.0
                    }
                );
            });
        }
    }

    #[test]
    fn decodes_mission_sized_global_ai_and_path_graph_in_exact_order() {
        let mut bytes = Vec::new();
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        bytes.push(0);
        bytes.extend_from_slice(&FINGERPRINT_GLOBAL_AI);
        bytes.push(1);
        bytes.extend_from_slice(&FINGERPRINT_SEEK_POINT);
        push_u32(&mut bytes, 7);
        bytes.push(8);
        bytes.push(1);
        bytes.extend_from_slice(&FINGERPRINT_ARCHERY_SECTOR);
        push_u16(&mut bytes, 2);
        push_u32(&mut bytes, 100);
        push_u32(&mut bytes, u32::MAX);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_i32(&mut bytes, 0);
        push_i32(&mut bytes, 0);
        push_i32(&mut bytes, 0);
        bytes.extend_from_slice(&FINGERPRINT_PATHFINDER);
        bytes.push(0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 11);
        push_u32(&mut bytes, 12);
        push_u32(&mut bytes, 13);
        push_u32(&mut bytes, u32::MAX);
        bytes.extend_from_slice(&FINGERPRINT_MISSION_STAT);
        for _ in 0..10 {
            push_u32(&mut bytes, 0);
        }
        for _ in 0..3 {
            push_f32(&mut bytes, 0.0);
        }
        push_u32(&mut bytes, u32::MAX);

        let topology = LegacyPostTailTopology {
            global_script_class: None,
            seek_point_count: 1,
            archery_sector_point_counts: vec![2],
            path_graph_area_counts: vec![2, 1],
            eof_offset: bytes.len() as u64,
        };
        with_reader(&bytes, |reader| {
            let tail = LegacyEnginePostTitbitsTail::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &topology,
                &LegacyPostTailLimits::default(),
                &NoVm,
            )
            .unwrap();
            assert_eq!(tail.global_ai.seek_points[0].last_calculated_interest, 8);
            assert_eq!(
                tail.global_ai.archery_sectors[0].point_owners,
                [LegacyElementRef(Some(100)), LegacyElementRef(None)]
            );
            assert_eq!(tail.pathfinder.layer_area_states, [vec![11, 12], vec![13]]);
        });
    }

    #[test]
    fn rejects_bad_fingerprint_and_truncated_time_t_with_precise_fields() {
        let mut bad_fingerprint = minimal_tail();
        bad_fingerprint[9] ^= 0xff;
        let error = with_reader(&bad_fingerprint, |reader| {
            LegacyEnginePostTitbitsTail::read(
                reader,
                LegacySaveAbiProfile::RetailWindowsX86V48,
                &empty_topology(bad_fingerprint.len() as u64),
                &LegacyPostTailLimits::default(),
                &NoVm,
            )
            .unwrap_err()
        });
        assert_eq!(error.field, "post_titbits_tail.global_ai.fingerprint");
        assert!(matches!(error.kind, LegacyIoErrorKind::InvalidValue { .. }));

        let mut truncated = minimal_tail();
        let time_t_end = 9 + 16 + 1 + 6 + 8 + 4;
        truncated.truncate(time_t_end - 1);
        let error = with_reader(&truncated, |reader| {
            LegacyEnginePostTitbitsTail::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &empty_topology(truncated.len() as u64),
                &LegacyPostTailLimits::default(),
                &NoVm,
            )
            .unwrap_err()
        });
        assert_eq!(error.field, "post_titbits_tail.global_ai.saved_random_seed");
    }

    #[test]
    fn enforces_count_limits_and_exact_eof() {
        let mut excessive_globals = minimal_tail();
        excessive_globals[..4].copy_from_slice(&2u32.to_le_bytes());
        let limits = LegacyPostTailLimits {
            script_globals: 1,
            ..LegacyPostTailLimits::default()
        };
        let error = with_reader(&excessive_globals, |reader| {
            LegacyEnginePostTitbitsTail::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &empty_topology(excessive_globals.len() as u64),
                &limits,
                &NoVm,
            )
            .unwrap_err()
        });
        assert!(matches!(
            error.kind,
            LegacyIoErrorKind::CountLimit {
                count: 2,
                maximum: 1
            }
        ));

        let bytes = minimal_tail();
        let error = with_reader(&bytes, |reader| {
            LegacyEnginePostTitbitsTail::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &empty_topology(bytes.len() as u64 + 1),
                &LegacyPostTailLimits::default(),
                &NoVm,
            )
            .unwrap_err()
        });
        assert_eq!(error.field, "post_titbits_tail.eof");
    }
}
