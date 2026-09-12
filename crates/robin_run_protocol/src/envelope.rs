use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    AnonymousParticipantPolicyV1, ArtifactRefV1, BoardMetricV1, CampaignContentManifestV1,
    CampaignRosterContinuityV1, CanonicalDocument as _, CanonicalValue, ChallengeNonce32, Digest32,
    OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId, PublicKey32, PublishedRulesetV1,
    ResourceLocaleRootV1, RulesetBoardScopeV1, RunMetricsV1, Signature64, SimulationSeed64,
    Validate, ValidationError,
};
#[cfg(test)]
use crate::{ContentManifestV1, SimulationSpeechTimingSourceV1};

pub const SUBMISSION_SIGNATURE_DOMAIN_V1: &[u8] = b"robinhood/leaderboards/1/submission\0";
/// Domain for the one fixed-size payload accepted by the leaderboard identity
/// key for participant and campaign-controller co-signatures.
pub const LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/co-sign-payload\0";
pub const USERNAME_UPDATE_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/username-update\0";
pub const REPLAY_SESSION_GENESIS_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/multiplayer/1/session-genesis\0";
pub const NAMED_SEAT_JOIN_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/multiplayer/1/join-attestation\0";
pub const CAMPAIGN_CONTINUATION_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/campaign-continuation\0";
/// The payload consists of this domain, a one-byte purpose tag, the replay
/// session digest, the authoritative submission-offer digest, and the exact
/// purpose-specific document digest.
pub const LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1: usize =
    LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1.len() + 1 + Digest32::LENGTH * 3;
pub const COMPETITION_RUN_GRANT_REQUEST_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/competition-run-grant-request\0";
pub const COMPETITION_RUN_GRANT_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/competition-run-grant\0";
pub const FRESH_RUN_PREFLIGHT_REQUEST_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/fresh-run-preflight-request\0";
pub const FRESH_RUN_PREFLIGHT_GRANT_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/fresh-run-preflight-grant\0";
pub const CAMPAIGN_CONTINUATION_PREFLIGHT_HOST_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/campaign-continuation-preflight-host\0";
pub const CAMPAIGN_CONTINUATION_PREFLIGHT_CONTROLLER_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/campaign-continuation-preflight-controller\0";
pub const CAMPAIGN_CONTINUATION_PREFLIGHT_GRANT_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/campaign-continuation-preflight-grant\0";
pub const MAX_REPLAY_SEATS_V1: u16 = 4;
pub const MAX_PARTICIPANT_INSTANCES_V1: u16 = 1_024;
pub const MAX_CAMPAIGN_SESSIONS_V1: u32 = 4_096;
/// Exact media type for the bitcode campaign artifact consumed by the ranked
/// Engine.
pub const RANKED_CAMPAIGN_MEDIA_TYPE_V1: &str = "application/x-robin-campaign+bitcode";
/// The one wire/storage format accepted by the current ranked schema. JSONL and older
/// Rust replay containers are local developer formats, not protocol lanes.
pub const RANKED_REPLAY_MEDIA_TYPE_V1: &str = "application/x-robin-rhrec+compact";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadChallengeV1 {
    pub schema_version: u32,
    pub upload_challenge_id: OpaqueId,
    pub upload_challenge_nonce: ChallengeNonce32,
    pub expires_at_unix_ms: u64,
}

impl Validate for UploadChallengeV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("UploadChallengeV1", self.schema_version)?;
        crate::validation::nonzero(
            "upload_challenge.upload_challenge_nonce",
            &self.upload_challenge_nonce,
        )?;
        if self.expires_at_unix_ms == 0 {
            return Err(ValidationError::Zero {
                field: "upload_challenge.expires_at_unix_ms",
            });
        }
        Ok(())
    }
}

/// A one-use challenge dedicated to a mutable username update.
///
/// It is intentionally a different namespace from replay upload challenges,
/// preventing a challenge minted for one operation from authorizing the other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsernameChallengeV1 {
    pub schema_version: u32,
    pub username_challenge_id: OpaqueId,
    pub username_challenge_nonce: ChallengeNonce32,
    pub expires_at_unix_ms: u64,
}

/// Request for a one-use username-update challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsernameChallengeRequestV1 {
    pub schema_version: u32,
    pub public_key: PublicKey32,
}

impl Validate for UsernameChallengeRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("UsernameChallengeRequestV1", self.schema_version)?;
        crate::validation::nonzero("username_challenge_request.public_key", &self.public_key)?;
        Ok(())
    }
}

impl Validate for UsernameChallengeV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("UsernameChallengeV1", self.schema_version)?;
        crate::validation::nonzero(
            "username_challenge.username_challenge_nonce",
            &self.username_challenge_nonce,
        )?;
        if self.expires_at_unix_ms == 0 {
            return Err(ValidationError::Zero {
                field: "username_challenge.expires_at_unix_ms",
            });
        }
        Ok(())
    }
}

mod session;
pub use session::{
    NamedSeatJoinAttestationV1, NamedSeatJoinClaimV1, RankedSessionConfigV1,
    ReplaySeatLifecycleEventV1, ReplaySeatLifecycleKindV1, ReplaySessionGenesisClaimV1,
    ReplaySessionGenesisV1, ReplaySessionTranscriptV1, SpeechTimingAuthorityV1,
};
/// Canonical seal over the exact run-specific inputs consumed by the engine.
/// The static content manifest is prepublished; `prepared_inputs_projection`
/// additionally binds the mutable team/inventory/reinforcement closure
/// derived from the starting campaign. Ranked verification recomputes this
/// document before consuming the engine's single-use prepared capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedMissionInputsSealV1 {
    pub schema_version: u32,
    pub prepared_inputs_projection_sha256: Digest32,
    pub content_manifest_sha256: Digest32,
    pub content_edition: OfficialContentEditionV1,
    pub content_subject: OfficialContentSubjectV1,
    pub starting_campaign_sha256: Digest32,
    pub starting_campaign_byte_length: u64,
    pub simulation_seed: SimulationSeed64,
    pub rules_config_sha256: Digest32,
    pub resource_locale_root: ResourceLocaleRootV1,
    pub speech_timing: SpeechTimingAuthorityV1,
    /// Reserved for explicitly unranked Spellforge simulations. Official
    /// ranked seals reject it until immutable policy semantics exist.
    pub spellforge_content_sha256: Option<Digest32>,
    /// Original-parity RNG streams are replay inputs, not ranked RNG. Their
    /// presence makes the seal unrankable.
    pub original_rng_replay_sha256: Option<Digest32>,
}

/// Host-authored request frozen before the first ranked simulation frame.
/// The random request nonce makes retries explicit; the remaining fields bind
/// the server authorization to one exact replay session and engine input set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitionRunGrantRequestClaimV1 {
    pub schema_version: u32,
    pub request_nonce: ChallengeNonce32,
    pub host_public_key: PublicKey32,
    pub replay_session_id: Digest32,
    pub host_participant_instance_id: Digest32,
    pub host_nonce: ChallengeNonce32,
    pub ranked_session: RankedSessionConfigV1,
}

impl CompetitionRunGrantRequestClaimV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        <Self as crate::canonical::DomainSignedClaim>::signing_bytes(self)
    }
}

impl crate::canonical::DomainSignedClaim for CompetitionRunGrantRequestClaimV1 {
    const DOMAIN: &'static [u8] = COMPETITION_RUN_GRANT_REQUEST_SIGNATURE_DOMAIN_V1;
}

impl Validate for CompetitionRunGrantRequestClaimV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("CompetitionRunGrantRequestClaimV1", self.schema_version)?;
        if self.request_nonce.is_zero()
            || self.host_public_key.is_zero()
            || self.replay_session_id.is_zero()
            || self.host_participant_instance_id.is_zero()
            || self.host_nonce.is_zero()
        {
            return Err(ValidationError::Zero {
                field: "competition_run_grant_request.identity",
            });
        }
        self.ranked_session.validate()?;
        if self.ranked_session.competition_manifest_sha256.is_none() {
            return Err(ValidationError::ClaimMismatch {
                field: "competition_run_grant_request.competition_manifest_sha256",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitionRunGrantRequestV1 {
    pub claim: CompetitionRunGrantRequestClaimV1,
    pub algorithm: SignatureAlgorithmV1,
    pub host_signature: Signature64,
}

impl CompetitionRunGrantRequestV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        self.claim.signing_bytes()
    }
}

impl Validate for CompetitionRunGrantRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.claim.validate()?;
        crate::validation::nonzero(
            "competition_run_grant_request.host_signature",
            &self.host_signature,
        )?;
        Ok(())
    }
}

/// Server-authored, one-use authorization for one exact scheduled run. The
/// service is the sole time authority; clients cannot supply admission times.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitionRunGrantClaimV1 {
    pub schema_version: u32,
    pub grant_id: OpaqueId,
    pub grant_nonce: ChallengeNonce32,
    pub grant_authority_public_key: PublicKey32,
    pub host_public_key: PublicKey32,
    pub competition_manifest_sha256: Digest32,
    pub ranked_session_sha256: Digest32,
    pub grant_request_sha256: Digest32,
    pub replay_session_id: Digest32,
    pub host_participant_instance_id: Digest32,
    pub host_nonce: ChallengeNonce32,
    pub admitted_at_unix_ms: u64,
    /// Inclusive last server millisecond at which the complete upload may be
    /// accepted. It is always strictly before the competition's exclusive end.
    pub expires_at_unix_ms: u64,
}

impl CompetitionRunGrantClaimV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        <Self as crate::canonical::DomainSignedClaim>::signing_bytes(self)
    }
}

impl crate::canonical::DomainSignedClaim for CompetitionRunGrantClaimV1 {
    const DOMAIN: &'static [u8] = COMPETITION_RUN_GRANT_SIGNATURE_DOMAIN_V1;
}

impl Validate for CompetitionRunGrantClaimV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("CompetitionRunGrantClaimV1", self.schema_version)?;
        if self.grant_nonce.is_zero()
            || self.grant_authority_public_key.is_zero()
            || self.host_public_key.is_zero()
            || self.competition_manifest_sha256.is_zero()
            || self.ranked_session_sha256.is_zero()
            || self.grant_request_sha256.is_zero()
            || self.replay_session_id.is_zero()
            || self.host_participant_instance_id.is_zero()
            || self.host_nonce.is_zero()
            || self.admitted_at_unix_ms == 0
            || self.expires_at_unix_ms <= self.admitted_at_unix_ms
        {
            return Err(ValidationError::ClaimMismatch {
                field: "competition_run_grant.identity_or_interval",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitionRunGrantV1 {
    pub claim: CompetitionRunGrantClaimV1,
    pub algorithm: SignatureAlgorithmV1,
    pub authority_signature: Signature64,
}

impl CompetitionRunGrantV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        self.claim.signing_bytes()
    }

    pub fn validate_request(
        &self,
        request: &CompetitionRunGrantRequestV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        request.validate()?;
        let request_sha256 =
            request
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "competition_run_grant.grant_request_sha256",
                })?;
        let ranked_session_sha256 =
            request
                .claim
                .ranked_session
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "competition_run_grant.ranked_session_sha256",
                })?;
        if self.claim.grant_request_sha256 != request_sha256
            || self.claim.ranked_session_sha256 != ranked_session_sha256
            || self.claim.host_public_key != request.claim.host_public_key
            || self.claim.competition_manifest_sha256
                != request
                    .claim
                    .ranked_session
                    .competition_manifest_sha256
                    .expect("validated competition grant request has a competition")
            || self.claim.replay_session_id != request.claim.replay_session_id
            || self.claim.host_participant_instance_id != request.claim.host_participant_instance_id
            || self.claim.host_nonce != request.claim.host_nonce
        {
            return Err(ValidationError::ClaimMismatch {
                field: "competition_run_grant.request_binding",
            });
        }
        Ok(())
    }
}

impl Validate for CompetitionRunGrantV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.claim.validate()?;
        crate::validation::nonzero(
            "competition_run_grant.authority_signature",
            &self.authority_signature,
        )?;
        Ok(())
    }
}

/// The only scopes whose starting state is selected from an operator-private
/// canonical template. Continuations use an accepted predecessor and must not
/// be admitted through this capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshRunScopeV1 {
    IndividualLevel,
    CampaignGenesis,
}

/// Host-signed request for fail-closed admission before the first simulation
/// frame. `starting_campaign` is the caller's exact local artifact identity;
/// the private canonical pin and its filesystem path never cross this API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshRunPreflightRequestClaimV1 {
    pub schema_version: u32,
    pub request_nonce: ChallengeNonce32,
    pub host_public_key: PublicKey32,
    pub replay_session_id: Digest32,
    pub host_participant_instance_id: Digest32,
    pub host_nonce: ChallengeNonce32,
    pub scope: FreshRunScopeV1,
    pub starting_campaign: ArtifactRefV1,
    pub ranked_session: RankedSessionConfigV1,
}

impl FreshRunPreflightRequestClaimV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        <Self as crate::canonical::DomainSignedClaim>::signing_bytes(self)
    }
}

impl crate::canonical::DomainSignedClaim for FreshRunPreflightRequestClaimV1 {
    const DOMAIN: &'static [u8] = FRESH_RUN_PREFLIGHT_REQUEST_SIGNATURE_DOMAIN_V1;
}

impl Validate for FreshRunPreflightRequestClaimV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("FreshRunPreflightRequestClaimV1", self.schema_version)?;
        if self.request_nonce.is_zero()
            || self.host_public_key.is_zero()
            || self.replay_session_id.is_zero()
            || self.host_participant_instance_id.is_zero()
            || self.host_nonce.is_zero()
        {
            return Err(ValidationError::Zero {
                field: "fresh_run_preflight_request.identity",
            });
        }
        self.starting_campaign.validate()?;
        self.ranked_session.validate()?;
        if self.starting_campaign.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1
            || self.starting_campaign.sha256 != self.ranked_session.starting_campaign_sha256
            || self.starting_campaign.byte_length
                != self.ranked_session.starting_campaign_byte_length
        {
            return Err(ValidationError::ClaimMismatch {
                field: "fresh_run_preflight_request.starting_campaign",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshRunPreflightRequestV1 {
    pub claim: FreshRunPreflightRequestClaimV1,
    pub algorithm: SignatureAlgorithmV1,
    pub host_signature: Signature64,
}

impl FreshRunPreflightRequestV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        self.claim.signing_bytes()
    }
}

impl Validate for FreshRunPreflightRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.claim.validate()?;
        crate::validation::nonzero(
            "fresh_run_preflight_request.host_signature",
            &self.host_signature,
        )?;
        Ok(())
    }
}

/// Server-authored claim proving that one exact fresh starting artifact and
/// immutable ranked tuple were accepted before frame zero. The authority key
/// is pinned by the immutable ruleset selected by `ranked_session_sha256`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshRunPreflightGrantClaimV1 {
    pub schema_version: u32,
    pub grant_id: OpaqueId,
    pub grant_nonce: ChallengeNonce32,
    pub grant_authority_public_key: PublicKey32,
    pub host_public_key: PublicKey32,
    pub grant_request_sha256: Digest32,
    pub ranked_session_sha256: Digest32,
    pub replay_session_id: Digest32,
    pub host_participant_instance_id: Digest32,
    pub host_nonce: ChallengeNonce32,
    pub scope: FreshRunScopeV1,
    pub starting_campaign: ArtifactRefV1,
    pub admitted_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
}

impl FreshRunPreflightGrantClaimV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        <Self as crate::canonical::DomainSignedClaim>::signing_bytes(self)
    }
}

impl crate::canonical::DomainSignedClaim for FreshRunPreflightGrantClaimV1 {
    const DOMAIN: &'static [u8] = FRESH_RUN_PREFLIGHT_GRANT_SIGNATURE_DOMAIN_V1;
}

impl Validate for FreshRunPreflightGrantClaimV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("FreshRunPreflightGrantClaimV1", self.schema_version)?;
        self.starting_campaign.validate()?;
        if self.grant_nonce.is_zero()
            || self.grant_authority_public_key.is_zero()
            || self.host_public_key.is_zero()
            || self.grant_request_sha256.is_zero()
            || self.ranked_session_sha256.is_zero()
            || self.replay_session_id.is_zero()
            || self.host_participant_instance_id.is_zero()
            || self.host_nonce.is_zero()
            || self.starting_campaign.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1
            || self.admitted_at_unix_ms == 0
            || self.expires_at_unix_ms <= self.admitted_at_unix_ms
        {
            return Err(ValidationError::ClaimMismatch {
                field: "fresh_run_preflight_grant.identity_or_interval",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshRunPreflightGrantV1 {
    pub claim: FreshRunPreflightGrantClaimV1,
    pub algorithm: SignatureAlgorithmV1,
    pub authority_signature: Signature64,
}

impl FreshRunPreflightGrantV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        self.claim.signing_bytes()
    }

    pub fn validate_request(
        &self,
        request: &FreshRunPreflightRequestV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        request.validate()?;
        let request_sha256 =
            request
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "fresh_run_preflight_grant.grant_request_sha256",
                })?;
        let ranked_session_sha256 =
            request
                .claim
                .ranked_session
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "fresh_run_preflight_grant.ranked_session_sha256",
                })?;
        if self.claim.grant_request_sha256 != request_sha256
            || self.claim.ranked_session_sha256 != ranked_session_sha256
            || self.claim.host_public_key != request.claim.host_public_key
            || self.claim.replay_session_id != request.claim.replay_session_id
            || self.claim.host_participant_instance_id != request.claim.host_participant_instance_id
            || self.claim.host_nonce != request.claim.host_nonce
            || self.claim.scope != request.claim.scope
            || self.claim.starting_campaign != request.claim.starting_campaign
        {
            return Err(ValidationError::ClaimMismatch {
                field: "fresh_run_preflight_grant.request_binding",
            });
        }
        Ok(())
    }
}

impl Validate for FreshRunPreflightGrantV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.claim.validate()?;
        crate::validation::nonzero(
            "fresh_run_preflight_grant.authority_signature",
            &self.authority_signature,
        )?;
        Ok(())
    }
}

/// Controller- and host-signed request for an arbitrary authenticated host to
/// continue one exact active campaign chain. The participant key set is the
/// intended durable lobby roster; reconnect instance IDs remain part of the
/// later session transcript and cannot be known before frame zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignContinuationPreflightRequestClaimV1 {
    pub schema_version: u32,
    pub request_nonce: ChallengeNonce32,
    pub host_public_key: PublicKey32,
    pub campaign_controller_public_key: PublicKey32,
    pub replay_session_id: Digest32,
    pub host_participant_instance_id: Digest32,
    pub host_nonce: ChallengeNonce32,
    pub max_concurrent_players: u16,
    pub participant_public_keys: Vec<PublicKey32>,
    pub chain_id: OpaqueId,
    pub predecessor_run_id: OpaqueId,
    pub predecessor_verification_sha256: Digest32,
    pub starting_campaign: ArtifactRefV1,
    pub ranked_session: RankedSessionConfigV1,
}

impl CampaignContinuationPreflightRequestClaimV1 {
    pub fn host_signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        crate::canonical::domain_separated_bytes(
            CAMPAIGN_CONTINUATION_PREFLIGHT_HOST_SIGNATURE_DOMAIN_V1,
            self,
        )
    }

    pub fn controller_signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        crate::canonical::domain_separated_bytes(
            CAMPAIGN_CONTINUATION_PREFLIGHT_CONTROLLER_SIGNATURE_DOMAIN_V1,
            self,
        )
    }
}

impl Validate for CampaignContinuationPreflightRequestClaimV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema(
            "CampaignContinuationPreflightRequestClaimV1",
            self.schema_version,
        )?;
        if self.request_nonce.is_zero()
            || self.host_public_key.is_zero()
            || self.campaign_controller_public_key.is_zero()
            || self.replay_session_id.is_zero()
            || self.host_participant_instance_id.is_zero()
            || self.host_nonce.is_zero()
            || self.predecessor_verification_sha256.is_zero()
        {
            return Err(ValidationError::Zero {
                field: "campaign_continuation_preflight_request.identity",
            });
        }
        self.starting_campaign.validate()?;
        self.ranked_session.validate()?;
        if self.starting_campaign.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1
            || self.starting_campaign.byte_length == 0
            || self.starting_campaign.sha256 != self.ranked_session.starting_campaign_sha256
            || self.starting_campaign.byte_length
                != self.ranked_session.starting_campaign_byte_length
            || self.ranked_session.content_edition != OfficialContentEditionV1::Full
            || self
                .ranked_session
                .campaign_content_manifest_sha256
                .is_none()
        {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_continuation_preflight_request.starting_campaign",
            });
        }
        if self.max_concurrent_players == 0
            || self.max_concurrent_players > MAX_REPLAY_SEATS_V1
            || self.participant_public_keys.is_empty()
            || self.participant_public_keys.len() > usize::from(self.max_concurrent_players)
            || !crate::validation::strictly_sorted(&self.participant_public_keys)
            || self
                .participant_public_keys
                .iter()
                .any(PublicKey32::is_zero)
            || self
                .participant_public_keys
                .binary_search(&self.host_public_key)
                .is_err()
            || self
                .participant_public_keys
                .binary_search(&self.campaign_controller_public_key)
                .is_err()
        {
            return Err(ValidationError::InvalidParticipantClaims);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignContinuationPreflightRequestV1 {
    pub claim: CampaignContinuationPreflightRequestClaimV1,
    pub algorithm: SignatureAlgorithmV1,
    pub host_signature: Signature64,
    pub controller_signature: Signature64,
}

impl Validate for CampaignContinuationPreflightRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.claim.validate()?;
        if self.host_signature.is_zero() || self.controller_signature.is_zero() {
            return Err(ValidationError::Zero {
                field: "campaign_continuation_preflight_request.signature",
            });
        }
        Ok(())
    }
}

/// Server authority proving that the exact predecessor was the active chain
/// head and that its immutable controller authorized this new host/session
/// tuple before simulation began.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignContinuationPreflightGrantClaimV1 {
    pub schema_version: u32,
    pub grant_id: OpaqueId,
    pub grant_nonce: ChallengeNonce32,
    pub grant_authority_public_key: PublicKey32,
    pub grant_request_sha256: Digest32,
    pub ranked_session_sha256: Digest32,
    pub host_public_key: PublicKey32,
    pub campaign_controller_public_key: PublicKey32,
    pub replay_session_id: Digest32,
    pub host_participant_instance_id: Digest32,
    pub host_nonce: ChallengeNonce32,
    pub max_concurrent_players: u16,
    pub participant_public_keys: Vec<PublicKey32>,
    pub chain_id: OpaqueId,
    pub predecessor_run_id: OpaqueId,
    pub predecessor_verification_sha256: Digest32,
    pub starting_campaign: ArtifactRefV1,
    pub admitted_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
}

impl CampaignContinuationPreflightGrantClaimV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        <Self as crate::canonical::DomainSignedClaim>::signing_bytes(self)
    }
}

impl crate::canonical::DomainSignedClaim for CampaignContinuationPreflightGrantClaimV1 {
    const DOMAIN: &'static [u8] = CAMPAIGN_CONTINUATION_PREFLIGHT_GRANT_SIGNATURE_DOMAIN_V1;
}

impl Validate for CampaignContinuationPreflightGrantClaimV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema(
            "CampaignContinuationPreflightGrantClaimV1",
            self.schema_version,
        )?;
        self.starting_campaign.validate()?;
        if self.grant_nonce.is_zero()
            || self.grant_authority_public_key.is_zero()
            || self.grant_request_sha256.is_zero()
            || self.ranked_session_sha256.is_zero()
            || self.host_public_key.is_zero()
            || self.campaign_controller_public_key.is_zero()
            || self.replay_session_id.is_zero()
            || self.host_participant_instance_id.is_zero()
            || self.host_nonce.is_zero()
            || self.predecessor_verification_sha256.is_zero()
            || self.starting_campaign.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1
            || self.starting_campaign.byte_length == 0
            || self.max_concurrent_players == 0
            || self.max_concurrent_players > MAX_REPLAY_SEATS_V1
            || self.participant_public_keys.is_empty()
            || self.participant_public_keys.len() > usize::from(self.max_concurrent_players)
            || !crate::validation::strictly_sorted(&self.participant_public_keys)
            || self
                .participant_public_keys
                .iter()
                .any(PublicKey32::is_zero)
            || self
                .participant_public_keys
                .binary_search(&self.host_public_key)
                .is_err()
            || self
                .participant_public_keys
                .binary_search(&self.campaign_controller_public_key)
                .is_err()
            || self.admitted_at_unix_ms == 0
            || self.expires_at_unix_ms <= self.admitted_at_unix_ms
        {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_continuation_preflight_grant.identity_or_interval",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignContinuationPreflightGrantV1 {
    pub claim: CampaignContinuationPreflightGrantClaimV1,
    pub algorithm: SignatureAlgorithmV1,
    pub authority_signature: Signature64,
}

impl CampaignContinuationPreflightGrantV1 {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        self.claim.signing_bytes()
    }

    pub fn validate_request(
        &self,
        request: &CampaignContinuationPreflightRequestV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        request.validate()?;
        let request_sha256 =
            request
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "campaign_continuation_preflight_grant.grant_request_sha256",
                })?;
        let ranked_session_sha256 =
            request
                .claim
                .ranked_session
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "campaign_continuation_preflight_grant.ranked_session_sha256",
                })?;
        let claim = &request.claim;
        if self.claim.grant_request_sha256 != request_sha256
            || self.claim.ranked_session_sha256 != ranked_session_sha256
            || self.claim.host_public_key != claim.host_public_key
            || self.claim.campaign_controller_public_key != claim.campaign_controller_public_key
            || self.claim.replay_session_id != claim.replay_session_id
            || self.claim.host_participant_instance_id != claim.host_participant_instance_id
            || self.claim.host_nonce != claim.host_nonce
            || self.claim.max_concurrent_players != claim.max_concurrent_players
            || self.claim.participant_public_keys != claim.participant_public_keys
            || self.claim.chain_id != claim.chain_id
            || self.claim.predecessor_run_id != claim.predecessor_run_id
            || self.claim.predecessor_verification_sha256 != claim.predecessor_verification_sha256
            || self.claim.starting_campaign != claim.starting_campaign
        {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_continuation_preflight_grant.request_binding",
            });
        }
        Ok(())
    }
}

impl Validate for CampaignContinuationPreflightGrantV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.claim.validate()?;
        crate::validation::nonzero(
            "campaign_continuation_preflight_grant.authority_signature",
            &self.authority_signature,
        )?;
        Ok(())
    }
}

impl Validate for PreparedMissionInputsSealV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PreparedMissionInputsSealV1", self.schema_version)?;
        self.content_subject.validate()?;
        if [
            self.prepared_inputs_projection_sha256,
            self.content_manifest_sha256,
            self.starting_campaign_sha256,
            self.rules_config_sha256,
        ]
        .into_iter()
        .any(|digest| digest.is_zero())
            || self.starting_campaign_byte_length == 0
            || self
                .spellforge_content_sha256
                .is_some_and(|digest| digest.is_zero())
            || self
                .original_rng_replay_sha256
                .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "prepared_mission_inputs_seal.identity_digest",
            });
        }
        self.resource_locale_root.validate()?;
        self.speech_timing.validate()
    }
}

impl PreparedMissionInputsSealV1 {
    pub fn validate_rankable(&self) -> Result<(), ValidationError> {
        self.validate()?;
        if self.spellforge_content_sha256.is_some() || self.original_rng_replay_sha256.is_some() {
            return Err(ValidationError::ClaimMismatch {
                field: "prepared_mission_inputs_seal.unranked_input_mode",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantClaimV1 {
    pub seat: u16,
    pub participant_instance_id: Digest32,
    pub public_key: PublicKey32,
    /// Controls only what the public leaderboard projection may disclose.
    /// Both variants remain durable-key authenticated and must co-sign.
    pub public_disclosure: ParticipantPublicDisclosureV1,
    /// `None` only for the mandatory host claim in seat 0. Every guest,
    /// including one displayed anonymously, carries its transport-verified
    /// join attestation.
    pub join_attestation: Option<NamedSeatJoinAttestationV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantPublicDisclosureV1 {
    NamedProfile,
    Anonymous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantSignatureV1 {
    pub public_key: PublicKey32,
    pub signature: Signature64,
}

/// Closed purpose set for the only leaderboard co-signing payload accepted by
/// the durable game identity. The numeric tag is part of the fixed signature
/// contract; new purposes require a new contract version rather than reusing a
/// tag.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum LeaderboardCoSignPurposeV1 {
    CampaignContinuation,
    Submission,
}

impl LeaderboardCoSignPurposeV1 {
    const fn signing_tag(self) -> u8 {
        match self {
            Self::CampaignContinuation => 1,
            Self::Submission => 2,
        }
    }
}

/// Deterministic, server-reconstructible identity for one co-sign operation.
///
/// The replay session prevents cross-session use while the digest of the exact
/// server-issued offer binds its one-use upload challenge and nonce. Callers
/// must derive this value from an authoritative validated offer; it is never a
/// client-selected sequence number.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardCoSignInstanceV1 {
    pub purpose: LeaderboardCoSignPurposeV1,
    pub replay_session_id: Digest32,
    pub submission_offer_sha256: Digest32,
}

impl LeaderboardCoSignInstanceV1 {
    pub fn from_offer(
        purpose: LeaderboardCoSignPurposeV1,
        offer: &SubmissionOfferV1,
    ) -> Result<Self, crate::canonical::CanonicalDocumentError> {
        Ok(Self {
            purpose,
            replay_session_id: offer.session_genesis.claim.replay_session_id,
            submission_offer_sha256: offer.canonical_digest()?,
        })
    }
}

impl Validate for LeaderboardCoSignInstanceV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.replay_session_id.is_zero() || self.submission_offer_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "leaderboard_co_sign.instance",
            });
        }
        Ok(())
    }
}

/// The exact request signed by a local player or a remote multiplayer
/// participant. `signing_bytes` is deliberately fixed-size and is the sole
/// co-signature payload; neither native nor browser identities expose a raw
/// signing operation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardCoSignRequestV1 {
    pub instance: LeaderboardCoSignInstanceV1,
    pub run_digest: Digest32,
}

impl LeaderboardCoSignRequestV1 {
    pub fn signing_bytes(
        &self,
    ) -> Result<[u8; LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1], ValidationError> {
        self.validate()?;
        let mut bytes = [0_u8; LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1];
        let mut offset = 0;
        let mut append = |part: &[u8]| {
            let end = offset + part.len();
            bytes[offset..end].copy_from_slice(part);
            offset = end;
        };
        append(LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1);
        append(&[self.instance.purpose.signing_tag()]);
        append(self.instance.replay_session_id.as_bytes());
        append(self.instance.submission_offer_sha256.as_bytes());
        append(self.run_digest.as_bytes());
        debug_assert_eq!(offset, LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1);
        Ok(bytes)
    }
}

impl Validate for LeaderboardCoSignRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.instance.validate()?;
        crate::validation::nonzero("leaderboard_co_sign.run_digest", &self.run_digest)?;
        Ok(())
    }
}

fn validate_participants(
    max_concurrent_players: u16,
    participant_instance_count: u16,
    claims: &[ParticipantClaimV1],
) -> Result<(), ValidationError> {
    if max_concurrent_players == 0
        || max_concurrent_players > MAX_REPLAY_SEATS_V1
        || participant_instance_count == 0
        || participant_instance_count > MAX_PARTICIPANT_INSTANCES_V1
    {
        return Err(ValidationError::EmptyPlayerCount);
    }
    if max_concurrent_players > participant_instance_count
        || claims.len() != usize::from(participant_instance_count)
    {
        return Err(ValidationError::TooManyParticipantClaims);
    }
    if claims
        .first()
        .is_none_or(|claim| claim.seat != 0 || claim.join_attestation.is_some())
    {
        return Err(ValidationError::MissingHostClaim);
    }
    if claims.iter().any(|claim| {
        claim.public_key.is_zero()
            || claim.participant_instance_id.is_zero()
            || claim.seat >= MAX_REPLAY_SEATS_V1
            || (claim.seat != 0 && claim.join_attestation.is_none())
    }) {
        return Err(ValidationError::Zero {
            field: "participant_claims.identity_or_attestation",
        });
    }
    for claim in claims.iter().skip(1) {
        let attestation =
            claim
                .join_attestation
                .as_ref()
                .ok_or(ValidationError::ClaimMismatch {
                    field: "participant_claims.join_attestation",
                })?;
        attestation.validate()?;
        if attestation.claim.public_key != claim.public_key
            || attestation.claim.participant_instance_id != claim.participant_instance_id
            || attestation.claim.seat != claim.seat
        {
            return Err(ValidationError::ClaimMismatch {
                field: "participant_claims.join_attestation",
            });
        }
    }
    if !claims.windows(2).all(|pair| {
        (pair[0].seat, pair[0].participant_instance_id)
            < (pair[1].seat, pair[1].participant_instance_id)
    }) || claims
        .iter()
        .map(|claim| claim.public_key)
        .collect::<BTreeSet<_>>()
        .len()
        != claims.len()
        || claims
            .iter()
            .map(|claim| claim.participant_instance_id)
            .collect::<BTreeSet<_>>()
            .len()
            != claims.len()
    {
        return Err(ValidationError::InvalidParticipantClaims);
    }
    Ok(())
}

fn validate_participant_session_context(
    genesis: &ReplaySessionGenesisV1,
    claims: &[ParticipantClaimV1],
) -> Result<(), ValidationError> {
    genesis.validate()?;
    let genesis_sha256 =
        genesis
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "session_genesis.canonical_digest",
            })?;
    let Some(host) = claims.first() else {
        return Err(ValidationError::MissingHostClaim);
    };
    if host.public_key != genesis.claim.host_public_key
        || host.participant_instance_id != genesis.claim.host_participant_instance_id
    {
        return Err(ValidationError::ClaimMismatch {
            field: "participant_claims.host_session_identity",
        });
    }
    for claim in claims.iter().skip(1) {
        let join = &claim
            .join_attestation
            .as_ref()
            .expect("validate_participants requires every guest attestation")
            .claim;
        if join.session_genesis_sha256 != genesis_sha256
            || join.host_endpoint_id != genesis.claim.host_public_key
            || join.replay_session_id != genesis.claim.replay_session_id
            || join.host_nonce != genesis.claim.host_nonce
            || join.mission_id != genesis.claim.ranked_session.mission_id
            || join.content_manifest_sha256 != genesis.claim.ranked_session.content_manifest_sha256
            || join.rules_config_sha256 != genesis.claim.ranked_session.rules_config_sha256
            || join.ruleset_manifest_sha256 != genesis.claim.ranked_session.ruleset_manifest_sha256
            || join.competition_manifest_sha256
                != genesis.claim.ranked_session.competition_manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "participant_claims.session_context",
            });
        }
    }
    Ok(())
}

fn validate_participants_against_transcript(
    claims: &[ParticipantClaimV1],
    transcript: &ReplaySessionTranscriptV1,
) -> Result<(), ValidationError> {
    let host = claims.first().ok_or(ValidationError::MissingHostClaim)?;
    if host.participant_instance_id != transcript.host_participant_instance_id {
        return Err(ValidationError::ClaimMismatch {
            field: "participant_claims.host_transcript",
        });
    }
    for claim in claims.iter().skip(1) {
        let join = &claim
            .join_attestation
            .as_ref()
            .expect("participant validation requires guest attestation")
            .claim;
        let event = transcript
            .events
            .get(join.join_event_ordinal as usize)
            .ok_or(ValidationError::ClaimMismatch {
                field: "participant_claims.join_event_ordinal",
            })?;
        if join.session_genesis_sha256 != transcript.session_genesis_sha256
            || join.replay_session_id != transcript.replay_session_id
            || event.seat != claim.seat
            || event.participant_instance_id != claim.participant_instance_id
            || event.lifecycle
                != (ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: join.connection_epoch,
                })
        {
            return Err(ValidationError::ClaimMismatch {
                field: "participant_claims.replay_transcript",
            });
        }
    }
    Ok(())
}

fn validate_metrics(field: &'static str, metrics: &[BoardMetricV1]) -> Result<(), ValidationError> {
    if metrics.is_empty() || !crate::validation::strictly_sorted(metrics) {
        return Err(ValidationError::InvalidMetrics { field });
    }
    Ok(())
}

fn durable_participant_keys(claims: &[ParticipantClaimV1]) -> Vec<PublicKey32> {
    let mut keys = claims
        .iter()
        .map(|participant| participant.public_key)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    keys
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScopeRequestV1 {
    IndividualLevel,
    CampaignGenesis,
    CampaignContinuation {
        chain_id: OpaqueId,
        predecessor_run_id: OpaqueId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionOfferRequestV1 {
    pub schema_version: u32,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub participant_claims: Vec<ParticipantClaimV1>,
    pub session_genesis: ReplaySessionGenesisV1,
    pub mission_id: String,
    pub scope_request: ScopeRequestV1,
    /// Exact allowlisted ruleset selected from leaderboard metadata. The
    /// server rejects unknown/inactive digests and still authors every pinned
    /// offer identity; this is selection, not a client-authored manifest.
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
}

impl Validate for SubmissionOfferRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("SubmissionOfferRequestV1", self.schema_version)?;
        crate::validation::text("submission_offer_request.mission_id", &self.mission_id, 256)?;
        crate::validation::nonzero(
            "submission_offer_request.ruleset_manifest_sha256",
            &self.ruleset_manifest_sha256,
        )?;
        validate_participants(
            self.max_concurrent_players,
            self.participant_instance_count,
            &self.participant_claims,
        )?;
        validate_participant_session_context(&self.session_genesis, &self.participant_claims)?;
        if self.session_genesis.claim.ranked_session.mission_id != self.mission_id
            || self
                .session_genesis
                .claim
                .ranked_session
                .ruleset_manifest_sha256
                != self.ruleset_manifest_sha256
            || self
                .session_genesis
                .claim
                .ranked_session
                .competition_manifest_sha256
                != self.competition_manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_offer_request.session_genesis",
            });
        }
        match (
            &self.scope_request,
            &self.session_genesis.claim.fresh_run_preflight_grant,
            &self
                .session_genesis
                .claim
                .campaign_continuation_preflight_grant,
        ) {
            (ScopeRequestV1::IndividualLevel, Some(grant), None) => {
                grant.validate_offer_request(self, FreshRunScopeV1::IndividualLevel)?;
            }
            (ScopeRequestV1::CampaignGenesis, Some(grant), None) => {
                grant.validate_offer_request(self, FreshRunScopeV1::CampaignGenesis)?;
            }
            (ScopeRequestV1::CampaignContinuation { .. }, None, Some(grant)) => {
                grant.validate_offer_request(self)?;
            }
            _ => {
                return Err(ValidationError::ClaimMismatch {
                    field: "submission_offer_request.run_preflight_grant_presence",
                });
            }
        }
        Ok(())
    }
}

// Shared identity policy only: campaign authority remains explicit in the adapters.
fn preflight_session_identity_matches(
    genesis: &ReplaySessionGenesisClaimV1,
    host_public_key: PublicKey32,
    replay_session_id: Digest32,
    host_participant_instance_id: Digest32,
    host_nonce: ChallengeNonce32,
    ranked_session_sha256: Digest32,
    expected_ranked_session_sha256: Digest32,
) -> bool {
    host_public_key == genesis.host_public_key
        && replay_session_id == genesis.replay_session_id
        && host_participant_instance_id == genesis.host_participant_instance_id
        && host_nonce == genesis.host_nonce
        && ranked_session_sha256 == expected_ranked_session_sha256
}

impl CampaignContinuationPreflightGrantV1 {
    fn binds_chain(&self, chain_id: &OpaqueId, predecessor_run_id: &OpaqueId) -> bool {
        &self.claim.chain_id == chain_id && &self.claim.predecessor_run_id == predecessor_run_id
    }

    fn binds_participants(
        &self,
        max_concurrent_players: u16,
        participants: &[ParticipantClaimV1],
    ) -> bool {
        self.claim.max_concurrent_players == max_concurrent_players
            && self.claim.participant_public_keys == durable_participant_keys(participants)
            && participants.iter().any(|participant| {
                participant.public_key == self.claim.campaign_controller_public_key
            })
    }

    fn validate_offer_request(
        &self,
        request: &SubmissionOfferRequestV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        let ScopeRequestV1::CampaignContinuation {
            chain_id,
            predecessor_run_id,
        } = &request.scope_request
        else {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_continuation_preflight_grant.offer_scope",
            });
        };
        let ranked_session_sha256 = request
            .session_genesis
            .claim
            .ranked_session
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "campaign_continuation_preflight_grant.ranked_session_sha256",
            })?;
        if !preflight_session_identity_matches(
            &request.session_genesis.claim,
            self.claim.host_public_key,
            self.claim.replay_session_id,
            self.claim.host_participant_instance_id,
            self.claim.host_nonce,
            self.claim.ranked_session_sha256,
            ranked_session_sha256,
        ) || !self.binds_chain(chain_id, predecessor_run_id)
            || !self.binds_participants(request.max_concurrent_players, &request.participant_claims)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_continuation_preflight_grant.offer_binding",
            });
        }
        Ok(())
    }

    fn validate_offer(&self, offer: &SubmissionOfferV1) -> Result<(), ValidationError> {
        self.validate()?;
        let InitialStateExpectationV1::CampaignContinuation {
            chain_id,
            predecessor_run_id,
            predecessor_verification_sha256,
            campaign_sha256,
            starting_campaign_byte_length,
            ..
        } = &offer.starting_state
        else {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_continuation_preflight_grant.offer_scope",
            });
        };
        let ranked_session_sha256 = offer
            .session_genesis
            .claim
            .ranked_session
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "campaign_continuation_preflight_grant.ranked_session_sha256",
            })?;
        if !preflight_session_identity_matches(
            &offer.session_genesis.claim,
            self.claim.host_public_key,
            self.claim.replay_session_id,
            self.claim.host_participant_instance_id,
            self.claim.host_nonce,
            self.claim.ranked_session_sha256,
            ranked_session_sha256,
        ) || !self.binds_chain(chain_id, predecessor_run_id)
            || self.claim.predecessor_verification_sha256 != *predecessor_verification_sha256
            || self.claim.starting_campaign.sha256 != *campaign_sha256
            || self.claim.starting_campaign.byte_length != *starting_campaign_byte_length
            || !self.binds_participants(offer.max_concurrent_players, &offer.participant_claims)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_continuation_preflight_grant.offer_binding",
            });
        }
        Ok(())
    }
}

impl FreshRunPreflightGrantV1 {
    fn validate_offer_request(
        &self,
        request: &SubmissionOfferRequestV1,
        expected_scope: FreshRunScopeV1,
    ) -> Result<(), ValidationError> {
        // Before admission, campaign facts are supplied by the ranked claim.
        let ranked = &request.session_genesis.claim.ranked_session;
        self.validate_submission_binding(
            &request.session_genesis.claim,
            expected_scope,
            ranked.starting_campaign_sha256,
            ranked.starting_campaign_byte_length,
        )
    }

    fn validate_offer(
        &self,
        offer: &SubmissionOfferV1,
        expected_scope: FreshRunScopeV1,
    ) -> Result<(), ValidationError> {
        // After admission, the authoritative offer owns campaign expectations.
        self.validate_submission_binding(
            &offer.session_genesis.claim,
            expected_scope,
            offer.starting_state.campaign_sha256(),
            offer.starting_state.starting_campaign_byte_length(),
        )
    }

    fn validate_submission_binding(
        &self,
        genesis: &ReplaySessionGenesisClaimV1,
        expected_scope: FreshRunScopeV1,
        campaign_sha256: Digest32,
        campaign_byte_length: u64,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        let ranked_session_sha256 = genesis.ranked_session.canonical_digest().map_err(|_| {
            ValidationError::ClaimMismatch {
                field: "fresh_run_preflight_grant.ranked_session_sha256",
            }
        })?;
        if !preflight_session_identity_matches(
            genesis,
            self.claim.host_public_key,
            self.claim.replay_session_id,
            self.claim.host_participant_instance_id,
            self.claim.host_nonce,
            self.claim.ranked_session_sha256,
            ranked_session_sha256,
        ) || self.claim.scope != expected_scope
            || self.claim.starting_campaign.sha256 != campaign_sha256
            || self.claim.starting_campaign.byte_length != campaign_byte_length
        {
            return Err(ValidationError::ClaimMismatch {
                field: "fresh_run_preflight_grant.offer_binding",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InitialStateExpectationV1 {
    IndividualLevel {
        template_id: OpaqueId,
        campaign_state_requirement: crate::CanonicalCampaignStateRequirementV1,
        campaign_sha256: Digest32,
        starting_campaign_byte_length: u64,
    },
    CampaignGenesis {
        template_id: OpaqueId,
        campaign_state_requirement: crate::CanonicalCampaignStateRequirementV1,
        campaign_sha256: Digest32,
        starting_campaign_byte_length: u64,
    },
    CampaignContinuation {
        chain_id: OpaqueId,
        predecessor_run_id: OpaqueId,
        predecessor_verification_sha256: Digest32,
        campaign_state_requirement: crate::CanonicalCampaignStateRequirementV1,
        campaign_sha256: Digest32,
        starting_campaign_byte_length: u64,
    },
}

impl InitialStateExpectationV1 {
    pub const fn scope_kind(&self) -> RunScopeKindV1 {
        match self {
            Self::IndividualLevel { .. } => RunScopeKindV1::IndividualLevel,
            Self::CampaignGenesis { .. } | Self::CampaignContinuation { .. } => {
                RunScopeKindV1::Campaign
            }
        }
    }

    pub const fn campaign_sha256(&self) -> Digest32 {
        match self {
            Self::IndividualLevel {
                campaign_sha256, ..
            }
            | Self::CampaignGenesis {
                campaign_sha256, ..
            }
            | Self::CampaignContinuation {
                campaign_sha256, ..
            } => *campaign_sha256,
        }
    }

    pub const fn campaign_state_requirement(&self) -> crate::CanonicalCampaignStateRequirementV1 {
        match self {
            Self::IndividualLevel {
                campaign_state_requirement,
                ..
            }
            | Self::CampaignGenesis {
                campaign_state_requirement,
                ..
            }
            | Self::CampaignContinuation {
                campaign_state_requirement,
                ..
            } => *campaign_state_requirement,
        }
    }

    pub const fn starting_campaign_byte_length(&self) -> u64 {
        match self {
            Self::IndividualLevel {
                starting_campaign_byte_length,
                ..
            }
            | Self::CampaignGenesis {
                starting_campaign_byte_length,
                ..
            }
            | Self::CampaignContinuation {
                starting_campaign_byte_length,
                ..
            } => *starting_campaign_byte_length,
        }
    }

    fn validate(&self) -> Result<(), ValidationError> {
        self.campaign_state_requirement().validate()?;
        if self.campaign_sha256().is_zero() || self.starting_campaign_byte_length() == 0 {
            return Err(ValidationError::Zero {
                field: "initial_state.campaign_identity",
            });
        }
        if let Self::CampaignContinuation {
            predecessor_verification_sha256,
            ..
        } = self
            && predecessor_verification_sha256.is_zero()
        {
            return Err(ValidationError::Zero {
                field: "initial_state.predecessor_verification_sha256",
            });
        }
        Ok(())
    }
}

/// Validate the complete official edition, subject, and ranked-scope lane.
///
/// This is intentionally independent of a server admission-profile snapshot:
/// signed offers, verifier jobs, and durable campaign publication all use the
/// same fail-closed authority. A fresh FULL campaign can only start at H01;
/// later field missions and Sherwood headquarters are continuation subjects.
pub fn validate_official_ranked_scope_subject_v1(
    edition: OfficialContentEditionV1,
    subject: &OfficialContentSubjectV1,
    starting_state: &InitialStateExpectationV1,
) -> Result<(), ValidationError> {
    let requirement = starting_state.campaign_state_requirement();
    subject.validate()?;
    requirement.validate()?;
    let subject_is_official = match (edition, subject) {
        (OfficialContentEditionV1::Demo, OfficialContentSubjectV1::FieldMission { mission_id }) => {
            crate::OFFICIAL_DEMO_FIELD_MISSION_IDS_V1.contains(&mission_id.as_str())
        }
        (OfficialContentEditionV1::Full, OfficialContentSubjectV1::FieldMission { mission_id }) => {
            crate::OFFICIAL_FULL_FIELD_MISSION_IDS_V1.contains(&mission_id.as_str())
        }
        (OfficialContentEditionV1::Full, OfficialContentSubjectV1::Headquarters { mission_id }) => {
            mission_id == crate::OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1
        }
        (OfficialContentEditionV1::Demo, OfficialContentSubjectV1::Headquarters { .. }) => false,
    };
    let lane_is_authorized = match (edition, subject, starting_state) {
        (
            OfficialContentEditionV1::Demo | OfficialContentEditionV1::Full,
            OfficialContentSubjectV1::FieldMission { .. },
            InitialStateExpectationV1::IndividualLevel { .. },
        ) => true,
        (
            OfficialContentEditionV1::Full,
            OfficialContentSubjectV1::FieldMission { mission_id },
            InitialStateExpectationV1::CampaignGenesis { .. },
        ) => mission_id == crate::OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1,
        (
            OfficialContentEditionV1::Full,
            OfficialContentSubjectV1::FieldMission { .. }
            | OfficialContentSubjectV1::Headquarters { .. },
            InitialStateExpectationV1::CampaignContinuation { .. },
        ) => true,
        _ => false,
    };
    if !subject_is_official || !lane_is_authorized || requirement.edition != edition {
        return Err(ValidationError::ClaimMismatch {
            field: "ranked_scope_subject.official_lane",
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionOfferV1 {
    pub schema_version: u32,
    pub upload_challenge_id: OpaqueId,
    pub upload_challenge_nonce: ChallengeNonce32,
    pub expires_at_unix_ms: u64,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub participant_claims: Vec<ParticipantClaimV1>,
    pub session_genesis: ReplaySessionGenesisV1,
    pub mission_id: String,
    pub competition_manifest_sha256: Option<Digest32>,
    pub build_manifest_sha256: Digest32,
    pub content_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub starting_state: InitialStateExpectationV1,
    pub allowed_metrics: Vec<BoardMetricV1>,
}

impl SubmissionOfferV1 {
    pub fn upload_challenge(&self) -> UploadChallengeV1 {
        UploadChallengeV1 {
            schema_version: self.schema_version,
            upload_challenge_id: self.upload_challenge_id.clone(),
            upload_challenge_nonce: self.upload_challenge_nonce,
            expires_at_unix_ms: self.expires_at_unix_ms,
        }
    }
}

impl Validate for SubmissionOfferV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("SubmissionOfferV1", self.schema_version)?;
        self.upload_challenge().validate()?;
        crate::validation::text("submission_offer.mission_id", &self.mission_id, 256)?;
        validate_participants(
            self.max_concurrent_players,
            self.participant_instance_count,
            &self.participant_claims,
        )?;
        validate_participant_session_context(&self.session_genesis, &self.participant_claims)?;
        self.starting_state.validate()?;
        validate_official_ranked_scope_subject_v1(
            self.session_genesis.claim.ranked_session.content_edition,
            &self.session_genesis.claim.ranked_session.content_subject,
            &self.starting_state,
        )?;
        match (
            &self.starting_state,
            &self.session_genesis.claim.fresh_run_preflight_grant,
            &self
                .session_genesis
                .claim
                .campaign_continuation_preflight_grant,
        ) {
            (InitialStateExpectationV1::IndividualLevel { .. }, Some(grant), None) => {
                grant.validate_offer(self, FreshRunScopeV1::IndividualLevel)?;
            }
            (InitialStateExpectationV1::CampaignGenesis { .. }, Some(grant), None) => {
                grant.validate_offer(self, FreshRunScopeV1::CampaignGenesis)?;
            }
            (InitialStateExpectationV1::CampaignContinuation { .. }, None, Some(grant)) => {
                grant.validate_offer(self)?;
            }
            _ => {
                return Err(ValidationError::ClaimMismatch {
                    field: "submission_offer.run_preflight_grant_presence",
                });
            }
        }
        if self
            .starting_state
            .campaign_state_requirement()
            .rules_config_sha256
            != self.rules_config_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_offer.starting_state.rules_config_sha256",
            });
        }
        let expected_campaign_content = match self.starting_state {
            InitialStateExpectationV1::IndividualLevel { .. } => None,
            InitialStateExpectationV1::CampaignGenesis { .. }
            | InitialStateExpectationV1::CampaignContinuation { .. } => {
                self.session_genesis
                    .claim
                    .ranked_session
                    .campaign_content_manifest_sha256
            }
        };
        if self
            .session_genesis
            .claim
            .ranked_session
            .campaign_content_manifest_sha256
            != expected_campaign_content
            || (self.starting_state.scope_kind() == RunScopeKindV1::Campaign
                && expected_campaign_content.is_none())
        {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_offer.campaign_content_manifest_sha256",
            });
        }
        for digest in [
            self.build_manifest_sha256,
            self.content_manifest_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
        ] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "submission_offer.identity_digest",
                });
            }
        }
        if self
            .competition_manifest_sha256
            .is_some_and(|digest| digest.is_zero())
            || self.competition_manifest_sha256
                != self
                    .session_genesis
                    .claim
                    .ranked_session
                    .competition_manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_offer.competition_manifest_sha256",
            });
        }
        if self
            .session_genesis
            .claim
            .competition_run_grant
            .as_ref()
            .is_some_and(|grant| self.expires_at_unix_ms > grant.claim.expires_at_unix_ms)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_offer.competition_run_grant_expiry",
            });
        }
        let preflight_expiry = self
            .session_genesis
            .claim
            .fresh_run_preflight_grant
            .as_ref()
            .map(|grant| grant.claim.expires_at_unix_ms)
            .or_else(|| {
                self.session_genesis
                    .claim
                    .campaign_continuation_preflight_grant
                    .as_ref()
                    .map(|grant| grant.claim.expires_at_unix_ms)
            })
            .expect("validated offer has exactly one run-preflight grant");
        if self.expires_at_unix_ms > preflight_expiry {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_offer.run_preflight_grant_expiry",
            });
        }
        if self.session_genesis.claim.ranked_session.mission_id != self.mission_id
            || self
                .session_genesis
                .claim
                .ranked_session
                .starting_campaign_sha256
                != self.starting_state.campaign_sha256()
            || self
                .session_genesis
                .claim
                .ranked_session
                .starting_campaign_byte_length
                != self.starting_state.starting_campaign_byte_length()
            || self
                .session_genesis
                .claim
                .ranked_session
                .build_manifest_sha256
                != self.build_manifest_sha256
            || self
                .session_genesis
                .claim
                .ranked_session
                .content_manifest_sha256
                != self.content_manifest_sha256
            || self
                .session_genesis
                .claim
                .ranked_session
                .rules_config_sha256
                != self.rules_config_sha256
            || self
                .session_genesis
                .claim
                .ranked_session
                .ruleset_manifest_sha256
                != self.ruleset_manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_offer.session_genesis",
            });
        }
        validate_metrics("submission_offer.allowed_metrics", &self.allowed_metrics)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayArtifactV1 {
    /// Replay bytes are transferred separately; this is their exact identity
    /// and length, not an inline payload.
    pub artifact: ArtifactRefV1,
    pub replay_schema_version: u32,
}

/// The exact replay and starting campaign co-signed by every authenticated
/// participant, independently of named or anonymous public disclosure.
///
/// `replay` is the sole replay artifact: the verifier resimulates these exact
/// bytes and the public download returns these exact bytes. Participant and
/// session identity belongs in the separately signed transcript, never in the
/// replay artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionArtifactsV1 {
    pub replay: ReplayArtifactV1,
    pub starting_campaign: ArtifactRefV1,
}

impl Validate for SubmissionArtifactsV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.replay.validate()?;
        self.starting_campaign.validate()?;
        if self.starting_campaign.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_artifacts.starting_campaign.media_type",
            });
        }
        Ok(())
    }
}

/// Explicit scope co-signed with the exact final replay envelope. Campaign
/// consent authorizes this independently verified session to participate in
/// the server-recognized chain rooted by the genesis offer (or named by the
/// continuation offer) and in its eventual full-campaign aggregate. It never
/// authorizes a different replay, offer, chain, or participant attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignAggregationConsentV1 {
    NotAuthorized,
    AuthorizeSignedSessionInServerRecognizedChainV1,
}

/// Exact continuation claim signed by the immutable controller established by
/// the ordinal-0 campaign genesis host. Public chain/run IDs are locators, not
/// authorization. The claim binds the predecessor, next signed genesis and
/// exact final replay artifact; the controller must also remain an authenticated
/// participant (and therefore a normal submission co-signer) in every session,
/// even when publicly anonymous.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignContinuationAuthorizationClaimV1 {
    pub schema_version: u32,
    pub campaign_controller_public_key: PublicKey32,
    pub chain_id: OpaqueId,
    pub predecessor_run_id: OpaqueId,
    pub predecessor_verification_sha256: Digest32,
    pub next_session_genesis_sha256: Digest32,
    pub next_artifacts: SubmissionArtifactsV1,
}

impl CampaignContinuationAuthorizationClaimV1 {
    /// Derive the only co-sign request valid for this continuation claim and
    /// authoritative server offer.
    ///
    /// This repeats the final-envelope cross-bind before any signature is
    /// produced, so an isolated controller signer never authorizes a claim for
    /// an unrelated upload challenge, replay session, predecessor, or artifact.
    pub fn co_sign_request(
        &self,
        offer: &SubmissionOfferV1,
    ) -> Result<LeaderboardCoSignRequestV1, crate::canonical::CanonicalDocumentError> {
        self.validate()?;
        offer.validate()?;
        let (
            chain_id,
            predecessor_run_id,
            predecessor_verification_sha256,
            starting_campaign_sha256,
            starting_campaign_byte_length,
        ) = match &offer.starting_state {
            InitialStateExpectationV1::CampaignContinuation {
                chain_id,
                predecessor_run_id,
                predecessor_verification_sha256,
                campaign_sha256,
                starting_campaign_byte_length,
                ..
            } => (
                chain_id,
                predecessor_run_id,
                *predecessor_verification_sha256,
                *campaign_sha256,
                *starting_campaign_byte_length,
            ),
            InitialStateExpectationV1::IndividualLevel { .. }
            | InitialStateExpectationV1::CampaignGenesis { .. } => {
                return Err(ValidationError::ClaimMismatch {
                    field: "campaign_continuation_authorization.scope",
                }
                .into());
            }
        };
        let session_genesis_sha256 = offer.session_genesis.canonical_digest()?;
        if &self.chain_id != chain_id
            || &self.predecessor_run_id != predecessor_run_id
            || self.predecessor_verification_sha256 != predecessor_verification_sha256
            || self.next_session_genesis_sha256 != session_genesis_sha256
            || self.next_artifacts.starting_campaign.sha256 != starting_campaign_sha256
            || self.next_artifacts.starting_campaign.byte_length != starting_campaign_byte_length
            || !offer
                .participant_claims
                .iter()
                .any(|participant| participant.public_key == self.campaign_controller_public_key)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_continuation_authorization.offer",
            }
            .into());
        }
        let digest_input = crate::canonical::domain_separated_bytes(
            CAMPAIGN_CONTINUATION_SIGNATURE_DOMAIN_V1,
            self,
        )?;
        Ok(LeaderboardCoSignRequestV1 {
            instance: LeaderboardCoSignInstanceV1::from_offer(
                LeaderboardCoSignPurposeV1::CampaignContinuation,
                offer,
            )?,
            run_digest: Digest32::digest_bytes(digest_input),
        })
    }
}

impl Validate for CampaignContinuationAuthorizationClaimV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema(
            "CampaignContinuationAuthorizationClaimV1",
            self.schema_version,
        )?;
        if self.campaign_controller_public_key.is_zero()
            || self.predecessor_verification_sha256.is_zero()
            || self.next_session_genesis_sha256.is_zero()
        {
            return Err(ValidationError::Zero {
                field: "campaign_continuation_authorization.identity",
            });
        }
        self.next_artifacts.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignContinuationAuthorizationV1 {
    pub claim: CampaignContinuationAuthorizationClaimV1,
    pub algorithm: SignatureAlgorithmV1,
    pub signature: Signature64,
}

impl CampaignContinuationAuthorizationV1 {
    pub fn signing_bytes(
        &self,
        offer: &SubmissionOfferV1,
    ) -> Result<[u8; LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1], crate::canonical::CanonicalDocumentError>
    {
        Ok(self.claim.co_sign_request(offer)?.signing_bytes()?)
    }
}

impl Validate for CampaignContinuationAuthorizationV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.claim.validate()?;
        crate::validation::nonzero(
            "campaign_continuation_authorization.signature",
            &self.signature,
        )?;
        Ok(())
    }
}

impl Validate for ReplayArtifactV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.artifact.validate()?;
        if self.artifact.media_type != RANKED_REPLAY_MEDIA_TYPE_V1 {
            return Err(ValidationError::ClaimMismatch {
                field: "replay.artifact.media_type",
            });
        }
        if self.artifact.byte_length == 0 {
            return Err(ValidationError::Zero {
                field: "replay.artifact.byte_length",
            });
        }
        if self.replay_schema_version != crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1 {
            return Err(ValidationError::ClaimMismatch {
                field: "replay.replay_schema_version",
            });
        }
        Ok(())
    }
}

/// Complete unsigned claim co-signed by every authenticated participant;
/// public disclosure is independent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionEnvelopeV1 {
    pub schema_version: u32,
    pub offer: SubmissionOfferV1,
    /// Co-signed verifier-only lifecycle evidence. The verifier binds these
    /// identities and ordinals to the identity-free seat commands in `replay`.
    /// Replay-download DTOs never expose this separate signing transcript.
    pub replay_session_transcript: ReplaySessionTranscriptV1,
    pub artifacts: SubmissionArtifactsV1,
    pub campaign_aggregation_consent: CampaignAggregationConsentV1,
    pub campaign_continuation_authorization: Option<CampaignContinuationAuthorizationV1>,
    pub requested_metrics: Vec<BoardMetricV1>,
}

impl SubmissionEnvelopeV1 {
    /// Derive the final all-participant request. The purpose-specific digest
    /// covers the complete validated envelope, including the canonical replay
    /// artifact, exact starting campaign, transcript, server offer, and any
    /// preceding campaign-controller authorization.
    pub fn co_sign_request(
        &self,
    ) -> Result<LeaderboardCoSignRequestV1, crate::canonical::CanonicalDocumentError> {
        self.validate()?;
        let digest_input =
            crate::canonical::domain_separated_bytes(SUBMISSION_SIGNATURE_DOMAIN_V1, self)?;
        Ok(LeaderboardCoSignRequestV1 {
            instance: LeaderboardCoSignInstanceV1::from_offer(
                LeaderboardCoSignPurposeV1::Submission,
                &self.offer,
            )?,
            run_digest: Digest32::digest_bytes(digest_input),
        })
    }

    pub fn signing_bytes(
        &self,
    ) -> Result<[u8; LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1], crate::canonical::CanonicalDocumentError>
    {
        Ok(self.co_sign_request()?.signing_bytes()?)
    }
}

impl Validate for SubmissionEnvelopeV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("SubmissionEnvelopeV1", self.schema_version)?;
        self.offer.validate()?;
        self.replay_session_transcript.validate()?;
        self.artifacts.validate()?;
        let session_genesis_sha256 =
            self.offer.session_genesis.canonical_digest().map_err(|_| {
                ValidationError::ClaimMismatch {
                    field: "submission.replay_session_transcript.session_genesis_sha256",
                }
            })?;
        if self.replay_session_transcript.session_genesis_sha256 != session_genesis_sha256
            || self.replay_session_transcript.replay_session_id
                != self.offer.session_genesis.claim.replay_session_id
            || self.replay_session_transcript.host_participant_instance_id
                != self
                    .offer
                    .session_genesis
                    .claim
                    .host_participant_instance_id
            || self.replay_session_transcript.max_concurrent_players
                != self.offer.max_concurrent_players
            || self.replay_session_transcript.participant_instance_count
                != self.offer.participant_instance_count
        {
            return Err(ValidationError::ClaimMismatch {
                field: "submission.replay_session_transcript.session_context",
            });
        }
        validate_participants_against_transcript(
            &self.offer.participant_claims,
            &self.replay_session_transcript,
        )?;
        if self.artifacts.starting_campaign.sha256 != self.offer.starting_state.campaign_sha256()
            || self.artifacts.starting_campaign.byte_length
                != self.offer.starting_state.starting_campaign_byte_length()
        {
            return Err(ValidationError::ClaimMismatch {
                field: "submission.artifacts.starting_campaign",
            });
        }
        let expected_consent = match self.offer.starting_state.scope_kind() {
            RunScopeKindV1::IndividualLevel => CampaignAggregationConsentV1::NotAuthorized,
            RunScopeKindV1::Campaign => {
                CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1
            }
        };
        if self.campaign_aggregation_consent != expected_consent {
            return Err(ValidationError::ClaimMismatch {
                field: "submission.campaign_aggregation_consent",
            });
        }
        match (
            &self.offer.starting_state,
            &self.campaign_continuation_authorization,
        ) {
            (InitialStateExpectationV1::CampaignContinuation { .. }, Some(authorization)) => {
                authorization.validate()?;
                let claim = &authorization.claim;
                claim
                    .co_sign_request(&self.offer)
                    .map_err(|_| ValidationError::ClaimMismatch {
                        field: "campaign_continuation_authorization.offer",
                    })?;
                if claim.next_artifacts != self.artifacts {
                    return Err(ValidationError::ClaimMismatch {
                        field: "campaign_continuation_authorization.submission",
                    });
                }
            }
            (InitialStateExpectationV1::CampaignContinuation { .. }, None)
            | (
                InitialStateExpectationV1::IndividualLevel { .. }
                | InitialStateExpectationV1::CampaignGenesis { .. },
                Some(_),
            ) => {
                return Err(ValidationError::ClaimMismatch {
                    field: "campaign_continuation_authorization.scope",
                });
            }
            (
                InitialStateExpectationV1::IndividualLevel { .. }
                | InitialStateExpectationV1::CampaignGenesis { .. },
                None,
            ) => {}
        }
        validate_metrics("submission.requested_metrics", &self.requested_metrics)?;
        if !self
            .requested_metrics
            .iter()
            .all(|metric| self.offer.allowed_metrics.binary_search(metric).is_ok())
        {
            return Err(ValidationError::MetricsNotOffered);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureAlgorithmV1 {
    Ed25519,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedSubmissionV1 {
    pub schema_version: u32,
    pub submission: SubmissionEnvelopeV1,
    pub algorithm: SignatureAlgorithmV1,
    pub participant_signatures: Vec<ParticipantSignatureV1>,
}

impl SignedSubmissionV1 {
    pub fn signing_bytes(
        &self,
    ) -> Result<[u8; LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1], crate::canonical::CanonicalDocumentError>
    {
        self.submission.signing_bytes()
    }
}

impl Validate for SignedSubmissionV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("SignedSubmissionV1", self.schema_version)?;
        self.submission.validate()?;
        let expected_signers = self
            .submission
            .offer
            .participant_claims
            .iter()
            .map(|claim| claim.public_key)
            .collect::<BTreeSet<_>>();
        if self.participant_signatures.len() != expected_signers.len()
            || self
                .participant_signatures
                .iter()
                .map(|signature| signature.public_key)
                .ne(expected_signers)
        {
            return Err(ValidationError::InvalidParticipantSignatures);
        }
        if self
            .participant_signatures
            .iter()
            .any(|participant| participant.public_key.is_zero() || participant.signature.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "participant_signatures.key_or_signature",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationLimitsV1 {
    pub max_input_bytes: u64,
    pub max_compressed_bytes: u64,
    pub max_decompressed_bytes: u64,
    pub max_base64_payload_bytes: u64,
    pub max_campaign_bytes: u64,
    pub max_frames: u32,
    pub max_version_bytes: u32,
    pub max_mission_id_bytes: u32,
    pub max_metadata_records: u32,
    pub max_entries_per_frame: u32,
}

impl Validate for VerificationLimitsV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        for (field, value) in [
            ("limits.max_input_bytes", self.max_input_bytes),
            ("limits.max_compressed_bytes", self.max_compressed_bytes),
            ("limits.max_decompressed_bytes", self.max_decompressed_bytes),
            (
                "limits.max_base64_payload_bytes",
                self.max_base64_payload_bytes,
            ),
            ("limits.max_campaign_bytes", self.max_campaign_bytes),
            ("limits.max_frames", u64::from(self.max_frames)),
            (
                "limits.max_version_bytes",
                u64::from(self.max_version_bytes),
            ),
            (
                "limits.max_mission_id_bytes",
                u64::from(self.max_mission_id_bytes),
            ),
            (
                "limits.max_metadata_records",
                u64::from(self.max_metadata_records),
            ),
            (
                "limits.max_entries_per_frame",
                u64::from(self.max_entries_per_frame),
            ),
        ] {
            if value == 0 {
                return Err(ValidationError::Zero { field });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationRequestV1 {
    pub schema_version: u32,
    pub request_id: OpaqueId,
    pub submission: SignedSubmissionV1,
    pub limits: VerificationLimitsV1,
}

impl Validate for VerificationRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("VerificationRequestV1", self.schema_version)?;
        self.submission.validate()?;
        self.limits.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunScopeKindV1 {
    IndividualLevel,
    Campaign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOutcomeV1 {
    Won,
    Lost,
    Interrupted,
}

/// Verifier-observed entry points which make a run ineligible for a public
/// verified board. The evidence is cumulative: once observed it cannot be
/// cleared later in the recording.
///
/// Replay resimulation proves a deterministic outcome under pinned rules and
/// content. It does not prove that a human supplied the input or that an
/// official binary executed it; without remote attestation, synthetic normal
/// UI commands are indistinguishable from human UI commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputTaintKindV1 {
    HttpPlayerCommand,
    HttpSimulationStep,
    HttpStateMutation,
    ConsoleCommand,
    CheatCommand,
    HeadlessAutomation,
    ReplayPlayback,
    StateLoad,
    MissionRestart,
    DebugInputInjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputTaintV1 {
    pub kind: InputTaintKindV1,
    pub first_frame: u32,
}

/// Stable, deliberately coarse reason safe to expose in public API results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputIneligibilityReasonV1 {
    HttpAutomation,
    ConsoleUsed,
    CheatUsed,
    HeadlessAutomation,
    ReplayPlayback,
    StateLoaded,
    MissionRestarted,
    DebugInputInjection,
}

impl InputTaintKindV1 {
    pub const fn public_reason(self) -> InputIneligibilityReasonV1 {
        match self {
            Self::HttpPlayerCommand | Self::HttpSimulationStep | Self::HttpStateMutation => {
                InputIneligibilityReasonV1::HttpAutomation
            }
            Self::ConsoleCommand => InputIneligibilityReasonV1::ConsoleUsed,
            Self::CheatCommand => InputIneligibilityReasonV1::CheatUsed,
            Self::HeadlessAutomation => InputIneligibilityReasonV1::HeadlessAutomation,
            Self::ReplayPlayback => InputIneligibilityReasonV1::ReplayPlayback,
            Self::StateLoad => InputIneligibilityReasonV1::StateLoaded,
            Self::MissionRestart => InputIneligibilityReasonV1::MissionRestarted,
            Self::DebugInputInjection => InputIneligibilityReasonV1::DebugInputInjection,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputProvenanceStatusV1 {
    Rankable,
    Tainted { taints: Vec<InputTaintV1> },
}

impl InputProvenanceStatusV1 {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Self::Tainted { taints } = self
            && (taints.is_empty() || !taints.windows(2).all(|pair| pair[0].kind < pair[1].kind))
        {
            return Err(ValidationError::InvalidInputTaints);
        }
        Ok(())
    }

    pub const fn is_rankable(&self) -> bool {
        matches!(self, Self::Rankable)
    }

    /// Coarse, stable reasons suitable for public UI. Multiple low-level HTTP
    /// taints intentionally collapse to one automation reason.
    pub fn public_reasons(&self) -> Vec<InputIneligibilityReasonV1> {
        match self {
            Self::Rankable => Vec::new(),
            Self::Tainted { taints } => taints
                .iter()
                .map(|taint| taint.kind.public_reason())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        }
    }
}

/// Canonical verifier proof that one verified campaign session reached the
/// immutable completion predicate selected by its ruleset and campaign
/// content catalog. Public DTOs expose only this document's canonical digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignCompleteEvidenceV1 {
    pub schema_version: u32,
    pub campaign_content_manifest_sha256: Digest32,
    pub content_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub verification_request_sha256: Digest32,
    pub replay_sha256: Digest32,
    pub terminal_subject: OfficialContentSubjectV1,
    pub final_campaign_sha256: Digest32,
    pub final_state_sha256: Digest32,
    pub observed_progression_percent: u8,
}

impl Validate for CampaignCompleteEvidenceV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("CampaignCompleteEvidenceV1", self.schema_version)?;
        for digest in [
            self.campaign_content_manifest_sha256,
            self.content_manifest_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
            self.verification_request_sha256,
            self.replay_sha256,
            self.final_campaign_sha256,
            self.final_state_sha256,
        ] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "campaign_complete_evidence.proof_digest",
                });
            }
        }
        self.terminal_subject.validate()?;
        if self.observed_progression_percent == 0 || self.observed_progression_percent > 100 {
            return Err(ValidationError::CountOutOfRange {
                field: "campaign_complete_evidence.observed_progression_percent",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedRunV1 {
    pub scope_kind: RunScopeKindV1,
    pub campaign_aggregation_consent: CampaignAggregationConsentV1,
    /// Campaign/HQ session identity and its zero-based position in the
    /// server-recognized chain. Individual-Level runs carry neither field.
    pub campaign_session_kind: Option<CampaignSessionKindV1>,
    pub campaign_session_ordinal: Option<u32>,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub named_participant_instance_count: u16,
    pub anonymous_participant_instance_count: u16,
    /// Every seat whose authenticated replay-session binding the verifier
    /// matched to the co-signed submission. Anonymous display is disclosure
    /// metadata only and never removes a key or required signature.
    pub authenticated_participant_claims: Vec<ParticipantClaimV1>,
    pub replay_session_transcript: ReplaySessionTranscriptV1,
    pub outcome: TerminalOutcomeV1,
    /// Exact starting campaign consumed by the authoritative Engine.
    pub starting_campaign: ArtifactRefV1,
    /// Exact terminal campaign derived by resimulating the canonical replay.
    pub final_campaign: ArtifactRefV1,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    /// SHA-256 of the terminal native Engine snapshot obtained from the sole
    /// canonical replay resimulation.
    pub final_state_sha256: Digest32,
    pub replay_frames: u32,
    pub original_score_delta: i64,
    pub active_simulation_ticks: u64,
    pub ransom_collected: u64,
    /// Present only when this successful Campaign session independently
    /// reached the ruleset-defined campaign-complete terminal predicate.
    pub campaign_complete_evidence: Option<CampaignCompleteEvidenceV1>,
    /// Complete verifier-derived achievement decisions, sorted by stable ID.
    pub achievements: Vec<VerifiedAchievementV1>,
    /// Bounded namespaced diagnostics for operators. These values are never
    /// authoritative inputs to ranking, campaign reduction, or public proof.
    pub diagnostics: BTreeMap<String, CanonicalValue>,
}

impl VerifiedRunV1 {
    pub const fn metrics(&self) -> RunMetricsV1 {
        RunMetricsV1 {
            original_score_delta: self.original_score_delta,
            active_simulation_ticks: self.active_simulation_ticks,
            ransom_collected: self.ransom_collected,
        }
    }
}

impl Validate for VerifiedRunV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        match (
            self.scope_kind,
            self.campaign_aggregation_consent,
            &self.campaign_session_kind,
            self.campaign_session_ordinal,
        ) {
            (
                RunScopeKindV1::IndividualLevel,
                CampaignAggregationConsentV1::NotAuthorized,
                None,
                None,
            ) => {}
            (
                RunScopeKindV1::Campaign,
                CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1,
                Some(kind),
                Some(ordinal),
            ) if ordinal < MAX_CAMPAIGN_SESSIONS_V1 => kind.validate()?,
            _ => {
                return Err(ValidationError::ClaimMismatch {
                    field: "verified_run.campaign_session",
                });
            }
        }
        validate_participants(
            self.max_concurrent_players,
            self.participant_instance_count,
            &self.authenticated_participant_claims,
        )?;
        let named_claims = self
            .authenticated_participant_claims
            .iter()
            .filter(|claim| claim.public_disclosure == ParticipantPublicDisclosureV1::NamedProfile)
            .count();
        let anonymous_claims = self.authenticated_participant_claims.len() - named_claims;
        if usize::from(self.named_participant_instance_count) != named_claims
            || usize::from(self.anonymous_participant_instance_count) != anonymous_claims
            || self
                .named_participant_instance_count
                .checked_add(self.anonymous_participant_instance_count)
                != Some(self.participant_instance_count)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verified_run.participant_instance_split",
            });
        }
        self.replay_session_transcript.validate()?;
        if self.replay_session_transcript.max_concurrent_players != self.max_concurrent_players
            || self.replay_session_transcript.participant_instance_count
                != self.participant_instance_count
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verified_run.session_counts",
            });
        }
        if self.replay_frames == 0
            || self
                .replay_session_transcript
                .events
                .iter()
                .any(|event| event.replay_ordinal >= self.replay_frames)
        {
            return Err(ValidationError::CountOutOfRange {
                field: "verified_run.replay_session_transcript.replay_ordinal",
            });
        }
        validate_participants_against_transcript(
            &self.authenticated_participant_claims,
            &self.replay_session_transcript,
        )?;
        for artifact in [&self.starting_campaign, &self.final_campaign] {
            artifact.validate()?;
            if artifact.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
                return Err(ValidationError::ClaimMismatch {
                    field: "verified_run.campaign.media_type",
                });
            }
        }
        crate::validation::nonzero("verified_run.state_digest", &self.final_state_sha256)?;
        let wrapped_score_delta = i64::from(
            self.final_campaign_score
                .wrapping_sub(self.starting_campaign_score) as u32,
        );
        if !(0..=i64::from(u32::MAX)).contains(&self.original_score_delta)
            || self.original_score_delta != wrapped_score_delta
        {
            return Err(ValidationError::InvalidOriginalScore);
        }
        if self.campaign_complete_evidence.is_some()
            && (self.scope_kind != RunScopeKindV1::Campaign
                || self.outcome != TerminalOutcomeV1::Won)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verified_run.campaign_complete_evidence",
            });
        }
        if let Some(evidence) = &self.campaign_complete_evidence {
            evidence.validate()?;
            if evidence.final_campaign_sha256 != self.final_campaign.sha256
                || evidence.final_state_sha256 != self.final_state_sha256
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "verified_run.campaign_complete_evidence.final_state",
                });
            }
        }
        if self.achievements.len() > 256
            || !self
                .achievements
                .windows(2)
                .all(|pair| pair[0].achievement_id < pair[1].achievement_id)
        {
            return Err(ValidationError::NotCanonicalOrder {
                field: "verified_run.achievements",
            });
        }
        for achievement in &self.achievements {
            achievement.validate()?;
        }
        if self.diagnostics.len() > 64 {
            return Err(ValidationError::CountOutOfRange {
                field: "verified_run.diagnostics",
            });
        }
        for (key, value) in &self.diagnostics {
            crate::validation::text("verified_run.diagnostics.key", key, 256)?;
            value.validate_depth(64)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedAchievementV1 {
    pub achievement_id: OpaqueId,
    /// Lossless verifier decision. In particular, an unavailable dependency
    /// is not silently collapsed into a legitimate `not_earned` result.
    pub evaluation: VerifiedAchievementEvaluationV1,
    /// Exact bounded verifier evidence. Empty evidence is valid when the
    /// achievement definition is itself a single terminal predicate.
    pub evidence: BTreeMap<String, CanonicalValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifiedAchievementEvaluationV1 {
    Unverifiable,
    NotEarned,
    Earned,
}

impl VerifiedAchievementV1 {
    pub const fn is_awarded(&self) -> bool {
        matches!(self.evaluation, VerifiedAchievementEvaluationV1::Earned)
    }
}

impl Validate for VerifiedAchievementV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.evidence.len() > 64 {
            return Err(ValidationError::CountOutOfRange {
                field: "verified_achievement.evidence",
            });
        }
        for (key, value) in &self.evidence {
            crate::validation::text("verified_achievement.evidence.key", key, 128)?;
            value.validate_depth(16)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CampaignSessionKindV1 {
    FieldMission { mission_id: String },
    Headquarters { hq_sequence: u32 },
}

impl CampaignSessionKindV1 {
    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::FieldMission { mission_id } => {
                crate::validation::text("campaign_session.mission_id", mission_id, 256)?;
            }
            Self::Headquarters { hq_sequence } if *hq_sequence == 0 => {
                return Err(ValidationError::Zero {
                    field: "campaign_session.hq_sequence",
                });
            }
            Self::Headquarters { .. } => {}
        }
        Ok(())
    }
}

/// Immutable verifier-result link used by the full-campaign reducer. HQ and
/// field sessions are represented uniformly and each remains independently
/// replay-verifiable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedCampaignSessionV1 {
    pub ordinal: u32,
    pub run_id: OpaqueId,
    pub kind: CampaignSessionKindV1,
    pub content_subject: OfficialContentSubjectV1,
    pub campaign_aggregation_consent: CampaignAggregationConsentV1,
    /// Sole canonical replay; its bytes are both verifier input and public
    /// download content.
    pub replay: ReplayArtifactV1,
    pub build_manifest_sha256: Digest32,
    pub content_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    pub verification_request_sha256: Digest32,
    pub verification_result_sha256: Digest32,
    pub starting_campaign: ArtifactRefV1,
    pub final_campaign: ArtifactRefV1,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub named_participant_instance_count: u16,
    pub anonymous_participant_instance_count: u16,
    pub authenticated_participant_keys: Vec<PublicKey32>,
    pub active_simulation_ticks: u64,
    pub ransom_collected: u64,
    pub campaign_complete_evidence_sha256: Option<Digest32>,
}

impl VerifiedCampaignSessionV1 {
    pub(crate) fn score_delta(&self) -> i64 {
        i64::from(self.final_campaign_score) - i64::from(self.starting_campaign_score)
    }

    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        self.kind.validate()?;
        self.content_subject.validate()?;
        let subject_matches = match (&self.kind, &self.content_subject) {
            (
                CampaignSessionKindV1::FieldMission {
                    mission_id: session,
                },
                OfficialContentSubjectV1::FieldMission {
                    mission_id: content,
                },
            ) => session == content,
            (
                CampaignSessionKindV1::Headquarters { .. },
                OfficialContentSubjectV1::Headquarters { .. },
            ) => true,
            _ => false,
        };
        if !subject_matches {
            return Err(ValidationError::ClaimMismatch {
                field: "verified_campaign_session.content_subject",
            });
        }
        if self.campaign_aggregation_consent
            != CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verified_campaign_session.campaign_aggregation_consent",
            });
        }
        self.replay.validate()?;
        for artifact in [&self.starting_campaign, &self.final_campaign] {
            artifact.validate()?;
            if artifact.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
                return Err(ValidationError::ClaimMismatch {
                    field: "verified_campaign_session.campaign.media_type",
                });
            }
        }
        for digest in [
            self.build_manifest_sha256,
            self.content_manifest_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
            self.verification_request_sha256,
            self.verification_result_sha256,
        ] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "verified_campaign_session.proof_digest",
                });
            }
        }
        if self.score_delta() < 0 {
            return Err(ValidationError::InvalidOriginalScore);
        }
        if self
            .campaign_complete_evidence_sha256
            .is_some_and(|digest| digest.is_zero())
            || self
                .competition_manifest_sha256
                .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "verified_campaign_session.campaign_complete_evidence_sha256",
            });
        }
        if self.max_concurrent_players == 0
            || self.participant_instance_count < self.max_concurrent_players
            || usize::from(self.participant_instance_count)
                != self.authenticated_participant_keys.len()
            || self
                .named_participant_instance_count
                .checked_add(self.anonymous_participant_instance_count)
                != Some(self.participant_instance_count)
            || self.authenticated_participant_keys.is_empty()
            || self
                .authenticated_participant_keys
                .iter()
                .any(PublicKey32::is_zero)
            || !crate::validation::strictly_sorted(&self.authenticated_participant_keys)
        {
            return Err(ValidationError::InvalidParticipantClaims);
        }
        Ok(())
    }
}

/// Server/verifier-derived full-campaign aggregate. Construction requires a
/// canonical genesis, an unbroken ordered chain (including every HQ session),
/// and an independently verified campaign-complete terminal on the final run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedCampaignAggregateV1 {
    pub schema_version: u32,
    pub aggregate_request_sha256: Digest32,
    pub chain_id: OpaqueId,
    pub full_campaign_run_id: OpaqueId,
    pub campaign_complete_terminal_run_id: OpaqueId,
    pub campaign_complete_evidence_sha256: Digest32,
    pub sessions: Vec<VerifiedCampaignSessionV1>,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u32,
    pub named_participant_instance_count: u32,
    pub anonymous_participant_instance_count: u32,
    /// Every durable authenticated team identity across the independently
    /// verified sessions, including publicly anonymous participants.
    pub authenticated_participant_keys: Vec<PublicKey32>,
    pub campaign_controller_public_key: PublicKey32,
    pub campaign_content_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    pub canonical_genesis_campaign: ArtifactRefV1,
    pub final_campaign: ArtifactRefV1,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    pub active_simulation_ticks: u64,
    pub ransom_collected: u64,
}

impl VerifiedCampaignAggregateV1 {
    pub fn metrics(&self) -> RunMetricsV1 {
        RunMetricsV1 {
            original_score_delta: i64::from(self.final_campaign_score)
                - i64::from(self.starting_campaign_score),
            active_simulation_ticks: self.active_simulation_ticks,
            ransom_collected: self.ransom_collected,
        }
    }

    /// Applies the immutable ruleset semantics that cannot be checked from an
    /// aggregate in isolation. Callers must use this before publishing or
    /// reducing a full-campaign result.
    pub fn validate_against_ruleset(
        &self,
        published: &PublishedRulesetV1,
        campaign_content: &CampaignContentManifestV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        published.validate()?;
        campaign_content.validate()?;
        let manifest = &published.manifest;
        if self.ruleset_manifest_sha256 != published.ruleset_manifest_sha256
            || !manifest.admits_rules_config_digest(self.rules_config_sha256)
            || manifest
                .allowed_campaign_content_manifest_sha256
                .binary_search(&self.campaign_content_manifest_sha256)
                .is_err()
            || campaign_content
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "verified_campaign.campaign_content_manifest",
                })?
                != self.campaign_content_manifest_sha256
            || manifest
                .board_scopes
                .binary_search(&RulesetBoardScopeV1::FullCampaign)
                .is_err()
            || self.sessions.iter().any(|session| {
                manifest
                    .allowed_build_manifest_sha256
                    .binary_search(&session.build_manifest_sha256)
                    .is_err()
                    || manifest
                        .replay_schema_versions
                        .binary_search(&session.replay.replay_schema_version)
                        .is_err()
                    || manifest
                        .allowed_content_manifest_sha256
                        .binary_search(&session.content_manifest_sha256)
                        .is_err()
                    || campaign_content.content_for(&session.content_subject)
                        != Some(session.content_manifest_sha256)
                    || session.max_concurrent_players
                        < manifest
                            .participant_eligibility
                            .minimum_max_concurrent_players
                    || session.max_concurrent_players
                        > manifest
                            .participant_eligibility
                            .maximum_max_concurrent_players
                    || session.participant_instance_count
                        > manifest
                            .participant_eligibility
                            .maximum_participant_instances
                    || (!manifest.participant_eligibility.allow_single_player
                        && session.max_concurrent_players == 1)
                    || (!manifest.participant_eligibility.allow_multiplayer
                        && session.max_concurrent_players > 1)
                    || (manifest.participant_eligibility.anonymous_policy
                        == AnonymousParticipantPolicyV1::Forbidden
                        && session.anonymous_participant_instance_count != 0)
            })
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verified_campaign.ruleset_tuple",
            });
        }
        self.validate_roster_continuity(manifest.campaign_roster_continuity)
    }

    fn validate_roster_continuity(
        &self,
        policy: CampaignRosterContinuityV1,
    ) -> Result<(), ValidationError> {
        match policy {
            CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets => Ok(()),
            CampaignRosterContinuityV1::ExactSameAuthenticatedKeysEverySession
                if self.sessions.iter().all(|session| {
                    session.authenticated_participant_keys == self.authenticated_participant_keys
                }) =>
            {
                Ok(())
            }
            CampaignRosterContinuityV1::ExactSameAuthenticatedKeysEverySession => {
                Err(ValidationError::ClaimMismatch {
                    field: "verified_campaign.ruleset_roster_continuity",
                })
            }
        }
    }
}

impl Validate for VerifiedCampaignAggregateV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("VerifiedCampaignAggregateV1", self.schema_version)?;
        if self.max_concurrent_players == 0
            || self.participant_instance_count < u32::from(self.max_concurrent_players)
            || self
                .named_participant_instance_count
                .checked_add(self.anonymous_participant_instance_count)
                != Some(self.participant_instance_count)
            || self.authenticated_participant_keys.is_empty()
            || self
                .authenticated_participant_keys
                .iter()
                .any(PublicKey32::is_zero)
            || !crate::validation::strictly_sorted(&self.authenticated_participant_keys)
            || self.campaign_controller_public_key.is_zero()
        {
            return Err(ValidationError::InvalidParticipantClaims);
        }
        if self.sessions.is_empty() || self.sessions.len() > 4_096 {
            return Err(ValidationError::CountOutOfRange {
                field: "verified_campaign.sessions",
            });
        }
        if self
            .sessions
            .iter()
            .map(|session| &session.run_id)
            .collect::<BTreeSet<_>>()
            .len()
            != self.sessions.len()
        {
            return Err(ValidationError::Duplicate {
                field: "verified_campaign.sessions",
                value: "run_id".into(),
            });
        }
        let mut next_hq_sequence = 1_u32;
        for (ordinal, session) in self.sessions.iter().enumerate() {
            session.validate()?;
            if session.ordinal != ordinal as u32
                || session.rules_config_sha256 != self.rules_config_sha256
                || session.ruleset_manifest_sha256 != self.ruleset_manifest_sha256
                || session.competition_manifest_sha256 != self.competition_manifest_sha256
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "verified_campaign.session_identity",
                });
            }
            if session
                .authenticated_participant_keys
                .binary_search(&self.campaign_controller_public_key)
                .is_err()
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "verified_campaign.campaign_controller",
                });
            }
            if let CampaignSessionKindV1::Headquarters { hq_sequence } = &session.kind {
                if *hq_sequence != next_hq_sequence {
                    return Err(ValidationError::ClaimMismatch {
                        field: "verified_campaign.session_hq_sequence",
                    });
                }
                next_hq_sequence =
                    next_hq_sequence
                        .checked_add(1)
                        .ok_or(ValidationError::CountOutOfRange {
                            field: "verified_campaign.session_hq_sequence",
                        })?;
            }
        }
        let authenticated_participant_key_union = self
            .sessions
            .iter()
            .flat_map(|session| session.authenticated_participant_keys.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if authenticated_participant_key_union != self.authenticated_participant_keys {
            return Err(ValidationError::ClaimMismatch {
                field: "verified_campaign.participant_key_union",
            });
        }
        let first = self.sessions.first().expect("nonempty checked");
        let last = self.sessions.last().expect("nonempty checked");
        if first.starting_campaign != self.canonical_genesis_campaign
            || first.starting_campaign_score != self.starting_campaign_score
            || last.final_campaign != self.final_campaign
            || last.final_campaign_score != self.final_campaign_score
            || last.run_id != self.campaign_complete_terminal_run_id
            || last.campaign_complete_evidence_sha256
                != Some(self.campaign_complete_evidence_sha256)
            || self.sessions[..self.sessions.len() - 1]
                .iter()
                .any(|session| session.campaign_complete_evidence_sha256.is_some())
            || self.sessions.windows(2).any(|pair| {
                pair[0].final_campaign != pair[1].starting_campaign
                    || pair[0].final_campaign_score != pair[1].starting_campaign_score
            })
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verified_campaign.chain_continuity",
            });
        }
        let score_delta =
            i64::from(self.final_campaign_score) - i64::from(self.starting_campaign_score);
        let summed_score = self
            .sessions
            .iter()
            .try_fold(0_i64, |sum, session| sum.checked_add(session.score_delta()));
        let summed_ticks = self.sessions.iter().try_fold(0_u64, |sum, session| {
            sum.checked_add(session.active_simulation_ticks)
        });
        let summed_ransom = self.sessions.iter().try_fold(0_u64, |sum, session| {
            sum.checked_add(session.ransom_collected)
        });
        let summed_instances = self.sessions.iter().try_fold(0_u32, |sum, session| {
            sum.checked_add(u32::from(session.participant_instance_count))
        });
        let summed_named_instances = self.sessions.iter().try_fold(0_u32, |sum, session| {
            sum.checked_add(u32::from(session.named_participant_instance_count))
        });
        let summed_anonymous_instances = self.sessions.iter().try_fold(0_u32, |sum, session| {
            sum.checked_add(u32::from(session.anonymous_participant_instance_count))
        });
        let max_concurrent = self
            .sessions
            .iter()
            .map(|session| session.max_concurrent_players)
            .max();
        if score_delta < 0
            || summed_score != Some(score_delta)
            || summed_ticks != Some(self.active_simulation_ticks)
            || summed_ransom != Some(self.ransom_collected)
            || summed_instances != Some(self.participant_instance_count)
            || summed_named_instances != Some(self.named_participant_instance_count)
            || summed_anonymous_instances != Some(self.anonymous_participant_instance_count)
            || max_concurrent != Some(self.max_concurrent_players)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verified_campaign.aggregate_metrics",
            });
        }
        for digest in [
            self.aggregate_request_sha256,
            self.campaign_content_manifest_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
            self.campaign_complete_evidence_sha256,
        ] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "verified_campaign.proof_digest",
                });
            }
        }
        for artifact in [&self.canonical_genesis_campaign, &self.final_campaign] {
            artifact.validate()?;
            if artifact.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
                return Err(ValidationError::ClaimMismatch {
                    field: "verified_campaign.campaign.media_type",
                });
            }
        }
        if self
            .competition_manifest_sha256
            .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "verified_campaign.competition_manifest_sha256",
            });
        }
        Ok(())
    }
}

// Keep the existing public facade while the worker contract has its own owner.
pub use crate::verification_result::{
    VerificationInfrastructureFailureCodeV1, VerificationInfrastructureFailureV1,
    VerificationRejectionCodeV1, VerificationRejectionV1, VerificationResultV1,
    VerificationStatusV1, VerifierAdmissionFailureCodeV1, VerifierWorkerOutputV1,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignChainReceiptV1 {
    pub schema_version: u32,
    pub chain_id: OpaqueId,
    pub predecessor_run_id: OpaqueId,
    pub predecessor_verification_sha256: Digest32,
    pub expected_starting_campaign: ArtifactRefV1,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    pub campaign_content_manifest_sha256: Digest32,
    pub expected_max_concurrent_players: u16,
    /// Publicly disclosed durable identities on the receipt. Anonymous display
    /// never makes a participant keyless in the authoritative submission and
    /// verifier result.
    pub participant_public_keys: Vec<PublicKey32>,
    pub campaign_controller_public_key: PublicKey32,
    pub state: CampaignChainStateV1,
}

impl Validate for CampaignChainReceiptV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("CampaignChainReceiptV1", self.schema_version)?;
        self.expected_starting_campaign.validate()?;
        if self.expected_starting_campaign.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_chain_receipt.expected_starting_campaign.media_type",
            });
        }
        for digest in [
            self.predecessor_verification_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
            self.campaign_content_manifest_sha256,
        ] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "campaign_chain_receipt.identity_digest",
                });
            }
        }
        if self
            .competition_manifest_sha256
            .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "campaign_chain_receipt.competition_manifest_sha256",
            });
        }
        if self.expected_max_concurrent_players == 0
            || self.expected_max_concurrent_players > MAX_REPLAY_SEATS_V1
            || self.participant_public_keys.is_empty()
            || self.participant_public_keys.len()
                > usize::from(self.expected_max_concurrent_players)
            || self
                .participant_public_keys
                .iter()
                .any(PublicKey32::is_zero)
            || !crate::validation::strictly_sorted(&self.participant_public_keys)
            || self.campaign_controller_public_key.is_zero()
            || self
                .participant_public_keys
                .binary_search(&self.campaign_controller_public_key)
                .is_err()
        {
            return Err(ValidationError::InvalidParticipantClaims);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CampaignChainStateV1 {
    Active,
    Complete { full_campaign_run_id: OpaqueId },
}

#[derive(Serialize)]
struct UsernameUpdateSignable<'a> {
    schema_version: u32,
    username_challenge_id: &'a OpaqueId,
    username_challenge_nonce: ChallengeNonce32,
    public_key: PublicKey32,
    username: &'a str,
}

/// Separately signed mutable username operation. Usernames are intentionally
/// not part of run submissions and are not required to be unique.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsernameUpdateEnvelopeV1 {
    pub schema_version: u32,
    pub username_challenge_id: OpaqueId,
    pub username_challenge_nonce: ChallengeNonce32,
    pub public_key: PublicKey32,
    pub username: String,
    pub signature: Signature64,
}

impl UsernameUpdateEnvelopeV1 {
    /// Validate every signed claim while intentionally ignoring the signature
    /// field. This is the safe pre-signing entry point for native and WASM
    /// identity bridges.
    pub fn validate_signing_claim(&self) -> Result<(), ValidationError> {
        crate::validation::schema("UsernameUpdateEnvelopeV1", self.schema_version)?;
        crate::validation::nonzero(
            "username_update.username_challenge_nonce",
            &self.username_challenge_nonce,
        )?;
        crate::validation::nonzero("username_update.public_key", &self.public_key)?;
        crate::validation::text("username_update.username", &self.username, 48)
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        crate::canonical::domain_separated_bytes(
            USERNAME_UPDATE_SIGNATURE_DOMAIN_V1,
            &UsernameUpdateSignable {
                schema_version: self.schema_version,
                username_challenge_id: &self.username_challenge_id,
                username_challenge_nonce: self.username_challenge_nonce,
                public_key: self.public_key,
                username: &self.username,
            },
        )
    }
}

impl Validate for UsernameUpdateEnvelopeV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.validate_signing_claim()?;
        crate::validation::nonzero("username_update.signature", &self.signature)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests;
