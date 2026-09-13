//! Strict readers for the self-describing v48 engine sections after the
//! engine-owned projectile trajectory.
//!
//! This module intentionally does not orchestrate the complete engine tail:
//! Sequence-manager data is serialized between follow/view references and
//! the ground mark. Each reader preserves its own byte range so the eventual
//! top-level importer can compose these sections without guessing boundaries.
//!
//! Wire order follows the original game's engine, minimap, ground-mark, and
//! titbit serialization. Field declaration order is wire order.

use super::read_helpers::DEFAULT_BULK_LIMIT;
use super::read_helpers::hex16;
use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyContext, LegacyRead, LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::payload_base::{
    LegacyBoundingBox2, LegacyElementRef, LegacyPoint2, LegacyPoint3, LegacySequenceElementRef,
};

const FINGERPRINT_MINIMAP: [u8; 16] = hex16("50f6249a4ee7522862f2c5f5442ae167");
const FINGERPRINT_GROUND_MARK: [u8; 16] = hex16("b7ebd8adf1c9be532ca495049f430da9");
const FINGERPRINT_TITBITS: [u8; 16] = hex16("0066cad32f8281aebfc9aba90a88aa34");

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
            failed_path_requests: DEFAULT_BULK_LIMIT,
            minimap_highlights: DEFAULT_BULK_LIMIT,
            selected_elements: DEFAULT_BULK_LIMIT,
            ground_marks: DEFAULT_BULK_LIMIT,
            titbits: DEFAULT_BULK_LIMIT,
        }
    }
}

/// Decode context shared by the post-simple sections.
#[derive(Clone, Copy)]
pub struct LegacyPostSimpleDecode<'a> {
    pub abi_profile: LegacySaveAbiProfile,
    pub limits: &'a LegacyPostSimpleLimits,
}

/// engine failed-path-request list.
///
/// [`super::adopt_paths`] must restore the authoritative queue
/// rather than synthesize a new path request from only actor and destination.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyPostSimpleDecode<'_>)]
pub struct LegacyFailedPathRequests {
    #[legacy(value = ctx.abi_profile)]
    pub abi_profile: LegacySaveAbiProfile,
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(count_u16 = ctx.limits.failed_path_requests, count_name = "count")]
    pub requests: Vec<LegacyFailedPathRequest>,
    #[legacy(offset)]
    pub end_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyFailedPathRequest {
    /// Raw 32-bit animation value written by enum serialization.
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
        Self::read_field(
            reader,
            "failed_path_requests",
            &LegacyPostSimpleDecode {
                abi_profile,
                limits,
            },
        )
    }
}

/// Serializable state of the UI-owned minimap.
///
/// [`super::adopt_simple`] returns this state to the host UI layer; it must not
/// be dropped merely because the simulation does not own the minimap widget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyPostSimpleDecode<'_>)]
pub struct LegacyMinimapState {
    #[legacy(value = ctx.abi_profile)]
    pub abi_profile: LegacySaveAbiProfile,
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(fingerprint = FINGERPRINT_MINIMAP, expected = "minimap fingerprint")]
    pub go_in: bool,
    pub map_displayed: bool,
    pub transition_counter: f32,
    pub highlight_refresh: u32,
    pub close_after_highlight: bool,
    /// This module is v48-only. The Original condition is version >= 25.
    pub restore: bool,
    pub memory_box: LegacyBoundingBox2,
    #[legacy(count_u32 = ctx.limits.minimap_highlights)]
    pub highlighted_elements: Vec<LegacyMinimapHighlight>,
    #[legacy(offset)]
    pub end_offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
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
        Self::read_field(
            reader,
            "minimap",
            &LegacyPostSimpleDecode {
                abi_profile,
                limits,
            },
        )
    }
}

/// One of the two consecutive engine PC-selection lists.
///
/// Every resolved entry must be a non-null player actor.
/// Resolution must enforce those requirements; this byte-level
/// reader cannot validate a reference before phase-one identities are wired.
///
/// Context: the maximum element count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = usize)]
pub struct LegacyElementSelection {
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(count_u32 = *ctx, count_name = "count")]
    pub elements: Vec<LegacyElementRef>,
    #[legacy(offset)]
    pub end_offset: u64,
}

impl LegacyElementSelection {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        field: impl Into<LegacyContext>,
        maximum: usize,
    ) -> LegacyResult<Self> {
        Self::read_field(reader, field, &maximum)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyFollowViewRefs {
    #[legacy(offset)]
    pub start_offset: u64,
    pub follow: LegacyElementRef,
    pub view: LegacyElementRef,
    #[legacy(offset)]
    pub end_offset: u64,
}

impl LegacyFollowViewRefs {
    pub fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        Self::read_field(reader, "follow_view", &())
    }
}

/// Serializable destination markers owned by the ground mark.
///
/// [`super::adopt_simple`] preserves the current sprite frame exactly. Recreating a
/// marker through the normal API starts its render lifetime at a different
/// frame boundary and can cause a visible replay mismatch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyPostSimpleDecode<'_>)]
pub struct LegacyGroundMarkState {
    #[legacy(value = ctx.abi_profile)]
    pub abi_profile: LegacySaveAbiProfile,
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(
        fingerprint = FINGERPRINT_GROUND_MARK,
        expected = "ground-mark fingerprint",
        count_u32 = ctx.limits.ground_marks
    )]
    pub marks: Vec<LegacyGroundMark>,
    #[legacy(offset)]
    pub end_offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
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
        Self::read_field(
            reader,
            "ground_mark",
            &LegacyPostSimpleDecode {
                abi_profile,
                limits,
            },
        )
    }
}

/// Serializable state of the titbit manager.
///
/// The two display-order floats are deliberately kept separately. The
/// The original game writes the same value twice and, when loading, the second value
/// overwrites the first without checking equality. Preserving both makes
/// malformed or historically divergent files diagnosable.
///
/// [`super::adopt_simple`] rebuilds the render-owned blinking/dotted counters using
/// the Original's load reset values while retaining every authoritative item,
/// ID, phase, and reference below.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = LegacyPostSimpleDecode<'_>)]
pub struct LegacyTitbitsState {
    #[legacy(value = ctx.abi_profile)]
    pub abi_profile: LegacySaveAbiProfile,
    #[legacy(offset)]
    pub start_offset: u64,
    #[legacy(fingerprint = FINGERPRINT_TITBITS, expected = "hint-manager fingerprint")]
    pub current_id: u32,
    #[legacy(name = "items", count_u32 = ctx.limits.titbits)]
    pub titbits: Vec<LegacyTitbit>,
    #[legacy(offset)]
    pub end_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyTitbit {
    /// Raw 32-bit titbit-kind value written by enum serialization.
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
        Self::read_field(
            reader,
            "titbits",
            &LegacyPostSimpleDecode {
                abi_profile,
                limits,
            },
        )
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::legacy_io::LegacyIoErrorKind;

    use crate::legacy_save::test_support::with_reader;

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
        assert!(matches!(error.kind, LegacyIoErrorKind::SbFile(_)));
    }
}
