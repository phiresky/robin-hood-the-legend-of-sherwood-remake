//! Decode an Original v48 save against an initialized Rust mission.
//!
//! The wire format omits mission-created array sizes and VM schemas.  This
//! entry point derives those facts from the attached [`LevelAssets`] and
//! mission [`EngineInner`], then drives the strict body decoder to EOF.

use thiserror::Error;

use crate::{
    engine::{EngineInner, LevelAssets},
    legacy_io::{LegacyIoError, LegacyReader},
    sbfile::SbFile,
    scb::ScbFile,
};

use super::{
    LegacySaveHeader,
    body::{
        LegacySaveBody, LegacySaveBodyDecodeContext, LegacySaveBodyLimits, LegacySaveBodyTopology,
    },
    payload_context::LegacyMissionPayloadDecodeContext,
    payload_vm::LegacyVmMemberDecoder,
    topology_adapter::{
        LegacyTopologyAdapterError, derive_grid_topology, derive_hiking_guide_topology,
        derive_post_tail_topology, derive_static_element_topology,
    },
};

#[derive(Debug, Error)]
pub enum LegacyInitializedSaveDecodeError {
    #[error(transparent)]
    Io(#[from] LegacyIoError),
    #[error(transparent)]
    Topology(#[from] LegacyTopologyAdapterError),
    #[error(
        "Original save mission profile id {save_mission_id} does not match initialized mission profile id {initialized_mission_id}"
    )]
    MissionMismatch {
        save_mission_id: u32,
        initialized_mission_id: u32,
    },
    #[error("initialized engine has no current campaign mission")]
    MissingCurrentMission,
    #[error("initialized campaign current mission index {index} is out of range")]
    CurrentMissionOutOfRange { index: usize },
    #[error("initialized campaign mission has no profile index")]
    MissingMissionProfileIndex,
    #[error("initialized mission profile index {index} is out of range")]
    MissionProfileOutOfRange { index: usize },
}

/// Parse owned Original v48 save bytes using the exact initialized mission.
///
/// This is intentionally a lossless parse, not state adoption. The returned
/// body retains Original creation-order references and byte offsets for the
/// subsequent conversion/fixup pass.
pub fn decode_initialized_v48_save(
    bytes: Vec<u8>,
    display_path: impl Into<String>,
    engine: &EngineInner,
    assets: &LevelAssets,
    scb: &ScbFile,
    limits: &LegacySaveBodyLimits,
) -> Result<LegacySaveBody, LegacyInitializedSaveDecodeError> {
    let eof_offset = u64::try_from(bytes.len()).expect("usize always fits u64");
    let mut file = SbFile::from_owned_bytes(bytes, display_path);
    let mut reader = LegacyReader::new(&mut file);
    let header = LegacySaveHeader::read(&mut reader)?;
    validate_initialized_mission(header, engine, assets)?;

    let static_elements = derive_static_element_topology(engine, assets)?;
    let topology = LegacySaveBodyTopology {
        static_creation_order_boundary: static_elements.static_creation_order_boundary,
        grid: derive_grid_topology(engine, assets)?,
        hiking_guide: derive_hiking_guide_topology(engine, assets)?,
        tail: derive_post_tail_topology(engine, assets, eof_offset)?,
    };
    let element_context = LegacyMissionPayloadDecodeContext::with_default_limits(
        scb,
        &static_elements.payload_metadata,
    );
    let vm_context = LegacyVmMemberDecoder::with_default_limits(scb);
    let context =
        LegacySaveBodyDecodeContext::new(&element_context, &vm_context, &vm_context, &vm_context);

    Ok(LegacySaveBody::read(
        &mut reader,
        header,
        limits,
        &topology,
        &context,
    )?)
}

fn validate_initialized_mission(
    header: LegacySaveHeader,
    engine: &EngineInner,
    assets: &LevelAssets,
) -> Result<(), LegacyInitializedSaveDecodeError> {
    let campaign = engine.campaign();
    let mission_index = campaign
        .current_mission_idx
        .ok_or(LegacyInitializedSaveDecodeError::MissingCurrentMission)?;
    let mission = campaign.missions.get(mission_index).ok_or(
        LegacyInitializedSaveDecodeError::CurrentMissionOutOfRange {
            index: mission_index,
        },
    )?;
    let profile_index = mission
        .profile_idx
        .ok_or(LegacyInitializedSaveDecodeError::MissingMissionProfileIndex)?;
    let profile_index = usize::try_from(profile_index).expect("u32 always fits usize");
    let mission_profile = assets.profile_manager.missions.get(profile_index).ok_or(
        LegacyInitializedSaveDecodeError::MissionProfileOutOfRange {
            index: profile_index,
        },
    )?;
    if header.mission_id != mission_profile.id {
        return Err(LegacyInitializedSaveDecodeError::MissionMismatch {
            save_mission_id: header.mission_id,
            initialized_mission_id: mission_profile.id,
        });
    }
    Ok(())
}
