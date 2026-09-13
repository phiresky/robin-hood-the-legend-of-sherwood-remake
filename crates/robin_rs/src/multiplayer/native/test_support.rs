//! Fixtures shared by the native server and client test modules.
use robin_engine::multiplayer::LeaderboardCoSignResponse;
use robin_run_protocol::{
    Digest32, LeaderboardCoSignInstanceV1, LeaderboardCoSignPurposeV1, LeaderboardCoSignRequestV1,
};

pub(super) fn leaderboard_request(
    purpose: LeaderboardCoSignPurposeV1,
    byte: u8,
) -> LeaderboardCoSignRequestV1 {
    LeaderboardCoSignRequestV1 {
        instance: LeaderboardCoSignInstanceV1 {
            purpose,
            replay_session_id: Digest32::from_bytes([byte; 32]),
            submission_offer_sha256: Digest32::from_bytes([byte.wrapping_add(1); 32]),
        },
        run_digest: Digest32::from_bytes([byte.wrapping_add(2); 32]),
    }
}

pub(super) fn signed_response(
    request: &LeaderboardCoSignRequestV1,
    key: &iroh::SecretKey,
) -> LeaderboardCoSignResponse {
    LeaderboardCoSignResponse {
        instance: request.instance,
        signer_public_key: *key.public().as_bytes(),
        signature: key.sign(&request.signing_bytes().unwrap()).to_bytes(),
    }
}
