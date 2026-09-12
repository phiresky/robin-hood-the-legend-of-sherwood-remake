//! Explicit fixture parameters preserve each suite's intentional identities.
use crate::*;

pub(crate) fn id(value: &str) -> OpaqueId {
    OpaqueId::new(value).unwrap()
}
pub(crate) fn campaign_artifact(byte: u8, byte_length: u64) -> ArtifactRefV1 {
    ArtifactRefV1 {
        sha256: Digest32::from_bytes([byte; 32]),
        byte_length,
        media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
    }
}
pub(crate) fn replay_artifact(byte: u8, byte_length: u64) -> ReplayArtifactV1 {
    ReplayArtifactV1 {
        artifact: ArtifactRefV1 {
            sha256: Digest32::from_bytes([byte; 32]),
            byte_length,
            media_type: RANKED_REPLAY_MEDIA_TYPE_V1.into(),
        },
        replay_schema_version: CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
    }
}
pub(crate) fn submission_artifacts(
    replay: ReplayArtifactV1,
    starting_campaign: ArtifactRefV1,
) -> SubmissionArtifactsV1 {
    SubmissionArtifactsV1 {
        replay,
        starting_campaign,
    }
}
pub(crate) fn host_claim(public_key_byte: u8) -> ParticipantClaimV1 {
    ParticipantClaimV1 {
        seat: 0,
        participant_instance_id: Digest32::from_bytes([12; 32]),
        public_key: PublicKey32::from_bytes([public_key_byte; 32]),
        public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
        join_attestation: None,
    }
}
pub(crate) fn fresh_preflight_grant(
    ranked: &RankedSessionConfigV1,
    scope: FreshRunScopeV1,
    nonce_base: u8,
    host_byte: u8,
    session_byte: u8,
    host_nonce_byte: u8,
    starting_campaign: ArtifactRefV1,
    admitted_at_unix_ms: u64,
    expires_at_unix_ms: u64,
) -> FreshRunPreflightGrantV1 {
    FreshRunPreflightGrantV1 {
        claim: FreshRunPreflightGrantClaimV1 {
            schema_version: 1,
            grant_id: id("fresh-grant-1"),
            grant_nonce: ChallengeNonce32::from_bytes([nonce_base; 32]),
            grant_authority_public_key: PublicKey32::from_bytes([nonce_base + 1; 32]),
            host_public_key: PublicKey32::from_bytes([host_byte; 32]),
            grant_request_sha256: Digest32::from_bytes([nonce_base + 2; 32]),
            ranked_session_sha256: ranked.canonical_digest().unwrap(),
            replay_session_id: Digest32::from_bytes([session_byte; 32]),
            host_participant_instance_id: Digest32::from_bytes([12; 32]),
            host_nonce: ChallengeNonce32::from_bytes([host_nonce_byte; 32]),
            scope,
            starting_campaign,
            admitted_at_unix_ms,
            expires_at_unix_ms,
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        authority_signature: Signature64::from_bytes([nonce_base + 3; 64]),
    }
}

#[test]
fn every_signature_domain_is_distinct_and_terminated() {
    let domains = [
        crate::envelope::SUBMISSION_SIGNATURE_DOMAIN_V1,
        crate::envelope::LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1,
        crate::envelope::USERNAME_UPDATE_SIGNATURE_DOMAIN_V1,
        crate::envelope::REPLAY_SESSION_GENESIS_SIGNATURE_DOMAIN_V1,
        crate::envelope::NAMED_SEAT_JOIN_SIGNATURE_DOMAIN_V1,
        crate::envelope::CAMPAIGN_CONTINUATION_SIGNATURE_DOMAIN_V1,
        crate::envelope::COMPETITION_RUN_GRANT_REQUEST_SIGNATURE_DOMAIN_V1,
        crate::envelope::COMPETITION_RUN_GRANT_SIGNATURE_DOMAIN_V1,
        crate::envelope::FRESH_RUN_PREFLIGHT_REQUEST_SIGNATURE_DOMAIN_V1,
        crate::envelope::FRESH_RUN_PREFLIGHT_GRANT_SIGNATURE_DOMAIN_V1,
        crate::envelope::CAMPAIGN_CONTINUATION_PREFLIGHT_HOST_SIGNATURE_DOMAIN_V1,
        crate::envelope::CAMPAIGN_CONTINUATION_PREFLIGHT_CONTROLLER_SIGNATURE_DOMAIN_V1,
        crate::envelope::CAMPAIGN_CONTINUATION_PREFLIGHT_GRANT_SIGNATURE_DOMAIN_V1,
        crate::moderation::DELETION_REQUEST_SIGNATURE_DOMAIN_V1,
        crate::query::SUBMISSION_OWNER_STATUS_SIGNATURE_DOMAIN_V1,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for domain in domains {
        assert_eq!(domain.last(), Some(&0));
        assert!(seen.insert(domain), "duplicate signature domain");
    }
}
