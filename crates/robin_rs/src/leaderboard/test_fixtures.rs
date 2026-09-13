//! Shared leaderboard unit-test fixtures.
//!
//! The mission-end, chain-store, receipt-watcher and ranked-session tests all
//! need the same canonical protocol documents (a demo ranked-session config,
//! campaign-chain receipts, fresh-run preflight grants, a signed official
//! session setup). Every builder fixes the values the tests share and takes
//! the values that differ between them as explicit parameters, so a fixture
//! never silently changes what a test asserts.

use crate::leaderboard_ranked_session::{
    OfficialRankedSessionSetupV1, RankedRunPreflightAdmissionV1, public_key, signature,
};
use ed25519_dalek::SigningKey;
use robin_run_protocol::DomainSignedClaim as _;
use robin_run_protocol::{
    ArtifactRefV1, CampaignChainReceiptV1, CampaignChainStateV1, CanonicalDocument as _,
    ChallengeNonce32, Digest32, FreshRunPreflightGrantClaimV1, FreshRunPreflightGrantV1,
    FreshRunPreflightRequestClaimV1, FreshRunPreflightRequestV1, FreshRunScopeV1,
    OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId, ParticipantClaimV1,
    ParticipantPublicDisclosureV1, PublicKey32, RANKED_CAMPAIGN_MEDIA_TYPE_V1,
    RankedSessionConfigV1, ReplaySessionGenesisClaimV1, ReplaySessionGenesisV1,
    ResourceLocaleRootV1, SCHEMA_VERSION_V1, Signature64, SignatureAlgorithmV1, SimulationSeed64,
    SpeechTimingAuthorityV1,
};
use std::collections::BTreeMap;

/// The demo field mission every leaderboard fixture is bound to.
pub(crate) const MISSION_ID: &str = "Dem_Lei_MP";

pub(crate) fn digest(byte: u8) -> Digest32 {
    Digest32::from_bytes([byte; 32])
}

pub(crate) fn signing_key(byte: u8) -> SigningKey {
    SigningKey::from_bytes(&[byte; 32])
}

/// Exact starting-campaign reference for the given encoded campaign bytes.
pub(crate) fn campaign_artifact(campaign_bytes: &[u8]) -> ArtifactRefV1 {
    ArtifactRefV1 {
        sha256: Digest32::digest_bytes(campaign_bytes),
        byte_length: campaign_bytes.len() as u64,
        media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
    }
}

/// The values that differ between the ranked-session config fixtures.
pub(crate) struct RankedConfigSpec {
    pub simulation_seed: u64,
    pub starting_campaign_sha256: Digest32,
    pub starting_campaign_byte_length: u64,
    pub prepared_inputs_projection_sha256: Digest32,
    pub prepared_mission_inputs_seal_sha256: Digest32,
    pub speech_timing: SpeechTimingAuthorityV1,
}

/// Demo-edition ranked session for [`MISSION_ID`] with the shared build /
/// content / rules / ruleset manifest digests `[4..=7]`.
pub(crate) fn ranked_session_config(spec: RankedConfigSpec) -> RankedSessionConfigV1 {
    RankedSessionConfigV1 {
        custom_rules_config: None,
        custom_canonical_campaign: None,
        schema_version: SCHEMA_VERSION_V1,
        mission_id: MISSION_ID.to_owned(),
        content_edition: OfficialContentEditionV1::Demo,
        content_subject: OfficialContentSubjectV1::FieldMission {
            mission_id: MISSION_ID.to_owned(),
        },
        simulation_seed: SimulationSeed64::new(spec.simulation_seed),
        starting_campaign_sha256: spec.starting_campaign_sha256,
        starting_campaign_byte_length: spec.starting_campaign_byte_length,
        prepared_inputs_projection_sha256: spec.prepared_inputs_projection_sha256,
        prepared_mission_inputs_seal_sha256: spec.prepared_mission_inputs_seal_sha256,
        build_manifest_sha256: digest(4),
        content_manifest_sha256: digest(5),
        campaign_content_manifest_sha256: None,
        rules_config_sha256: digest(6),
        ruleset_manifest_sha256: digest(7),
        competition_manifest_sha256: None,
        spellforge_content_sha256: None,
        resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
        speech_timing: spec.speech_timing,
    }
}

/// The values that differ between the campaign-chain receipt fixtures.
pub(crate) struct ChainReceiptSpec {
    pub chain_id: &'static str,
    pub predecessor_run_id: OpaqueId,
    pub expected_starting_campaign: ArtifactRefV1,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub campaign_content_manifest_sha256: Digest32,
    /// Sole participant and campaign controller of the single-player chain.
    pub controller: PublicKey32,
}

/// Active single-player chain receipt (cap 1, no competition) whose only
/// participant is also the campaign controller.
pub(crate) fn chain_receipt(spec: ChainReceiptSpec) -> CampaignChainReceiptV1 {
    CampaignChainReceiptV1 {
        schema_version: SCHEMA_VERSION_V1,
        chain_id: OpaqueId::new(spec.chain_id).unwrap(),
        predecessor_run_id: spec.predecessor_run_id,
        predecessor_verification_sha256: digest(6),
        expected_starting_campaign: spec.expected_starting_campaign,
        rules_config_sha256: spec.rules_config_sha256,
        ruleset_manifest_sha256: spec.ruleset_manifest_sha256,
        competition_manifest_sha256: None,
        campaign_content_manifest_sha256: spec.campaign_content_manifest_sha256,
        expected_max_concurrent_players: 1,
        participant_public_keys: vec![spec.controller],
        campaign_controller_public_key: spec.controller,
        state: CampaignChainStateV1::Active,
    }
}

/// Individual-level fresh-run grant bound to `ranked` and `host_public_key`
/// with a placeholder authority signature. Only for tests that never verify
/// the grant authority; signed grants come from [`official_setup`].
pub(crate) fn unsigned_fresh_run_preflight_grant(
    host_public_key: PublicKey32,
    ranked: &RankedSessionConfigV1,
    starting_campaign: ArtifactRefV1,
) -> FreshRunPreflightGrantV1 {
    FreshRunPreflightGrantV1 {
        claim: FreshRunPreflightGrantClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            grant_id: OpaqueId::new("fresh-grant-1").unwrap(),
            grant_nonce: ChallengeNonce32::from_bytes([21; 32]),
            grant_authority_public_key: PublicKey32::from_bytes([22; 32]),
            host_public_key,
            grant_request_sha256: digest(23),
            ranked_session_sha256: ranked.canonical_digest().unwrap(),
            replay_session_id: digest(11),
            host_participant_instance_id: digest(12),
            host_nonce: ChallengeNonce32::from_bytes([13; 32]),
            scope: FreshRunScopeV1::IndividualLevel,
            starting_campaign,
            admitted_at_unix_ms: 1,
            expires_at_unix_ms: 1_800_000_000_000,
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        authority_signature: Signature64::from_bytes([24; 64]),
    }
}

/// Session genesis with the fixed replay-session ids `[11..=13]` matching
/// [`unsigned_fresh_run_preflight_grant`] and a placeholder host signature.
pub(crate) fn unsigned_session_genesis(
    host_public_key: PublicKey32,
    ranked_session: RankedSessionConfigV1,
    fresh_run_preflight_grant: Option<FreshRunPreflightGrantV1>,
) -> ReplaySessionGenesisV1 {
    ReplaySessionGenesisV1 {
        claim: ReplaySessionGenesisClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            network_protocol_version: robin_engine::multiplayer::NET_PROTOCOL_VERSION,
            host_public_key,
            replay_session_id: digest(11),
            host_participant_instance_id: digest(12),
            host_nonce: ChallengeNonce32::from_bytes([13; 32]),
            ranked_session,
            fresh_run_preflight_grant,
            campaign_continuation_preflight_grant: None,
            competition_run_grant: None,
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        host_signature: Signature64::from_bytes([14; 64]),
    }
}

/// Seat-0 named-profile claim for the host of `genesis`.
pub(crate) fn host_participant_claim(
    genesis: &ReplaySessionGenesisV1,
    public_key: PublicKey32,
) -> ParticipantClaimV1 {
    ParticipantClaimV1 {
        seat: 0,
        participant_instance_id: genesis.claim.host_participant_instance_id,
        public_key,
        public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
        join_attestation: None,
    }
}

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

/// Fully signed official session setup: a host-signed fresh-run preflight
/// request answered by a grant from the fixed authority key `0x7a`.
pub(crate) fn official_setup(
    host_key: &SigningKey,
    ranked_session: RankedSessionConfigV1,
    custom_package_present: bool,
) -> OfficialRankedSessionSetupV1 {
    let authority_key = signing_key(0x7a);
    let request_claim = FreshRunPreflightRequestClaimV1 {
        schema_version: SCHEMA_VERSION_V1,
        request_nonce: ChallengeNonce32::from_bytes([0x31; 32]),
        host_public_key: public_key(host_key),
        replay_session_id: digest(0x32),
        host_participant_instance_id: digest(0x33),
        host_nonce: ChallengeNonce32::from_bytes([0x34; 32]),
        scope: FreshRunScopeV1::IndividualLevel,
        starting_campaign: ArtifactRefV1 {
            sha256: ranked_session.starting_campaign_sha256,
            byte_length: ranked_session.starting_campaign_byte_length,
            media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_string(),
        },
        ranked_session: ranked_session.clone(),
    };
    let request = FreshRunPreflightRequestV1 {
        host_signature: signature(host_key, &request_claim.signing_bytes().unwrap()),
        claim: request_claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
    };
    let grant_claim = FreshRunPreflightGrantClaimV1 {
        schema_version: SCHEMA_VERSION_V1,
        grant_id: OpaqueId::new("fresh-grant-test").unwrap(),
        grant_nonce: ChallengeNonce32::from_bytes([0x35; 32]),
        grant_authority_public_key: public_key(&authority_key),
        host_public_key: public_key(host_key),
        grant_request_sha256: request.canonical_digest().unwrap(),
        ranked_session_sha256: ranked_session.canonical_digest().unwrap(),
        replay_session_id: request.claim.replay_session_id,
        host_participant_instance_id: request.claim.host_participant_instance_id,
        host_nonce: request.claim.host_nonce,
        scope: request.claim.scope,
        starting_campaign: request.claim.starting_campaign.clone(),
        admitted_at_unix_ms: 1_000,
        expires_at_unix_ms: 2_000,
    };
    let grant = FreshRunPreflightGrantV1 {
        authority_signature: signature(&authority_key, &grant_claim.signing_bytes().unwrap()),
        claim: grant_claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
    };
    OfficialRankedSessionSetupV1 {
        ranked_session,
        custom_package_present,
        run_preflight: RankedRunPreflightAdmissionV1::Fresh { request, grant },
        run_preflight_grant_public_key: public_key(&authority_key),
        trusted_now_unix_ms: 1_500,
    }
}
