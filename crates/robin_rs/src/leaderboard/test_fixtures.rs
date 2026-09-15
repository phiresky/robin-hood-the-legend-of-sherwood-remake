//! Shared leaderboard unit-test fixtures.
//!
//! Every builder fixes the values the tests share and takes the values that
//! differ between them as explicit parameters, so a fixture never silently
//! changes what a test asserts.

use robin_run_protocol::{
    ArtifactRefV1, BoardMetricV1, BoardMissionV2, BoardSimulationPolicyV1, BoardV2,
    ChallengeNonce32, Digest32, OfficialContentEditionV1, OpaqueId, ParticipantPublicDisclosureV1,
    PublicKey32, RANKED_REPLAY_MEDIA_TYPE_V1, RankedSimulationDifficultyV1,
    RankedSimulationPolicyV1, ReplayArtifactV1, SCHEMA_VERSION_V1, SCHEMA_VERSION_V2, SubmissionV2,
    UploadChallengeV1, ViewerContentRequirementV2,
};
use std::collections::BTreeMap;

/// The demo field mission every leaderboard fixture is bound to.
pub(crate) const MISSION_ID: &str = "Dem_Lei_MP";

/// Rankable single-frame built-in replay of [`MISSION_ID`] with seed 42 and
/// no input, starting from `campaign_bytes`.
pub(crate) fn single_frame_replay(campaign_bytes: Vec<u8>) -> robin_engine::replay::ReplayData {
    let replay = robin_engine::replay::ReplayData::try_from(robin_engine::replay::ReplayFile {
        header: robin_engine::replay::ReplayHeader {
            mission_id: MISSION_ID.to_owned(),
            mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                MISSION_ID, MISSION_ID, MISSION_ID,
            )
            .expect("valid built-in leaderboard test descriptor"),
            rng_seed: 42,
            sim_config: robin_engine::engine::SimConfig::default(),
            spellforge_package: None,
            version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
            total_frames: 1,
            rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
            campaign: campaign_bytes,
        },
        frames: BTreeMap::from([(
            0,
            robin_engine::replay::ReplayFrame {
                timeline_before: 0,
                timeline_after: 0,
                input: robin_engine::engine::SimulationFrameInput::default(),
                host_controls: Vec::new(),
            },
        )]),
        hashes: BTreeMap::new(),
        save_markers: BTreeMap::new(),
        load_backs: BTreeMap::new(),
    });
    replay.expect("valid single-frame leaderboard replay fixture")
}

/// Compact canonical bytes of [`single_frame_replay`] with a default campaign.
pub(crate) fn compact_replay_bytes() -> Vec<u8> {
    let replay = single_frame_replay(bitcode::encode(&robin_engine::campaign::Campaign::default()));
    robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
        .expect("fixture replay encodes")
        .into_bytes()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn replay_artifact(bytes: &[u8]) -> ReplayArtifactV1 {
    ReplayArtifactV1 {
        artifact: ArtifactRefV1 {
            sha256: Digest32::digest_bytes(bytes),
            byte_length: bytes.len() as u64,
            media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
        },
        replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
    }
}

/// A board of `edition` over [`MISSION_ID`] with both metrics.
pub(crate) fn board(
    board_id: &str,
    edition: OfficialContentEditionV1,
    simulation_policy: BoardSimulationPolicyV1,
) -> BoardV2 {
    BoardV2 {
        board_id: OpaqueId::new(board_id).unwrap(),
        display_name: board_id.to_owned(),
        edition,
        preset_id: "standard".to_owned(),
        preset_name: "Standard".to_owned(),
        difficulty_id: "normal".to_owned(),
        difficulty_name: "Normal".to_owned(),
        simulation_policy,
        allow_state_load: true,
        metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
        viewer_content_requirement: ViewerContentRequirementV2::BundledDemo,
        missions: vec![BoardMissionV2 {
            mission_id: MISSION_ID.to_owned(),
            display_name: "Leicester".to_owned(),
        }],
    }
}

pub(crate) fn standard_medium_policy() -> BoardSimulationPolicyV1 {
    BoardSimulationPolicyV1::Fixed {
        policy: RankedSimulationPolicyV1::standard(RankedSimulationDifficultyV1::Medium),
    }
}

/// A complete, valid V2 submission of the fixture replay by `uploader`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn submission(uploader: PublicKey32) -> SubmissionV2 {
    SubmissionV2 {
        schema_version: SCHEMA_VERSION_V2,
        upload_challenge: UploadChallengeV1 {
            schema_version: SCHEMA_VERSION_V1,
            upload_challenge_id: OpaqueId::new("upload-1").unwrap(),
            upload_challenge_nonce: ChallengeNonce32::from_bytes([2; 32]),
            expires_at_unix_ms: 1_800_000_000_000,
        },
        uploader_public_key: uploader,
        public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
        board_id: OpaqueId::new("demo-standard-normal").unwrap(),
        mission_id: MISSION_ID.to_owned(),
        replay: replay_artifact(&compact_replay_bytes()),
        requested_metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
    }
}
