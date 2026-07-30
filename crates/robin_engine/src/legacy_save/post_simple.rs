//! Strict readers for the self-describing v48 engine sections after the
//! engine-owned projectile trajectory.
//!
//! This module intentionally does not orchestrate the complete engine tail:
//! `RHSequenceManager` is serialized between the follow/view references and
//! `RHGroundMark`. Each reader preserves its own byte range so the eventual
//! top-level importer can compose these sections without guessing boundaries.
//!
//! Wire order is taken from `original-code/RHengine.cpp`,
//! `original-code/RHminimap.cpp`, `original-code/RHgroundmark.cpp`, and
//! `original-code/RHtitbit.cpp`.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::payload_base::{
    LegacyBoundingBox2, LegacyElementRef, LegacyPoint2, LegacyPoint3, LegacySequenceElementRef,
    read_element_ref, read_sequence_element_ref,
};

const FINGERPRINT_MINIMAP: [u8; 16] = hex16("50f6249a4ee7522862f2c5f5442ae167");
const FINGERPRINT_GROUND_MARK: [u8; 16] = hex16("b7ebd8adf1c9be532ca495049f430da9");
const FINGERPRINT_TITBITS: [u8; 16] = hex16("0066cad32f8281aebfc9aba90a88aa34");

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
pub struct LegacyPostSimpleLimits {
    pub failed_path_requests: usize,
    pub minimap_highlights: usize,
    pub selected_elements: usize,
    pub ground_marks: usize,
    pub titbits: usize,
}

impl Default for LegacyPostSimpleLimits {
    fn default() -> Self {
        Self {
            failed_path_requests: 65_535,
            minimap_highlights: 65_535,
            selected_elements: 65_535,
            ground_marks: 65_535,
            titbits: 65_535,
        }
    }
}

/// `RHEngine::mListFailedPathRequests`.
///
/// TODO(save-import): the current Rust snapshot does not retain every one of
/// these retry parameters. Conversion must restore the authoritative queue
/// rather than synthesize a new path request from only actor and destination.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyFailedPathRequests {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub requests: Vec<LegacyFailedPathRequest>,
    pub end_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyFailedPathRequest {
    /// Raw 32-bit `RHanimation` value written by `CHECKENUM`.
    pub action: i32,
    pub reverse: bool,
    pub use_first_point: bool,
    pub tolerance: f32,
    pub speed: u8,
    pub area: u16,
    pub half_diagonal_index: u16,
    pub layer: u16,
    pub sector: u16,
    pub time: u32,
    pub goal: LegacyPoint2,
    pub source: LegacyPoint2,
    pub actor: LegacyElementRef,
    pub antagonist: LegacyElementRef,
    pub sequence_element: LegacySequenceElementRef,
}

impl LegacyFailedPathRequests {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyPostSimpleLimits,
    ) -> LegacyResult<Self> {
        reader.scope("failed_path_requests", |reader| {
            audit_abi(abi_profile);
            let start_offset = reader.offset();
            let count = read_count_u16(reader, "count", limits.failed_path_requests)?;
            let mut requests = Vec::new();
            reserve(reader, &mut requests, count, "requests")?;
            for index in 0..count {
                requests.push(reader.scope(format!("requests[{index}]"), |reader| {
                    Ok(LegacyFailedPathRequest {
                        action: reader.read_i32("action")?,
                        reverse: reader.read_bool("reverse")?,
                        use_first_point: reader.read_bool("use_first_point")?,
                        tolerance: reader.read_f32("tolerance")?,
                        speed: reader.read_u8("speed")?,
                        area: reader.read_u16("area")?,
                        half_diagonal_index: reader.read_u16("half_diagonal_index")?,
                        layer: reader.read_u16("layer")?,
                        sector: reader.read_u16("sector")?,
                        time: reader.read_u32("time")?,
                        goal: read_point2(reader, "goal")?,
                        source: read_point2(reader, "source")?,
                        actor: read_element_ref(reader, "actor")?,
                        antagonist: read_element_ref(reader, "antagonist")?,
                        sequence_element: read_sequence_element_ref(reader, "sequence_element")?,
                    })
                })?);
            }
            Ok(Self {
                abi_profile,
                start_offset,
                requests,
                end_offset: reader.offset(),
            })
        })
    }
}

/// Serializable state of the UI-owned `RHMinimap`.
///
/// TODO(save-import): loading must hand this state to the host UI layer. It
/// must not be dropped merely because the simulation's `Engine` does not own
/// the minimap widget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyMinimapState {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub go_in: bool,
    pub map_displayed: bool,
    pub transition_counter: f32,
    pub highlight_refresh: u32,
    pub close_after_highlight: bool,
    pub restore: bool,
    pub memory_box: LegacyBoundingBox2,
    pub highlighted_elements: Vec<LegacyMinimapHighlight>,
    pub end_offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyMinimapHighlight {
    pub element: LegacyElementRef,
    pub refresh: bool,
}

impl LegacyMinimapState {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyPostSimpleLimits,
    ) -> LegacyResult<Self> {
        reader.scope("minimap", |reader| {
            audit_abi(abi_profile);
            let start_offset = reader.offset();
            reader.read_signature("fingerprint", FINGERPRINT_MINIMAP, "MD5(\"RHMinimap\")")?;
            let go_in = reader.read_bool("go_in")?;
            let map_displayed = reader.read_bool("map_displayed")?;
            let transition_counter = reader.read_f32("transition_counter")?;
            let highlight_refresh = reader.read_u32("highlight_refresh")?;
            let close_after_highlight = reader.read_bool("close_after_highlight")?;
            // This module is v48-only. The Original condition is version >= 25.
            let restore = reader.read_bool("restore")?;
            let memory_box = read_box2(reader, "memory_box")?;
            let count =
                reader.read_count_u32("highlighted_elements.count", limits.minimap_highlights)?;
            let mut highlighted_elements = Vec::new();
            reserve(
                reader,
                &mut highlighted_elements,
                count,
                "highlighted_elements",
            )?;
            for index in 0..count {
                highlighted_elements.push(reader.scope(
                    format!("highlighted_elements[{index}]"),
                    |reader| {
                        Ok(LegacyMinimapHighlight {
                            element: read_element_ref(reader, "element")?,
                            refresh: reader.read_bool("refresh")?,
                        })
                    },
                )?);
            }
            Ok(Self {
                abi_profile,
                start_offset,
                go_in,
                map_displayed,
                transition_counter,
                highlight_refresh,
                close_after_highlight,
                restore,
                memory_box,
                highlighted_elements,
                end_offset: reader.offset(),
            })
        })
    }
}

/// One of the two consecutive engine PC-selection lists.
///
/// The Original asserts that every resolved entry is non-null and casts it to
/// `RHElementActorPC`. Resolution must reproduce those checks; this byte-level
/// reader cannot validate a reference before phase-one identities are wired.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyElementSelection {
    pub start_offset: u64,
    pub elements: Vec<LegacyElementRef>,
    pub end_offset: u64,
}

impl LegacyElementSelection {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        field: impl Into<String>,
        maximum: usize,
    ) -> LegacyResult<Self> {
        reader.scope(field, |reader| {
            let start_offset = reader.offset();
            let count = reader.read_count_u32("count", maximum)?;
            let mut elements = Vec::new();
            reserve(reader, &mut elements, count, "elements")?;
            for index in 0..count {
                elements.push(read_element_ref(reader, format!("elements[{index}]"))?);
            }
            Ok(Self {
                start_offset,
                elements,
                end_offset: reader.offset(),
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyFollowViewRefs {
    pub start_offset: u64,
    pub follow: LegacyElementRef,
    pub view: LegacyElementRef,
    pub end_offset: u64,
}

impl LegacyFollowViewRefs {
    pub fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        reader.scope("follow_view", |reader| {
            let start_offset = reader.offset();
            Ok(Self {
                start_offset,
                follow: read_element_ref(reader, "follow")?,
                view: read_element_ref(reader, "view")?,
                end_offset: reader.offset(),
            })
        })
    }
}

/// Serializable destination markers owned by `RHGroundMark`.
///
/// TODO(save-import): preserve the current sprite frame exactly. Recreating a
/// marker through the normal API starts its render lifetime at a different
/// frame boundary and can cause a visible replay mismatch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyGroundMarkState {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub marks: Vec<LegacyGroundMark>,
    pub end_offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyGroundMark {
    pub current_sprite_frame: u16,
    pub current_level: u16,
    pub position: LegacyPoint2,
}

impl LegacyGroundMarkState {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyPostSimpleLimits,
    ) -> LegacyResult<Self> {
        reader.scope("ground_mark", |reader| {
            audit_abi(abi_profile);
            let start_offset = reader.offset();
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_GROUND_MARK,
                "MD5(\"RHGroundMark\")",
            )?;
            let count = reader.read_count_u32("marks.count", limits.ground_marks)?;
            let mut marks = Vec::new();
            reserve(reader, &mut marks, count, "marks")?;
            for index in 0..count {
                marks.push(reader.scope(format!("marks[{index}]"), |reader| {
                    Ok(LegacyGroundMark {
                        current_sprite_frame: reader.read_u16("current_sprite_frame")?,
                        current_level: reader.read_u16("current_level")?,
                        position: read_point2(reader, "position")?,
                    })
                })?);
            }
            Ok(Self {
                abi_profile,
                start_offset,
                marks,
                end_offset: reader.offset(),
            })
        })
    }
}

/// Serializable state of `RHTitbits`.
///
/// The two display-order floats are deliberately kept separately. The
/// Original writes the same member twice and, when loading, the second value
/// overwrites the first without checking equality. Preserving both makes
/// malformed or historically divergent files diagnosable.
///
/// TODO(save-import): rebuild the render-owned blinking/dotted counters using
/// the Original's load reset values while retaining every authoritative item,
/// ID, phase, and reference below.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyTitbitsState {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub current_id: u32,
    pub titbits: Vec<LegacyTitbit>,
    pub end_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyTitbit {
    /// Raw 32-bit `RHtitbitKind` written by `CHECKENUM`.
    pub kind: i32,
    pub frame_count: u16,
    pub sprite_frame: u16,
    pub sprite_row: u16,
    pub phase: u16,
    pub display_order_first: f32,
    pub display_order_effective: f32,
    pub layer: u16,
    pub blinking: bool,
    pub id: u32,
    pub element_info_supplier: LegacyElementRef,
    pub element_manager: LegacyElementRef,
    pub position: LegacyPoint3,
}

impl LegacyTitbitsState {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyPostSimpleLimits,
    ) -> LegacyResult<Self> {
        reader.scope("titbits", |reader| {
            audit_abi(abi_profile);
            let start_offset = reader.offset();
            reader.read_signature("fingerprint", FINGERPRINT_TITBITS, "MD5(\"RHTitbits\")")?;
            let current_id = reader.read_u32("current_id")?;
            let count = reader.read_count_u32("items.count", limits.titbits)?;
            let mut titbits = Vec::new();
            reserve(reader, &mut titbits, count, "items")?;
            for index in 0..count {
                titbits.push(reader.scope(format!("items[{index}]"), |reader| {
                    Ok(LegacyTitbit {
                        kind: reader.read_i32("kind")?,
                        frame_count: reader.read_u16("frame_count")?,
                        sprite_frame: reader.read_u16("sprite_frame")?,
                        sprite_row: reader.read_u16("sprite_row")?,
                        phase: reader.read_u16("phase")?,
                        display_order_first: reader.read_f32("display_order_first")?,
                        display_order_effective: reader.read_f32("display_order_effective")?,
                        layer: reader.read_u16("layer")?,
                        blinking: reader.read_bool("blinking")?,
                        id: reader.read_u32("id")?,
                        element_info_supplier: read_element_ref(reader, "element_info_supplier")?,
                        element_manager: read_element_ref(reader, "element_manager")?,
                        position: read_point3(reader, "position")?,
                    })
                })?);
            }
            Ok(Self {
                abi_profile,
                start_offset,
                current_id,
                titbits,
                end_offset: reader.offset(),
            })
        })
    }
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

fn read_box2(
    reader: &mut LegacyReader<'_>,
    field: impl Into<String>,
) -> LegacyResult<LegacyBoundingBox2> {
    reader.scope(field, |reader| {
        Ok(LegacyBoundingBox2 {
            top_left: read_point2(reader, "top_left")?,
            bottom_right: read_point2(reader, "bottom_right")?,
            bounds_are_set: reader.read_bool("bounds_are_set")?,
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

fn read_count_u16(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display + Copy,
    maximum: usize,
) -> LegacyResult<usize> {
    let offset = reader.offset();
    let count = usize::from(reader.read_u16(field)?);
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

fn audit_abi(abi_profile: LegacySaveAbiProfile) {
    debug_assert!(abi_profile.is_little_endian());
    debug_assert_eq!(LegacySaveAbiProfile::BOOL_WIDTH, 1);
    debug_assert_eq!(LegacySaveAbiProfile::WORD_WIDTH, 2);
    debug_assert_eq!(LegacySaveAbiProfile::LONG_WIDTH, 4);
    debug_assert_eq!(LegacySaveAbiProfile::ENUM_WIDTH, 4);
    debug_assert_eq!(LegacySaveAbiProfile::FLOAT_WIDTH, 4);
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::legacy_io::LegacyIoErrorKind;
    use crate::sbfile::{SB_FILE_READ, SbFile};

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut temporary = NamedTempFile::new().unwrap();
        temporary.write_all(bytes).unwrap();
        temporary.flush().unwrap();
        let mut file = SbFile::open(temporary.path().to_str().unwrap(), SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    fn u16_bytes(value: u16, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u32_bytes(value: u32, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i32_bytes(value: i32, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn f32_bytes(value: f32, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn point2_bytes(x: f32, y: f32, bytes: &mut Vec<u8>) {
        f32_bytes(x, bytes);
        f32_bytes(y, bytes);
    }

    fn point3_bytes(x: f32, y: f32, z: f32, bytes: &mut Vec<u8>) {
        f32_bytes(x, bytes);
        f32_bytes(y, bytes);
        f32_bytes(z, bytes);
    }

    fn failed_path_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        u16_bytes(1, &mut bytes);
        i32_bytes(123, &mut bytes);
        bytes.push(1);
        bytes.push(0);
        f32_bytes(2.5, &mut bytes);
        bytes.push(7);
        for value in [8, 9, 10, 11] {
            u16_bytes(value, &mut bytes);
        }
        u32_bytes(12, &mut bytes);
        point2_bytes(13.0, 14.0, &mut bytes);
        point2_bytes(15.0, 16.0, &mut bytes);
        u32_bytes(17, &mut bytes);
        u32_bytes(u32::MAX, &mut bytes);
        u32_bytes(18, &mut bytes);
        bytes
    }

    fn minimap_bytes() -> Vec<u8> {
        let mut bytes = FINGERPRINT_MINIMAP.to_vec();
        bytes.extend_from_slice(&[1, 0]);
        f32_bytes(0.25, &mut bytes);
        u32_bytes(31, &mut bytes);
        bytes.extend_from_slice(&[1, 0]);
        point2_bytes(1.0, 2.0, &mut bytes);
        point2_bytes(3.0, 4.0, &mut bytes);
        bytes.push(1);
        u32_bytes(1, &mut bytes);
        u32_bytes(91, &mut bytes);
        bytes.push(1);
        bytes
    }

    fn ground_mark_bytes() -> Vec<u8> {
        let mut bytes = FINGERPRINT_GROUND_MARK.to_vec();
        u32_bytes(1, &mut bytes);
        u16_bytes(4, &mut bytes);
        u16_bytes(5, &mut bytes);
        point2_bytes(6.0, 7.0, &mut bytes);
        bytes
    }

    fn titbits_bytes() -> Vec<u8> {
        let mut bytes = FINGERPRINT_TITBITS.to_vec();
        u32_bytes(99, &mut bytes);
        u32_bytes(1, &mut bytes);
        i32_bytes(3, &mut bytes);
        for value in [4, 5, 6, 7] {
            u16_bytes(value, &mut bytes);
        }
        f32_bytes(8.0, &mut bytes);
        f32_bytes(9.0, &mut bytes);
        u16_bytes(10, &mut bytes);
        bytes.push(1);
        u32_bytes(11, &mut bytes);
        u32_bytes(12, &mut bytes);
        u32_bytes(u32::MAX, &mut bytes);
        point3_bytes(13.0, 14.0, 15.0, &mut bytes);
        bytes
    }

    #[test]
    fn decodes_failed_paths_identically_for_both_v48_abis() {
        let bytes = failed_path_bytes();
        let limits = LegacyPostSimpleLimits::default();
        let mut decoded = Vec::new();
        for abi in [
            LegacySaveAbiProfile::RetailWindowsX86V48,
            LegacySaveAbiProfile::PortLinuxI386V48,
        ] {
            decoded.push(with_reader(&bytes, |reader| {
                LegacyFailedPathRequests::read(reader, abi, &limits).unwrap()
            }));
        }
        assert_eq!(decoded[0].requests, decoded[1].requests);
        assert_eq!(decoded[0].end_offset, bytes.len() as u64);
        let request = &decoded[0].requests[0];
        assert_eq!(request.action, 123);
        assert_eq!(request.actor, LegacyElementRef(Some(17)));
        assert_eq!(request.antagonist, LegacyElementRef(None));
        assert_eq!(request.sequence_element, LegacySequenceElementRef(Some(18)));
    }

    #[test]
    fn decodes_minimap_selection_and_follow_view_boundaries() {
        let mut bytes = minimap_bytes();
        u32_bytes(2, &mut bytes);
        u32_bytes(101, &mut bytes);
        u32_bytes(102, &mut bytes);
        u32_bytes(1, &mut bytes);
        u32_bytes(103, &mut bytes);
        u32_bytes(104, &mut bytes);
        u32_bytes(u32::MAX, &mut bytes);

        with_reader(&bytes, |reader| {
            let limits = LegacyPostSimpleLimits::default();
            let minimap = LegacyMinimapState::read(
                reader,
                LegacySaveAbiProfile::RetailWindowsX86V48,
                &limits,
            )
            .unwrap();
            assert_eq!(minimap.highlighted_elements[0].element.0, Some(91));
            let selected =
                LegacyElementSelection::read(reader, "selected", limits.selected_elements).unwrap();
            let before_lock = LegacyElementSelection::read(
                reader,
                "selected_before_lock",
                limits.selected_elements,
            )
            .unwrap();
            let refs = LegacyFollowViewRefs::read(reader).unwrap();
            assert_eq!(
                selected.elements,
                vec![LegacyElementRef(Some(101)), LegacyElementRef(Some(102))]
            );
            assert_eq!(before_lock.elements, vec![LegacyElementRef(Some(103))]);
            assert_eq!(refs.follow, LegacyElementRef(Some(104)));
            assert_eq!(refs.view, LegacyElementRef(None));
            assert_eq!(refs.end_offset, bytes.len() as u64);
        });
    }

    #[test]
    fn decodes_ground_marks_and_preserves_both_titbit_display_orders() {
        let mut bytes = ground_mark_bytes();
        bytes.extend_from_slice(&titbits_bytes());
        for abi in [
            LegacySaveAbiProfile::RetailWindowsX86V48,
            LegacySaveAbiProfile::PortLinuxI386V48,
        ] {
            with_reader(&bytes, |reader| {
                let limits = LegacyPostSimpleLimits::default();
                let ground = LegacyGroundMarkState::read(reader, abi, &limits).unwrap();
                let titbits = LegacyTitbitsState::read(reader, abi, &limits).unwrap();
                assert_eq!(ground.marks[0].current_sprite_frame, 4);
                assert_eq!(titbits.current_id, 99);
                assert_eq!(titbits.titbits[0].display_order_first, 8.0);
                assert_eq!(titbits.titbits[0].display_order_effective, 9.0);
                assert_eq!(
                    titbits.titbits[0].element_info_supplier,
                    LegacyElementRef(Some(12))
                );
                assert_eq!(titbits.titbits[0].element_manager, LegacyElementRef(None));
                assert_eq!(titbits.end_offset, bytes.len() as u64);
            });
        }
    }

    #[test]
    fn rejects_bad_fingerprint_and_bounded_counts_before_allocation() {
        let mut bad_minimap = minimap_bytes();
        bad_minimap[0] ^= 0xff;
        let error = with_reader(&bad_minimap, |reader| {
            LegacyMinimapState::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacyPostSimpleLimits::default(),
            )
            .unwrap_err()
        });
        assert_eq!(error.offset, 0);
        assert_eq!(error.field, "minimap.fingerprint");
        assert!(matches!(error.kind, LegacyIoErrorKind::InvalidValue { .. }));

        let bytes = 2_u16.to_le_bytes();
        let error = with_reader(&bytes, |reader| {
            LegacyFailedPathRequests::read(
                reader,
                LegacySaveAbiProfile::RetailWindowsX86V48,
                &LegacyPostSimpleLimits {
                    failed_path_requests: 1,
                    ..LegacyPostSimpleLimits::default()
                },
            )
            .unwrap_err()
        });
        assert_eq!(error.offset, 0);
        assert_eq!(error.field, "failed_path_requests.count");
        assert!(matches!(error.kind, LegacyIoErrorKind::InvalidValue { .. }));
    }

    #[test]
    fn reports_truncated_titbit_at_the_exact_nested_field() {
        let mut bytes = titbits_bytes();
        bytes.truncate(bytes.len() - 2);
        let error = with_reader(&bytes, |reader| {
            LegacyTitbitsState::read(
                reader,
                LegacySaveAbiProfile::PortLinuxI386V48,
                &LegacyPostSimpleLimits::default(),
            )
            .unwrap_err()
        });
        assert_eq!(error.offset, bytes.len() as u64 - 2);
        assert_eq!(error.field, "titbits.items[0].position.z");
        assert!(matches!(error.kind, LegacyIoErrorKind::SbFile { .. }));
    }
}
