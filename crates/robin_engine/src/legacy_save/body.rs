//! Strict top-level orchestration for an Original v48 save body.
//!
//! The v48 body is not self-describing. Two campaign streams precede the
//! engine, and several engine subsystems walk mission-created arrays without
//! writing their lengths or script schemas. This module therefore composes the
//! typed section readers in the exact order used by
//! `original-code/RHengine.cpp::RHEngine::Serialize`, while requiring all
//! omitted mission shape from the caller.
//!
//! This is deliberately only a byte-level parse. Resolving legacy references
//! and adopting the result into [`crate::Engine`] is a separate conversion
//! step.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::campaign::{LegacyCampaignLimits, LegacySaveCampaigns};
use super::elements::{LegacyElementEnvelope, LegacyElementReadConfig};
use super::engine::{LegacyEngineLimits, LegacyEnginePreamble};
use super::payload_dispatch::{
    LegacyElementPayloadDecodeContext, LegacyElementPayloadLimits, LegacyElementPayloadStream,
};
use super::post_grid::{
    LegacyFastFindGridState, LegacyGridDecodeContext, LegacyGridLimits, LegacyGridTopology,
};
use super::post_hiking::{
    LegacyHikingGuideDecodeContext, LegacyHikingGuideState, LegacyHikingGuideTopology,
    LegacyPostHikingLimits, LegacyProjectileTrajectorySection,
};
use super::post_sequence_manager::{LegacySequenceManagerLimits, LegacySequenceManagerState};
use super::post_simple::{
    LegacyElementSelection, LegacyFailedPathRequests, LegacyFollowViewRefs, LegacyGroundMarkState,
    LegacyMinimapState, LegacyPostSimpleLimits, LegacyTitbitsState,
};
use super::post_tail::{
    LegacyEnginePostTitbitsTail, LegacyPostTailDecodeContext, LegacyPostTailLimits,
    LegacyPostTailTopology,
};
use super::{LegacySaveAbiProfile, LegacySaveHeader};

/// Independent allocation bounds for every body reader.
///
/// These limits constrain untrusted serialized counts. Mission-sized arrays
/// which do not serialize a count belong in [`LegacySaveBodyTopology`]
/// instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySaveBodyLimits {
    pub campaign: LegacyCampaignLimits,
    pub engine: LegacyEngineLimits,
    pub maximum_elements: usize,
    pub element_payloads: LegacyElementPayloadLimits,
    pub grid: LegacyGridLimits,
    pub hiking: LegacyPostHikingLimits,
    pub simple: LegacyPostSimpleLimits,
    pub sequence_manager: LegacySequenceManagerLimits,
    pub tail: LegacyPostTailLimits,
}

impl Default for LegacySaveBodyLimits {
    fn default() -> Self {
        Self {
            campaign: LegacyCampaignLimits::default(),
            engine: LegacyEngineLimits::default(),
            maximum_elements: 1_000_000,
            element_payloads: LegacyElementPayloadLimits::default(),
            grid: LegacyGridLimits::default(),
            hiking: LegacyPostHikingLimits::default(),
            simple: LegacyPostSimpleLimits::default(),
            sequence_manager: LegacySequenceManagerLimits::default(),
            tail: LegacyPostTailLimits::default(),
        }
    }
}

/// Mission-created shape which v48 does not write into the save stream.
///
/// Every member must be derived from the exact mission initialized for the
/// save header. Supplying a shape from another mission is a structural error,
/// not a compatibility fallback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySaveBodyTopology {
    /// `RHEngine::mulNumberOfCreatedStaticElements` after mission
    /// initialization and before dynamic save elements are reconstructed.
    pub static_creation_order_boundary: u32,
    pub grid: LegacyGridTopology,
    pub hiking_guide: LegacyHikingGuideTopology,
    pub tail: LegacyPostTailTopology,
}

/// Borrowed, SCB-aware callbacks required by the non-self-describing sections.
///
/// The element dispatcher is generic because its existing API requires a
/// sized context. The remaining readers accept independent trait objects so a
/// caller may share one [`super::payload_vm::LegacyVmMemberDecoder`] or use
/// subsystem-specific validation adapters.
pub struct LegacySaveBodyDecodeContext<'a, E: LegacyElementPayloadDecodeContext> {
    pub element_payloads: &'a E,
    pub grid: &'a dyn LegacyGridDecodeContext,
    pub hiking_guide: &'a dyn LegacyHikingGuideDecodeContext,
    pub tail: &'a dyn LegacyPostTailDecodeContext,
}

impl<'a, E: LegacyElementPayloadDecodeContext> LegacySaveBodyDecodeContext<'a, E> {
    pub fn new(
        element_payloads: &'a E,
        grid: &'a dyn LegacyGridDecodeContext,
        hiking_guide: &'a dyn LegacyHikingGuideDecodeContext,
        tail: &'a dyn LegacyPostTailDecodeContext,
    ) -> Self {
        Self {
            element_payloads,
            grid,
            hiking_guide,
            tail,
        }
    }
}

/// The single v48 lock-user byte between `RHFastFindGrid` and `RHHikingGuide`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyUserLockState {
    pub start_offset: u64,
    pub locked: bool,
    pub end_offset: u64,
}

impl LegacyUserLockState {
    fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        reader.scope("user_lock", |reader| {
            let start_offset = reader.offset();
            let locked = reader.read_bool("locked")?;
            Ok(Self {
                start_offset,
                locked,
                end_offset: reader.offset(),
            })
        })
    }
}

/// Complete, lossless typed parse of an Original v48 save body.
///
/// Child structs retain their own start/end offsets. The orchestrator also
/// verifies every adjacent boundary, so a future child-reader regression
/// cannot silently introduce a gap or overlap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySaveBody {
    pub header: LegacySaveHeader,
    pub start_offset: u64,
    pub campaigns: LegacySaveCampaigns,
    pub engine: LegacyEnginePreamble,
    pub element_envelope: LegacyElementEnvelope,
    pub element_payloads: LegacyElementPayloadStream,
    pub grid: LegacyFastFindGridState,
    pub user_lock: LegacyUserLockState,
    pub hiking_guide: LegacyHikingGuideState,
    pub projectile_trajectory: LegacyProjectileTrajectorySection,
    pub failed_path_requests: LegacyFailedPathRequests,
    pub minimap: LegacyMinimapState,
    pub selected_elements: LegacyElementSelection,
    pub selected_before_lock: LegacyElementSelection,
    pub follow_view: LegacyFollowViewRefs,
    pub sequence_manager: LegacySequenceManagerState,
    pub ground_mark: LegacyGroundMarkState,
    pub titbits: LegacyTitbitsState,
    pub tail: LegacyEnginePostTitbitsTail,
    pub end_offset: u64,
}

impl LegacySaveBody {
    /// Parse the body after [`LegacySaveHeader::read`].
    ///
    /// `topology.tail.eof_offset` must be the exact containing file size. The
    /// tail decoder checks that the pending shield reference ends there.
    pub fn read<E: LegacyElementPayloadDecodeContext>(
        reader: &mut LegacyReader<'_>,
        header: LegacySaveHeader,
        limits: &LegacySaveBodyLimits,
        topology: &LegacySaveBodyTopology,
        context: &LegacySaveBodyDecodeContext<'_, E>,
    ) -> LegacyResult<Self> {
        reader.scope("rhsg.body", |reader| {
            let start_offset = reader.offset();
            require_offset(
                reader,
                "body.start_offset",
                start_offset,
                header.body_offset,
                "reader positioned at LegacySaveHeader::body_offset",
            )?;

            let campaigns = LegacySaveCampaigns::read(reader, &limits.campaign)?;
            validate_boundary(
                reader,
                "campaigns.engine",
                campaigns.live.end_offset,
                campaigns.engine_offset,
            )?;

            let engine = LegacyEnginePreamble::read(reader, header.abi_profile, &limits.engine)?;
            validate_boundary(
                reader,
                "campaigns.engine",
                campaigns.engine_offset,
                engine.start_offset,
            )?;

            if topology.static_creation_order_boundary > engine.creation_counter {
                return Err(reader.invalid_value(
                    engine.elements_offset,
                    "static_creation_order_boundary",
                    topology.static_creation_order_boundary,
                    "a mission-created boundary no greater than the saved creation counter",
                ));
            }

            let campaign_character_count = campaigns.live.campaign.characters.len();
            let element_envelope = LegacyElementEnvelope::read_phase1(
                reader,
                &LegacyElementReadConfig {
                    maximum_elements: limits.maximum_elements,
                    campaign_character_count,
                    static_creation_order_boundary: topology.static_creation_order_boundary,
                },
            )?;
            validate_boundary(
                reader,
                "engine.elements",
                engine.elements_offset,
                element_envelope.start_offset,
            )?;

            let element_payloads = LegacyElementPayloadStream::read(
                reader,
                header.abi_profile,
                &element_envelope,
                campaign_character_count,
                &limits.element_payloads,
                context.element_payloads,
            )?;
            validate_boundary(
                reader,
                "elements.phase2",
                element_envelope.phase2_offset,
                element_payloads.start_offset,
            )?;

            let grid = LegacyFastFindGridState::read(
                reader,
                header.abi_profile,
                &topology.grid,
                &limits.grid,
                &limits.element_payloads.base,
                context.grid,
            )?;
            validate_boundary(
                reader,
                "elements.grid",
                element_payloads.end_offset,
                grid.start_offset,
            )?;

            let user_lock = LegacyUserLockState::read(reader)?;
            validate_boundary(
                reader,
                "grid.user_lock",
                grid.end_offset,
                user_lock.start_offset,
            )?;

            let hiking_guide = LegacyHikingGuideState::read(
                reader,
                &topology.hiking_guide,
                &limits.hiking,
                context.hiking_guide,
            )?;
            validate_boundary(
                reader,
                "user_lock.hiking_guide",
                user_lock.end_offset,
                hiking_guide.start_offset,
            )?;

            let projectile_trajectory = LegacyProjectileTrajectorySection::read(
                reader,
                header.abi_profile,
                &limits.hiking,
            )?;
            validate_boundary(
                reader,
                "hiking_guide.projectile_trajectory",
                hiking_guide.end_offset,
                projectile_trajectory.start_offset,
            )?;

            let failed_path_requests =
                LegacyFailedPathRequests::read(reader, header.abi_profile, &limits.simple)?;
            validate_boundary(
                reader,
                "projectile_trajectory.failed_path_requests",
                projectile_trajectory.end_offset,
                failed_path_requests.start_offset,
            )?;

            let minimap = LegacyMinimapState::read(reader, header.abi_profile, &limits.simple)?;
            validate_boundary(
                reader,
                "failed_path_requests.minimap",
                failed_path_requests.end_offset,
                minimap.start_offset,
            )?;

            let selected_elements = LegacyElementSelection::read(
                reader,
                "selected_elements",
                limits.simple.selected_elements,
            )?;
            validate_boundary(
                reader,
                "minimap.selected_elements",
                minimap.end_offset,
                selected_elements.start_offset,
            )?;

            let selected_before_lock = LegacyElementSelection::read(
                reader,
                "selected_before_lock",
                limits.simple.selected_elements,
            )?;
            validate_boundary(
                reader,
                "selected_elements.selected_before_lock",
                selected_elements.end_offset,
                selected_before_lock.start_offset,
            )?;

            let follow_view = LegacyFollowViewRefs::read(reader)?;
            validate_boundary(
                reader,
                "selected_before_lock.follow_view",
                selected_before_lock.end_offset,
                follow_view.start_offset,
            )?;

            let sequence_manager = LegacySequenceManagerState::read(
                reader,
                header.abi_profile,
                &limits.sequence_manager,
            )?;
            validate_boundary(
                reader,
                "follow_view.sequence_manager",
                follow_view.end_offset,
                sequence_manager.start_offset,
            )?;

            let ground_mark =
                LegacyGroundMarkState::read(reader, header.abi_profile, &limits.simple)?;
            validate_boundary(
                reader,
                "sequence_manager.ground_mark",
                sequence_manager.end_offset,
                ground_mark.start_offset,
            )?;

            let titbits = LegacyTitbitsState::read(reader, header.abi_profile, &limits.simple)?;
            validate_boundary(
                reader,
                "ground_mark.titbits",
                ground_mark.end_offset,
                titbits.start_offset,
            )?;

            let tail = LegacyEnginePostTitbitsTail::read(
                reader,
                header.abi_profile,
                &topology.tail,
                &limits.tail,
                context.tail,
            )?;
            validate_boundary(
                reader,
                "titbits.tail",
                titbits.end_offset,
                tail.start_offset,
            )?;

            let end_offset = reader.offset();
            require_offset(
                reader,
                "body.end_offset",
                end_offset,
                tail.end_offset,
                "reader positioned at LegacyEnginePostTitbitsTail::end_offset",
            )?;
            require_offset(
                reader,
                "body.eof_offset",
                end_offset,
                topology.tail.eof_offset,
                "reader positioned at the exact caller-supplied save-stream EOF offset",
            )?;

            Ok(Self {
                header,
                start_offset,
                campaigns,
                engine,
                element_envelope,
                element_payloads,
                grid,
                user_lock,
                hiking_guide,
                projectile_trajectory,
                failed_path_requests,
                minimap,
                selected_elements,
                selected_before_lock,
                follow_view,
                sequence_manager,
                ground_mark,
                titbits,
                tail,
                end_offset,
            })
        })
    }

    pub fn abi_profile(&self) -> LegacySaveAbiProfile {
        self.header.abi_profile
    }
}

fn validate_boundary(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    preceding_end: u64,
    following_start: u64,
) -> LegacyResult<()> {
    if preceding_end == following_start {
        Ok(())
    } else {
        Err(reader.invalid_value(
            following_start,
            field,
            format_args!("preceding end {preceding_end}, following start {following_start}"),
            "identical adjacent section boundary offsets",
        ))
    }
}

fn require_offset(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    actual: u64,
    expected: u64,
    expected_description: &'static str,
) -> LegacyResult<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(reader.invalid_value(actual, field, actual, expected_description))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::sbfile::{SB_FILE_READ, SbFile};

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut temporary = NamedTempFile::new().unwrap();
        temporary.write_all(bytes).unwrap();
        temporary.flush().unwrap();
        let path = temporary.path().to_str().unwrap();
        let mut file = SbFile::open(path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    #[test]
    fn user_lock_consumes_exactly_one_byte_and_preserves_nonzero_truth() {
        with_reader(&[0x7f, 0xaa], |reader| {
            let state = LegacyUserLockState::read(reader).unwrap();
            assert_eq!(
                state,
                LegacyUserLockState {
                    start_offset: 0,
                    locked: true,
                    end_offset: 1,
                }
            );
            assert_eq!(reader.read_u8("next_section").unwrap(), 0xaa);
        });
    }

    #[test]
    fn adjacent_boundary_validation_rejects_a_gap() {
        with_reader(&[], |reader| {
            let error = validate_boundary(reader, "left.right", 10, 11).unwrap_err();
            assert_eq!(error.offset, 11);
            assert_eq!(error.field, "left.right");
        });
    }
}
