//! Strict original-game v48 engine-tail decoding after titbit serialization.
//!
//! This is the final byte range written by engine serialization:
//!
//! 1. optional mission-global VM members;
//! 2. engine script globals;
//! 3. timer and camera sequence-element references;
//! 4. all AI state;
//! 5. pathfinder state;
//! 6. the dead-player reference and mission statistics;
//! 7. the pending shield danger point/reference, followed by EOF.
//!
//! Neither global AI nor Pathfinder stores its mission-sized shape. The
//! caller must supply the seek-point, archery-sector, and path-graph topology
//! created by the exact mission data. No boundary scanning or inferred count
//! is used. Field declaration order is wire order.

use super::read_helpers::DEFAULT_BULK_LIMIT;
use super::read_helpers::hex16;
use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyRead, LegacyReader, LegacyResult};

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
    /// Maximum number of `0xff` bytes introduced by the Linux port's
    /// historical save-file EOF-copy bug.
    pub linux_copy_eof_bytes: usize,
}

impl Default for LegacyPostTailLimits {
    fn default() -> Self {
        Self {
            script_globals: crate::natives::DEFAULT_SCRIPT_GLOBAL_SLOT_LIMIT,
            timer_sequence_elements: DEFAULT_BULK_LIMIT,
            path_requests: DEFAULT_BULK_LIMIT,
            mission_pc_names: DEFAULT_BULK_LIMIT,
            wide_string_code_units: 4_096,
            seek_points: DEFAULT_BULK_LIMIT,
            archery_sectors: DEFAULT_BULK_LIMIT,
            archery_points_per_sector: DEFAULT_BULK_LIMIT,
            path_graph_layers: DEFAULT_BULK_LIMIT,
            path_graph_areas_per_layer: DEFAULT_BULK_LIMIT,
            linux_copy_eof_bytes: DEFAULT_BULK_LIMIT,
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
    /// One entry per AI seek point.
    pub seek_point_count: usize,
    /// Number of archery points in each mission-created
    /// archery sector, in stable array order.
    pub archery_sector_point_counts: Vec<usize>,
    /// Number of graph areas in every pathfinder layer, in stable order.
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

/// Decode context shared by the engine-tail sections.
#[derive(Clone, Copy)]
pub struct LegacyPostTailDecode<'a> {
    pub abi_profile: LegacySaveAbiProfile,
    pub topology: &'a LegacyPostTailTopology,
    pub limits: &'a LegacyPostTailLimits,
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
    /// End of the actual engine serialization payload, before any bytes
    /// appended by the old Linux `CopyFile` loop.
    pub serialized_end_offset: u64,
    pub trailing_linux_copy_eof_bytes: usize,
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
        let ctx = LegacyPostTailDecode {
            abi_profile,
            topology,
            limits,
        };
        reader.scope("post_titbits_tail", |reader| {
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
            let script_globals = LegacyScriptGlobals::read_field(reader, "script_globals", &ctx)?;
            let timers = LegacyTimerSequenceState::read_field(reader, "timer_sequences", &ctx)?;
            let global_ai = LegacyGlobalAiState::read_field(reader, "global_ai", &ctx)?;
            let pathfinder = LegacyPathfinderState::read_field(reader, "pathfinder", &ctx)?;
            let dead_pc = read_element_ref(reader, "dead_pc")?;
            let mission_statistics =
                LegacyMissionStatistics::read_field(reader, "mission_statistics", &ctx)?;
            let shield = LegacyPendingShieldState::read_field(reader, "shield", &())?;

            let serialized_end_offset = reader.offset();
            if serialized_end_offset > topology.eof_offset {
                return Err(reader.invalid_value(
                    serialized_end_offset,
                    "eof",
                    serialized_end_offset,
                    "exact caller-supplied save-stream EOF offset",
                ));
            }
            let trailing_linux_copy_eof_bytes_u64 = topology.eof_offset - serialized_end_offset;
            let trailing_linux_copy_eof_bytes = usize::try_from(trailing_linux_copy_eof_bytes_u64)
                .map_err(|_| {
                    reader.invalid_value(
                        serialized_end_offset,
                        "linux_copy_eof_bytes",
                        trailing_linux_copy_eof_bytes_u64,
                        "trailing byte count representable on this host",
                    )
                })?;
            if trailing_linux_copy_eof_bytes > 0 {
                if abi_profile != LegacySaveAbiProfile::PortLinuxI386V48 {
                    return Err(reader.invalid_value(
                        serialized_end_offset,
                        "eof",
                        serialized_end_offset,
                        "exact caller-supplied Windows save-stream EOF offset",
                    ));
                }
                if trailing_linux_copy_eof_bytes > limits.linux_copy_eof_bytes {
                    return Err(reader.invalid_value(
                        serialized_end_offset,
                        "linux_copy_eof_bytes",
                        trailing_linux_copy_eof_bytes,
                        "known Linux CopyFile artifact count within the configured limit",
                    ));
                }
                for index in 0..trailing_linux_copy_eof_bytes {
                    let byte = reader.read_u8(format_args!("linux_copy_eof_bytes[{index}]"))?;
                    if byte != 0xff {
                        let byte_offset = reader.offset() - 1;
                        return Err(reader.invalid_value(
                            byte_offset,
                            format_args!("linux_copy_eof_bytes[{index}]"),
                            format_args!("0x{byte:02x}"),
                            "0xff written by fputc(fgetc(...)) at EOF",
                        ));
                    }
                }
            }
            let end_offset = reader.offset();

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
                serialized_end_offset,
                trailing_linux_copy_eof_bytes,
                end_offset,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyPostTailDecode<'_>)]
pub struct LegacyScriptGlobals {
    #[legacy(offset)]
    pub start_offset: u64,
    /// Exact two's-complement bits of the original game's signed-integer array.
    #[legacy(count_u32 = ctx.limits.script_globals, count_name = "count")]
    pub values: Vec<i32>,
    #[legacy(offset)]
    pub end_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyPostTailDecode<'_>)]
pub struct LegacyTimerSequenceState {
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(count_u32 = ctx.limits.timer_sequence_elements)]
    pub timer_elements: Vec<LegacySequenceElementRef>,
    #[legacy(read = read_camera_element(reader))]
    pub camera_element: Option<LegacySequenceElementRef>,
    #[legacy(offset)]
    pub end_offset: u64,
}

fn read_camera_element(
    reader: &mut LegacyReader<'_>,
) -> LegacyResult<Option<LegacySequenceElementRef>> {
    if reader.read_bool("camera.present")? {
        Ok(Some(read_sequence_element_ref(reader, "camera.element")?))
    } else {
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(fingerprint = FINGERPRINT_SEEK_POINT, expected = "seek-point fingerprint")]
pub struct LegacySeekPointStatus {
    pub frame_when_fully_interesting: u32,
    pub last_calculated_interest: u8,
    pub locked: bool,
}

/// Context: the mission-created number of archery points in this sector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = usize)]
pub struct LegacyArcherySectorState {
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(
        fingerprint = FINGERPRINT_ARCHERY_SECTOR,
        expected = "archery-sector fingerprint"
    )]
    pub number_of_owners: u16,
    #[legacy(len = *ctx)]
    pub point_owners: Vec<LegacyElementRef>,
    #[legacy(offset)]
    pub end_offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyTimeT32(pub i32);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyPostTailDecode<'_>)]
pub struct LegacyGlobalAiState {
    #[legacy(value = ctx.abi_profile)]
    pub abi_profile: LegacySaveAbiProfile,
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(fingerprint = FINGERPRINT_GLOBAL_AI, expected = "AI fingerprint")]
    pub stupid_soldiers_cheat: bool,
    #[legacy(len = ctx.topology.seek_point_count)]
    pub seek_points: Vec<LegacySeekPointStatus>,
    #[legacy(read = read_archery_sectors(reader, ctx.topology))]
    pub archery_sectors: Vec<LegacyArcherySectorState>,
    pub green_alert_soldiers: u16,
    pub yellow_alert_soldiers: u16,
    pub red_alert_soldiers: u16,
    pub overall_alert_status: i32,
    pub overall_villain_alert_status: i32,
    /// Both supported producers are audited 32-bit builds. Their serialized
    /// `time_t` is a signed four-byte value, independent of the Rust host.
    #[legacy(read = read_time_t32(reader, ctx.abi_profile).map(LegacyTimeT32))]
    pub saved_random_seed: LegacyTimeT32,
    #[legacy(offset)]
    pub end_offset: u64,
}

fn read_archery_sectors(
    reader: &mut LegacyReader<'_>,
    topology: &LegacyPostTailTopology,
) -> LegacyResult<Vec<LegacyArcherySectorState>> {
    let point_counts = &topology.archery_sector_point_counts;
    let mut point_counts_iter = point_counts.iter();
    reader.read_list("archery_sectors", point_counts.len(), |reader, item| {
        let point_count = point_counts_iter
            .next()
            .expect("read_list requests exactly one item per archery sector");
        LegacyArcherySectorState::read_field(reader, item, point_count)
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyPostTailDecode<'_>)]
pub struct LegacyPathfinderState {
    #[legacy(offset)]
    pub start_offset: u64,
    /// The writer always emits false after excluding an ignored front request.
    #[legacy(fingerprint = FINGERPRINT_PATHFINDER, expected = "pathfinder fingerprint")]
    pub do_not_ignore_next_path: bool,
    #[legacy(count_u16 = ctx.limits.path_requests)]
    pub requests: Vec<LegacyPathRequest>,
    /// Mutable graph state in mission layer/area order.
    #[legacy(read = read_layer_area_states(reader, ctx.topology))]
    pub layer_area_states: Vec<Vec<u32>>,
    #[legacy(offset)]
    pub end_offset: u64,
}

fn read_layer_area_states(
    reader: &mut LegacyReader<'_>,
    topology: &LegacyPostTailTopology,
) -> LegacyResult<Vec<Vec<u32>>> {
    let area_counts = &topology.path_graph_area_counts;
    let mut area_counts_iter = area_counts.iter();
    reader.read_list("layer_area_states", area_counts.len(), |reader, item| {
        let area_count = *area_counts_iter
            .next()
            .expect("read_list requests exactly one item per path-graph layer");
        reader.scope(item, |reader| {
            reader.read_list("states", area_count, |reader, item| {
                u32::read_field(reader, item, &())
            })
        })
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyPostTailDecode<'_>)]
pub struct LegacyMissionStatistics {
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(
        fingerprint = FINGERPRINT_MISSION_STAT,
        expected = "mission-statistics fingerprint"
    )]
    pub collected_money: u32,
    pub bonus_money: u32,
    pub soldier_money: u32,
    pub living_soldier_count: u32,
    pub total_soldier_count: u32,
    pub new_peasant_count: u32,
    pub killed_peasant_count: u32,
    pub killed_allied_count: u32,
    pub added_score: u32,
    #[legacy(read = read_pc_names(reader, ctx.limits))]
    pub pc_names: Vec<String>,
    #[legacy(offset)]
    pub end_offset: u64,
}

fn read_pc_names(
    reader: &mut LegacyReader<'_>,
    limits: &LegacyPostTailLimits,
) -> LegacyResult<Vec<String>> {
    let count = reader.read_count_u32("pc_names.count", limits.mission_pc_names)?;
    reader.read_list("pc_names", count, |reader, item| {
        reader.scope(item, |reader| {
            reader.read_wide_string("value", limits.wide_string_code_units)
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyPendingShieldState {
    #[legacy(offset)]
    pub start_offset: u64,
    pub danger_point: LegacyPoint3,
    pub protected_pc: LegacyElementRef,
    #[legacy(offset)]
    pub end_offset: u64,
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

#[cfg(test)]
mod tests {
    use crate::legacy_save::test_support::{push_f32, push_i32, push_u16, push_u32};

    use super::*;
    use crate::legacy_io::LegacyIoErrorKind;

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

    use crate::legacy_save::test_support::with_reader;

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

        let mut bytes = minimal_tail();
        bytes.push(0);
        let error = with_reader(&bytes, |reader| {
            LegacyEnginePostTitbitsTail::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &empty_topology(bytes.len() as u64),
                &LegacyPostTailLimits::default(),
                &NoVm,
            )
            .unwrap_err()
        });
        assert_eq!(error.field, "post_titbits_tail.linux_copy_eof_bytes[0]");
    }

    #[test]
    fn accepts_only_known_linux_copyfile_eof_artifacts() {
        let mut bytes = minimal_tail();
        let serialized_len = bytes.len() as u64;
        bytes.extend_from_slice(&[0xff; 3]);

        let tail = with_reader(&bytes, |reader| {
            LegacyEnginePostTitbitsTail::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &empty_topology(bytes.len() as u64),
                &LegacyPostTailLimits::default(),
                &NoVm,
            )
            .unwrap()
        });
        assert_eq!(tail.serialized_end_offset, serialized_len);
        assert_eq!(tail.trailing_linux_copy_eof_bytes, 3);
        assert_eq!(tail.end_offset, bytes.len() as u64);

        let error = with_reader(&bytes, |reader| {
            LegacyEnginePostTitbitsTail::read(
                reader,
                LegacySaveAbiProfile::RetailWindowsX86V48,
                &empty_topology(bytes.len() as u64),
                &LegacyPostTailLimits::default(),
                &NoVm,
            )
            .unwrap_err()
        });
        assert_eq!(error.field, "post_titbits_tail.eof");
    }
}
