//! Authenticated multiplayer evidence for ranked leaderboard runs.
//!
//! This module is deliberately separate from presentation and HTTP upload.
//! It owns the cryptographic session genesis, durable participant claims, and
//! the lossless mapping from authenticated transport lifecycle observations to
//! dense replay ordinals. If any observation cannot be matched exactly to the
//! authoritative replay, callers get an error and the run remains browse-only.

use ed25519_dalek::{Signer, SigningKey};
use robin_engine::player_command::PlayerCommand;
use robin_engine::replay::ReplayData;
#[cfg(test)]
use robin_run_protocol::InitialStateExpectationV1;
use robin_run_protocol::{
    ArtifactRefV1, BoardMetricV1, CampaignAggregationConsentV1, CampaignChainReceiptV1,
    CampaignChainStateV1, CampaignContinuationAuthorizationClaimV1,
    CampaignContinuationPreflightGrantV1, CampaignContinuationPreflightRequestClaimV1,
    CampaignContinuationPreflightRequestV1, CampaignRosterContinuityV1, CanonicalDocument as _,
    CompetitionRunGrantV1, Digest32, FreshRunPreflightGrantV1, FreshRunPreflightRequestClaimV1,
    FreshRunPreflightRequestV1, FreshRunScopeV1, NamedSeatJoinAttestationV1, NamedSeatJoinClaimV1,
    OpaqueId, ParticipantClaimV1, ParticipantPublicDisclosureV1, PublicKey32,
    RANKED_CAMPAIGN_MEDIA_TYPE_V1, RankedSessionConfigV1, ReplaySeatLifecycleEventV1,
    ReplaySeatLifecycleKindV1, ReplaySessionGenesisClaimV1, ReplaySessionGenesisV1,
    ReplaySessionTranscriptV1, SCHEMA_VERSION_V1, ScopeRequestV1, Signature64,
    SignatureAlgorithmV1, SubmissionArtifactsV1, SubmissionEnvelopeV1, SubmissionOfferRequestV1,
    SubmissionOfferV1, Validate as _,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum RankedSessionError {
    #[error("invalid ranked-session document: {0}")]
    InvalidDocument(String),
    #[error("ranked-session identity does not match the authenticated transport")]
    TransportIdentityMismatch,
    #[error("ranked-session signature verification failed")]
    InvalidSignature,
    #[error("ranked-session participant is not admitted: {0}")]
    ParticipantNotAdmitted(String),
    #[error("ranked-session lifecycle does not match the authoritative replay: {0}")]
    ReplayLifecycleMismatch(String),
    #[error("ranked-session counter overflow: {0}")]
    CounterOverflow(&'static str),
    #[error("ranked-session wire document exceeds {maximum} bytes")]
    DocumentTooLarge { maximum: usize },
    #[error("ranked-session wire document is not canonical JSON")]
    NonCanonicalDocument,
}

/// The control-frame bound used for ranked lifecycle documents. Replays and
/// campaign bytes never travel here; only signed claims, transcript metadata,
/// offers, and artifact digests do.
pub const MAX_RANKED_SESSION_WIRE_DOCUMENT_BYTES: usize = 64 * 1024;

/// Exact request/authority response retained locally before frame zero. The
/// request is not reconstructed from the grant: retaining both makes the host
/// and every peer compare the authority response with the same signed local
/// tuple that was submitted for admission.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RankedRunPreflightAdmissionV1 {
    Fresh {
        request: FreshRunPreflightRequestV1,
        grant: FreshRunPreflightGrantV1,
    },
    CampaignContinuation {
        request: CampaignContinuationPreflightRequestV1,
        grant: CampaignContinuationPreflightGrantV1,
    },
}

impl RankedRunPreflightAdmissionV1 {
    pub fn host_public_key(&self) -> PublicKey32 {
        match self {
            Self::Fresh { request, .. } => request.claim.host_public_key,
            Self::CampaignContinuation { request, .. } => request.claim.host_public_key,
        }
    }

    pub fn session_identity(&self) -> (Digest32, Digest32, robin_run_protocol::ChallengeNonce32) {
        match self {
            Self::Fresh { request, .. } => (
                request.claim.replay_session_id,
                request.claim.host_participant_instance_id,
                request.claim.host_nonce,
            ),
            Self::CampaignContinuation { request, .. } => (
                request.claim.replay_session_id,
                request.claim.host_participant_instance_id,
                request.claim.host_nonce,
            ),
        }
    }

    fn ranked_session(&self) -> &RankedSessionConfigV1 {
        match self {
            Self::Fresh { request, .. } => &request.claim.ranked_session,
            Self::CampaignContinuation { request, .. } => &request.claim.ranked_session,
        }
    }

    fn grant_authority_public_key(&self) -> PublicKey32 {
        match self {
            Self::Fresh { grant, .. } => grant.claim.grant_authority_public_key,
            Self::CampaignContinuation { grant, .. } => grant.claim.grant_authority_public_key,
        }
    }

    fn validity_interval(&self) -> (u64, u64) {
        match self {
            Self::Fresh { grant, .. } => (
                grant.claim.admitted_at_unix_ms,
                grant.claim.expires_at_unix_ms,
            ),
            Self::CampaignContinuation { grant, .. } => (
                grant.claim.admitted_at_unix_ms,
                grant.claim.expires_at_unix_ms,
            ),
        }
    }

    pub fn scope_request(&self) -> ScopeRequestV1 {
        match self {
            Self::Fresh { request, .. } => match request.claim.scope {
                FreshRunScopeV1::IndividualLevel => ScopeRequestV1::IndividualLevel,
                FreshRunScopeV1::CampaignGenesis => ScopeRequestV1::CampaignGenesis,
            },
            Self::CampaignContinuation { request, .. } => ScopeRequestV1::CampaignContinuation {
                chain_id: request.claim.chain_id.clone(),
                predecessor_run_id: request.claim.predecessor_run_id.clone(),
            },
        }
    }

    pub fn campaign_controller_public_key(&self) -> Option<PublicKey32> {
        match self {
            Self::Fresh { .. } => None,
            Self::CampaignContinuation { request, .. } => {
                Some(request.claim.campaign_controller_public_key)
            }
        }
    }
}

/// Locally retained predecessor/roster tuple used to build the only typed
/// continuation preflight claim. The builder below contributes fresh session
/// entropy so callers cannot accidentally reuse identifiers across missions.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignContinuationPreflightSetupV1 {
    pub campaign_controller_public_key: PublicKey32,
    pub max_concurrent_players: u16,
    pub participant_public_keys: Vec<PublicKey32>,
    pub chain_id: OpaqueId,
    pub predecessor_run_id: OpaqueId,
    pub predecessor_verification_sha256: Digest32,
}

/// Authenticated lobby identity snapshot retained before any ranked preflight
/// is requested. The host transport constructs this from durable handshake
/// identities; callers cannot substitute presentation names or seat labels.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedPreflightLobbyV1 {
    pub host_public_key: PublicKey32,
    pub max_concurrent_players: u16,
    pub participant_public_keys: Vec<PublicKey32>,
}

/// Host broadcast asking authenticated peers to select the one locally held
/// active campaign receipt which exactly matches the intended ranked lobby.
/// It contains no authority and grants no capability by itself.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignContinuationReceiptSelectionRequestV1 {
    pub schema_version: u32,
    pub request_nonce: robin_run_protocol::ChallengeNonce32,
    pub lobby: RankedPreflightLobbyV1,
    pub roster_continuity: CampaignRosterContinuityV1,
    pub starting_campaign: ArtifactRefV1,
    pub ranked_session: RankedSessionConfigV1,
}

impl robin_run_protocol::Validate for CampaignContinuationReceiptSelectionRequestV1 {
    fn validate(&self) -> Result<(), robin_run_protocol::ValidationError> {
        if self.schema_version != SCHEMA_VERSION_V1
            || self.request_nonce.is_zero()
            || self.lobby.host_public_key.is_zero()
            || self.lobby.max_concurrent_players == 0
            || self.lobby.participant_public_keys.is_empty()
            || self.lobby.participant_public_keys.len()
                != usize::from(self.lobby.max_concurrent_players)
            || self
                .lobby
                .participant_public_keys
                .iter()
                .any(PublicKey32::is_zero)
            || self
                .lobby
                .participant_public_keys
                .windows(2)
                .any(|keys| keys[0] >= keys[1])
            || self
                .lobby
                .participant_public_keys
                .binary_search(&self.lobby.host_public_key)
                .is_err()
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "continuation_receipt_selection.lobby",
            });
        }
        self.starting_campaign.validate()?;
        self.ranked_session.validate()?;
        if self.starting_campaign.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1
            || self.starting_campaign.sha256 != self.ranked_session.starting_campaign_sha256
            || self.starting_campaign.byte_length
                != self.ranked_session.starting_campaign_byte_length
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "continuation_receipt_selection.starting_campaign",
            });
        }
        Ok(())
    }
}

impl CampaignContinuationReceiptSelectionRequestV1 {
    pub fn from_lobby(
        lobby: RankedPreflightLobbyV1,
        ranked_session: RankedSessionConfigV1,
        roster_continuity: CampaignRosterContinuityV1,
    ) -> Result<Self, RankedSessionError> {
        let request = Self {
            schema_version: SCHEMA_VERSION_V1,
            request_nonce: random_nonce(),
            roster_continuity,
            starting_campaign: ArtifactRefV1 {
                sha256: ranked_session.starting_campaign_sha256,
                byte_length: ranked_session.starting_campaign_byte_length,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_string(),
            },
            lobby,
            ranked_session,
        };
        request.validate().map_err(invalid_document)?;
        Ok(request)
    }
}

/// Controller-selected active receipt returned over its authenticated seat.
/// The request is repeated exactly so a delayed response cannot cross lobbies
/// or mission attempts.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignContinuationReceiptSelectionV1 {
    pub request: CampaignContinuationReceiptSelectionRequestV1,
    pub receipt: CampaignChainReceiptV1,
}

impl robin_run_protocol::Validate for CampaignContinuationReceiptSelectionV1 {
    fn validate(&self) -> Result<(), robin_run_protocol::ValidationError> {
        self.request.validate()?;
        self.receipt.validate()?;
        let ranked = &self.request.ranked_session;
        let receipt = &self.receipt;
        if receipt.state != CampaignChainStateV1::Active
            || receipt.expected_starting_campaign != self.request.starting_campaign
            || receipt.rules_config_sha256 != ranked.rules_config_sha256
            || receipt.ruleset_manifest_sha256 != ranked.ruleset_manifest_sha256
            || receipt.competition_manifest_sha256 != ranked.competition_manifest_sha256
            || Some(receipt.campaign_content_manifest_sha256)
                != ranked.campaign_content_manifest_sha256
            || receipt.expected_max_concurrent_players != self.request.lobby.max_concurrent_players
            || receipt
                .participant_public_keys
                .binary_search(&receipt.campaign_controller_public_key)
                .is_err()
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "continuation_receipt_selection.receipt",
            });
        }
        if self.request.roster_continuity
            == CampaignRosterContinuityV1::ExactSameAuthenticatedKeysEverySession
            && receipt.participant_public_keys != self.request.lobby.participant_public_keys
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "continuation_receipt_selection.exact_roster_continuity",
            });
        }
        Ok(())
    }
}

impl CampaignContinuationReceiptSelectionV1 {
    pub fn preflight_setup(
        &self,
    ) -> Result<CampaignContinuationPreflightSetupV1, RankedSessionError> {
        self.validate().map_err(invalid_document)?;
        Ok(CampaignContinuationPreflightSetupV1 {
            campaign_controller_public_key: self.receipt.campaign_controller_public_key,
            max_concurrent_players: self.request.lobby.max_concurrent_players,
            participant_public_keys: self.request.lobby.participant_public_keys.clone(),
            chain_id: self.receipt.chain_id.clone(),
            predecessor_run_id: self.receipt.predecessor_run_id.clone(),
            predecessor_verification_sha256: self.receipt.predecessor_verification_sha256,
        })
    }
}

/// Exhaustive authenticated response from one remote lobby identity. Explicit
/// negative responses let the host select CampaignGenesis without waiting for
/// a network timeout when no controller owns a matching active receipt.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum CampaignContinuationReceiptSelectionResponseV1 {
    Selected {
        selection: CampaignContinuationReceiptSelectionV1,
    },
    NoMatchingReceipt {
        request: CampaignContinuationReceiptSelectionRequestV1,
        responder_public_key: PublicKey32,
    },
}

impl CampaignContinuationReceiptSelectionResponseV1 {
    pub fn request(&self) -> &CampaignContinuationReceiptSelectionRequestV1 {
        match self {
            Self::Selected { selection } => &selection.request,
            Self::NoMatchingReceipt { request, .. } => request,
        }
    }

    pub fn responder_public_key(&self) -> PublicKey32 {
        match self {
            Self::Selected { selection } => selection.receipt.campaign_controller_public_key,
            Self::NoMatchingReceipt {
                responder_public_key,
                ..
            } => *responder_public_key,
        }
    }
}

impl robin_run_protocol::Validate for CampaignContinuationReceiptSelectionResponseV1 {
    fn validate(&self) -> Result<(), robin_run_protocol::ValidationError> {
        match self {
            Self::Selected { selection } => selection.validate(),
            Self::NoMatchingReceipt {
                request,
                responder_public_key,
            } => {
                request.validate()?;
                if responder_public_key.is_zero()
                    || *responder_public_key == request.lobby.host_public_key
                    || request
                        .lobby
                        .participant_public_keys
                        .binary_search(responder_public_key)
                        .is_err()
                {
                    return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                        field: "continuation_receipt_selection.no_match_responder",
                    });
                }
                Ok(())
            }
        }
    }
}

/// Explicit bootstrap input for the currently supported official ranked lane.
/// A package-backed Spellforge/custom mission is never inferred rankable from
/// a Lua or package digest: the verifier does not yet possess the complete
/// custom content needed to reproduce it.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialRankedSessionSetupV1 {
    pub ranked_session: RankedSessionConfigV1,
    pub custom_package_present: bool,
    pub run_preflight: RankedRunPreflightAdmissionV1,
    pub run_preflight_grant_public_key: PublicKey32,
    /// Current time established by the same HTTPS authority path used to load
    /// immutable ranked manifests. It is captured once during setup so every
    /// peer evaluates the grant at the pre-frame boundary.
    pub trusted_now_unix_ms: u64,
}

/// Host-published setup projection. Trusted time is deliberately absent:
/// every peer supplies its own pre-frame clock when converting this wire
/// document into local admission state.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialRankedSessionWireSetupV1 {
    pub ranked_session: RankedSessionConfigV1,
    pub custom_package_present: bool,
    pub run_preflight: RankedRunPreflightAdmissionV1,
    pub run_preflight_grant_public_key: PublicKey32,
}

/// Client-local trust anchors and deterministic inputs prepared independently
/// of the host's wire setup.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialRankedSessionExpectationV1 {
    pub ranked_session: RankedSessionConfigV1,
    pub custom_package_present: bool,
    pub run_preflight_grant_public_key: PublicKey32,
}

impl OfficialRankedSessionWireSetupV1 {
    pub fn from_local_setup(
        setup: &OfficialRankedSessionSetupV1,
    ) -> Result<Self, RankedSessionError> {
        setup.validate().map_err(invalid_document)?;
        Ok(Self {
            ranked_session: setup.ranked_session.clone(),
            custom_package_present: setup.custom_package_present,
            run_preflight: setup.run_preflight.clone(),
            run_preflight_grant_public_key: setup.run_preflight_grant_public_key,
        })
    }

    pub fn prepare_for_authenticated_peer(
        self,
        expectation: &OfficialRankedSessionExpectationV1,
        authenticated_host_public_key: PublicKey32,
        local_public_key: PublicKey32,
        local_trusted_now_unix_ms: u64,
    ) -> Result<OfficialRankedSessionSetupV1, RankedSessionError> {
        if self.ranked_session != expectation.ranked_session
            || self.custom_package_present != expectation.custom_package_present
            || self.run_preflight_grant_public_key != expectation.run_preflight_grant_public_key
            || self.run_preflight.host_public_key() != authenticated_host_public_key
            || local_public_key.is_zero()
        {
            return Err(RankedSessionError::TransportIdentityMismatch);
        }
        if let RankedRunPreflightAdmissionV1::CampaignContinuation { request, .. } =
            &self.run_preflight
            && request
                .claim
                .participant_public_keys
                .binary_search(&local_public_key)
                .is_err()
        {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "local durable identity is absent from continuation preflight roster".to_string(),
            ));
        }
        let setup = OfficialRankedSessionSetupV1 {
            ranked_session: self.ranked_session,
            custom_package_present: self.custom_package_present,
            run_preflight: self.run_preflight,
            run_preflight_grant_public_key: self.run_preflight_grant_public_key,
            trusted_now_unix_ms: local_trusted_now_unix_ms,
        };
        // Preserve the precise local-time failure at this trust boundary;
        // `Validate` intentionally projects it to a stable claim-mismatch
        // field for generic protocol consumers.
        validate_run_preflight(&setup)?;
        setup.validate().map_err(invalid_document)?;
        Ok(setup)
    }
}

impl robin_run_protocol::Validate for OfficialRankedSessionSetupV1 {
    fn validate(&self) -> Result<(), robin_run_protocol::ValidationError> {
        self.ranked_session.validate()?;
        if self.custom_package_present {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "official_ranked_session.custom_package_present",
            });
        }
        if !robin_run_protocol::official_content_subjects_v1(self.ranked_session.content_edition)
            .contains(&self.ranked_session.content_subject)
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "official_ranked_session.content_subject",
            });
        }
        validate_run_preflight(self).map_err(|_| {
            robin_run_protocol::ValidationError::ClaimMismatch {
                field: "official_ranked_session.run_preflight",
            }
        })?;
        Ok(())
    }
}

impl OfficialRankedSessionSetupV1 {
    pub fn scope_request(&self) -> ScopeRequestV1 {
        self.run_preflight.scope_request()
    }

    pub fn campaign_controller_public_key(&self) -> Option<PublicKey32> {
        self.run_preflight.campaign_controller_public_key()
    }
}

pub fn encode_ranked_wire_document(
    value: &(impl serde::Serialize + ?Sized),
) -> Result<Vec<u8>, RankedSessionError> {
    let bytes = robin_run_protocol::canonical_json_bytes(value).map_err(invalid_document)?;
    if bytes.len() > MAX_RANKED_SESSION_WIRE_DOCUMENT_BYTES {
        return Err(RankedSessionError::DocumentTooLarge {
            maximum: MAX_RANKED_SESSION_WIRE_DOCUMENT_BYTES,
        });
    }
    Ok(bytes)
}

pub fn decode_ranked_wire_document<T>(bytes: &[u8]) -> Result<T, RankedSessionError>
where
    T: serde::de::DeserializeOwned + serde::Serialize + robin_run_protocol::Validate,
{
    let document = decode_canonical_ranked_wire_document::<T>(bytes)?;
    document.validate().map_err(invalid_document)?;
    Ok(document)
}

/// Decode only a bounded canonical document. This narrower primitive exists
/// for small protocol leaves such as `ParticipantSignatureV1` whose enclosing
/// signed request owns semantic validation and which intentionally do not
/// implement the protocol-wide `Validate` trait.
pub fn decode_canonical_ranked_wire_document<T>(bytes: &[u8]) -> Result<T, RankedSessionError>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    if bytes.len() > MAX_RANKED_SESSION_WIRE_DOCUMENT_BYTES {
        return Err(RankedSessionError::DocumentTooLarge {
            maximum: MAX_RANKED_SESSION_WIRE_DOCUMENT_BYTES,
        });
    }
    let document: T = serde_json::from_slice(bytes).map_err(invalid_document)?;
    if encode_ranked_wire_document(&document)? != bytes {
        return Err(RankedSessionError::NonCanonicalDocument);
    }
    Ok(document)
}

fn invalid_document(error: impl std::fmt::Display) -> RankedSessionError {
    RankedSessionError::InvalidDocument(error.to_string())
}

fn public_key(key: &SigningKey) -> PublicKey32 {
    PublicKey32::from_bytes(key.verifying_key().to_bytes())
}

fn signature(key: &SigningKey, bytes: &[u8]) -> Signature64 {
    Signature64::from_bytes(key.sign(bytes).to_bytes())
}

fn verify_signature(
    key: PublicKey32,
    bytes: &[u8],
    signature: Signature64,
) -> Result<(), RankedSessionError> {
    robin_run_protocol::verify_ed25519_strict(key.as_bytes(), signature.as_bytes(), bytes)
        .map_err(|_| RankedSessionError::InvalidSignature)
}

fn validate_run_preflight(setup: &OfficialRankedSessionSetupV1) -> Result<(), RankedSessionError> {
    setup.ranked_session.validate().map_err(invalid_document)?;
    if setup.run_preflight_grant_public_key.is_zero() || setup.trusted_now_unix_ms == 0 {
        return Err(RankedSessionError::InvalidDocument(
            "ranked run preflight is missing its pinned authority or trusted time".to_string(),
        ));
    }
    if setup.run_preflight.ranked_session() != &setup.ranked_session {
        return Err(RankedSessionError::InvalidDocument(
            "ranked run preflight names a different prepared session".to_string(),
        ));
    }
    if setup.run_preflight.grant_authority_public_key() != setup.run_preflight_grant_public_key {
        return Err(RankedSessionError::InvalidDocument(
            "ranked run preflight grant is not signed by the ruleset-pinned authority".to_string(),
        ));
    }
    let (admitted_at, expires_at) = setup.run_preflight.validity_interval();
    if setup.trusted_now_unix_ms < admitted_at || setup.trusted_now_unix_ms > expires_at {
        return Err(RankedSessionError::InvalidDocument(
            "ranked run preflight grant is not valid at the trusted setup time".to_string(),
        ));
    }

    match &setup.run_preflight {
        RankedRunPreflightAdmissionV1::Fresh { request, grant } => {
            grant.validate_request(request).map_err(invalid_document)?;
            verify_signature(
                request.claim.host_public_key,
                &request.signing_bytes().map_err(invalid_document)?,
                request.host_signature,
            )?;
            verify_signature(
                grant.claim.grant_authority_public_key,
                &grant.signing_bytes().map_err(invalid_document)?,
                grant.authority_signature,
            )?;
        }
        RankedRunPreflightAdmissionV1::CampaignContinuation { request, grant } => {
            grant.validate_request(request).map_err(invalid_document)?;
            verify_signature(
                request.claim.host_public_key,
                &request
                    .claim
                    .host_signing_bytes()
                    .map_err(invalid_document)?,
                request.host_signature,
            )?;
            verify_signature(
                request.claim.campaign_controller_public_key,
                &request
                    .claim
                    .controller_signing_bytes()
                    .map_err(invalid_document)?,
                request.controller_signature,
            )?;
            verify_signature(
                grant.claim.grant_authority_public_key,
                &grant.signing_bytes().map_err(invalid_document)?,
                grant.authority_signature,
            )?;
        }
    }
    Ok(())
}
fn random_digest() -> Digest32 {
    // Session authentication runs outside simulation and requires fresh cryptographic entropy.
    #[allow(clippy::disallowed_methods)]
    Digest32::from_bytes(rand::random())
}

fn random_nonce() -> robin_run_protocol::ChallengeNonce32 {
    // Session authentication runs outside simulation and requires fresh cryptographic entropy.
    #[allow(clippy::disallowed_methods)]
    robin_run_protocol::ChallengeNonce32::from_bytes(rand::random())
}

/// Validate the host-authored genesis against both the authenticated iroh
/// endpoint and the exact locally prepared ranked inputs.
pub fn validate_session_genesis(
    genesis: &ReplaySessionGenesisV1,
    authenticated_host_endpoint: [u8; 32],
    expected_ranked_session: &RankedSessionConfigV1,
) -> Result<(), RankedSessionError> {
    genesis.validate().map_err(invalid_document)?;
    expected_ranked_session
        .validate()
        .map_err(invalid_document)?;
    if *genesis.claim.host_public_key.as_bytes() != authenticated_host_endpoint
        || &genesis.claim.ranked_session != expected_ranked_session
    {
        return Err(RankedSessionError::TransportIdentityMismatch);
    }
    verify_signature(
        genesis.claim.host_public_key,
        &genesis.signing_bytes().map_err(invalid_document)?,
        genesis.host_signature,
    )
}

/// Strict official admission validation. In addition to the host signature,
/// this compares the wire genesis with the exact locally retained preflight
/// request and authority grant. A peer never accepts a grant merely because a
/// host embedded it in a signed genesis.
pub fn validate_official_session_genesis(
    genesis: &ReplaySessionGenesisV1,
    authenticated_host_endpoint: [u8; 32],
    expected_setup: &OfficialRankedSessionSetupV1,
) -> Result<(), RankedSessionError> {
    expected_setup.validate().map_err(invalid_document)?;
    validate_session_genesis(
        genesis,
        authenticated_host_endpoint,
        &expected_setup.ranked_session,
    )?;
    if genesis.claim.host_public_key != expected_setup.run_preflight.host_public_key() {
        return Err(RankedSessionError::TransportIdentityMismatch);
    }
    let (replay_session_id, host_participant_instance_id, host_nonce) =
        expected_setup.run_preflight.session_identity();
    if genesis.claim.replay_session_id != replay_session_id
        || genesis.claim.host_participant_instance_id != host_participant_instance_id
        || genesis.claim.host_nonce != host_nonce
    {
        return Err(RankedSessionError::InvalidDocument(
            "ranked genesis replaced the locally preflighted session identity".to_string(),
        ));
    }
    let grants_match = match &expected_setup.run_preflight {
        RankedRunPreflightAdmissionV1::Fresh { grant, .. } => {
            genesis.claim.fresh_run_preflight_grant.as_ref() == Some(grant)
                && genesis
                    .claim
                    .campaign_continuation_preflight_grant
                    .is_none()
        }
        RankedRunPreflightAdmissionV1::CampaignContinuation { grant, .. } => {
            genesis.claim.campaign_continuation_preflight_grant.as_ref() == Some(grant)
                && genesis.claim.fresh_run_preflight_grant.is_none()
        }
    };
    if !grants_match {
        return Err(RankedSessionError::InvalidDocument(
            "ranked genesis replaced or omitted the locally preflighted authority grant"
                .to_string(),
        ));
    }
    Ok(())
}

/// Sign the closed named-seat claim with the same durable native identity used
/// by leaderboard submission. The transport endpoint is separately bound in
/// the claim and may only differ for the browser relay client.
pub fn sign_named_seat_join(
    key: &SigningKey,
    claim: NamedSeatJoinClaimV1,
) -> Result<NamedSeatJoinAttestationV1, RankedSessionError> {
    claim.validate().map_err(invalid_document)?;
    if claim.public_key != public_key(key) {
        return Err(RankedSessionError::TransportIdentityMismatch);
    }
    let signed = NamedSeatJoinAttestationV1 {
        signature: signature(key, &claim.signing_bytes().map_err(invalid_document)?),
        claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
    };
    signed.validate().map_err(invalid_document)?;
    Ok(signed)
}

/// Complete a browser join after the isolated durable signer returns the
/// signature for `claim.signing_bytes()`. This is intentionally typed: the
/// caller cannot route an arbitrary payload through this API.
pub fn complete_browser_named_seat_join(
    claim: NamedSeatJoinClaimV1,
    signature: [u8; 64],
) -> Result<NamedSeatJoinAttestationV1, RankedSessionError> {
    claim.validate().map_err(invalid_document)?;
    let attestation = NamedSeatJoinAttestationV1 {
        claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
        signature: Signature64::from_bytes(signature),
    };
    verify_named_seat_join(
        &attestation,
        attestation.claim.transport_endpoint_id.as_bytes(),
    )?;
    Ok(attestation)
}

/// Verify a participant's durable identity and its binding to the exact
/// authenticated transport endpoint. Browser durable and transport keys may
/// differ; native callers pass the same key for both.
pub fn verify_named_seat_join(
    attestation: &NamedSeatJoinAttestationV1,
    authenticated_transport_endpoint: &[u8; 32],
) -> Result<(), RankedSessionError> {
    attestation.validate().map_err(invalid_document)?;
    if attestation.claim.transport_endpoint_id.as_bytes() != authenticated_transport_endpoint {
        return Err(RankedSessionError::TransportIdentityMismatch);
    }
    verify_signature(
        attestation.claim.public_key,
        &attestation.signing_bytes().map_err(invalid_document)?,
        attestation.signature,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingAdmissionKind {
    Fresh,
    Reconnect,
}

#[derive(Clone, Debug)]
struct LifecycleObservation {
    seat: u16,
    participant_instance_id: Digest32,
    lifecycle: ReplaySeatLifecycleKindV1,
}

#[derive(Clone, Debug)]
struct ParticipantState {
    claim: ParticipantClaimV1,
    last_connection_epoch: u32,
    connected: bool,
}

/// Host-owned ranked-session state. Construction freezes and signs the exact
/// prepared input identity before authoritative replay begins.
pub struct RankedSessionHost {
    genesis: ReplaySessionGenesisV1,
    participants: BTreeMap<u16, ParticipantState>,
    owner_seats: BTreeMap<PublicKey32, u16>,
    observations: Vec<LifecycleObservation>,
    pending_admission: Option<(PendingAdmissionKind, NamedSeatJoinClaimV1)>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedSessionEvidenceV1 {
    pub session_genesis: ReplaySessionGenesisV1,
    pub participant_claims: Vec<ParticipantClaimV1>,
    pub replay_session_transcript: ReplaySessionTranscriptV1,
}

/// Exact client authority retained after mission inputs are prepared but
/// before the host accepts the durable seat claim.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedSessionClientAdmissionV1 {
    pub expected_setup: OfficialRankedSessionSetupV1,
    pub authenticated_host_endpoint: PublicKey32,
    pub local_public_key: PublicKey32,
    pub local_transport_endpoint: PublicKey32,
}

impl robin_run_protocol::Validate for RankedSessionClientAdmissionV1 {
    fn validate(&self) -> Result<(), robin_run_protocol::ValidationError> {
        self.expected_setup.validate()?;
        if self.authenticated_host_endpoint.is_zero()
            || self.local_public_key.is_zero()
            || self.local_transport_endpoint.is_zero()
        {
            return Err(robin_run_protocol::ValidationError::Zero {
                field: "ranked_client_admission.identity",
            });
        }
        Ok(())
    }
}

impl RankedSessionClientAdmissionV1 {
    pub fn new_official(
        setup: OfficialRankedSessionSetupV1,
        authenticated_host_endpoint: PublicKey32,
        local_public_key: PublicKey32,
        local_transport_endpoint: PublicKey32,
    ) -> Result<Self, RankedSessionError> {
        setup.validate().map_err(invalid_document)?;
        if setup.run_preflight.host_public_key() != authenticated_host_endpoint {
            return Err(RankedSessionError::TransportIdentityMismatch);
        }
        if let RankedRunPreflightAdmissionV1::CampaignContinuation { request, .. } =
            &setup.run_preflight
            && request
                .claim
                .participant_public_keys
                .binary_search(&local_public_key)
                .is_err()
        {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "local durable identity is absent from the continuation preflight roster"
                    .to_string(),
            ));
        }
        let admission = Self {
            expected_setup: setup,
            authenticated_host_endpoint,
            local_public_key,
            local_transport_endpoint,
        };
        admission.validate().map_err(invalid_document)?;
        Ok(admission)
    }
}

/// Client-side authenticated ranked session. It contains public evidence only;
/// the durable secret remains confined to the native key store or browser
/// signer vault.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedSessionClientV1 {
    pub admission: RankedSessionClientAdmissionV1,
    pub local_seat: u16,
    pub session_genesis: ReplaySessionGenesisV1,
    pub participant_claims: Vec<ParticipantClaimV1>,
}

impl robin_run_protocol::Validate for RankedSessionClientV1 {
    fn validate(&self) -> Result<(), robin_run_protocol::ValidationError> {
        self.admission.validate()?;
        if self.local_seat == 0 {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "ranked_client.local_seat",
            });
        }
        validate_official_session_genesis(
            &self.session_genesis,
            *self.admission.authenticated_host_endpoint.as_bytes(),
            &self.admission.expected_setup,
        )
        .map_err(|_| robin_run_protocol::ValidationError::ClaimMismatch {
            field: "ranked_client.session_genesis",
        })?;
        validate_participant_roster(&self.session_genesis, &self.participant_claims).map_err(
            |_| robin_run_protocol::ValidationError::ClaimMismatch {
                field: "ranked_client.participant_claims",
            },
        )?;
        let local = self
            .participant_claims
            .iter()
            .find(|claim| claim.seat == self.local_seat)
            .ok_or(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "ranked_client.local_participant",
            })?;
        let join = local.join_attestation.as_ref().ok_or(
            robin_run_protocol::ValidationError::ClaimMismatch {
                field: "ranked_client.local_join_attestation",
            },
        )?;
        if local.public_key != self.admission.local_public_key
            || join.claim.transport_endpoint_id != self.admission.local_transport_endpoint
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "ranked_client.local_identity",
            });
        }
        Ok(())
    }
}

/// Irreversible eligibility owner used by transport/bootstrap integration.
/// Ranking failures never fabricate evidence or abort otherwise-compatible
/// gameplay: the state moves once to `BrowseOnly` and stays there.
pub enum RankedSessionLifecycle {
    /// The transport exists, but the exact prepared mission inputs are not yet
    /// available. This state must be resolved before authoritative simulation.
    AwaitingPreparedInputs,
    ClientAdmissionPending(Box<RankedSessionClientAdmissionV1>),
    Ranked(Box<RankedSessionHost>),
    RankedClient(Box<RankedSessionClientV1>),
    BrowseOnly {
        reason: String,
    },
}

pub type SharedRankedSessionLifecycle = Arc<Mutex<RankedSessionLifecycle>>;

impl RankedSessionLifecycle {
    pub fn awaiting_prepared_inputs() -> Self {
        Self::AwaitingPreparedInputs
    }

    pub fn ranked(session: RankedSessionHost) -> Self {
        Self::Ranked(Box::new(session))
    }

    pub fn browse_only(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        assert!(
            !reason.trim().is_empty(),
            "browse-only reason must be explicit"
        );
        Self::BrowseOnly { reason }
    }

    pub fn downgrade(&mut self, reason: impl Into<String>) {
        if matches!(self, Self::BrowseOnly { .. }) {
            return;
        }
        *self = Self::browse_only(reason);
    }

    pub fn browse_only_reason(&self) -> Option<&str> {
        match self {
            Self::AwaitingPreparedInputs
            | Self::ClientAdmissionPending(_)
            | Self::Ranked(_)
            | Self::RankedClient(_) => None,
            Self::BrowseOnly { reason } => Some(reason),
        }
    }

    pub fn install_ranked(&mut self, session: RankedSessionHost) -> Result<(), RankedSessionError> {
        if !matches!(self, Self::AwaitingPreparedInputs) {
            return Err(RankedSessionError::InvalidDocument(
                "ranked session can only be installed once before simulation".to_string(),
            ));
        }
        *self = Self::ranked(session);
        Ok(())
    }

    pub fn install_client_admission(
        &mut self,
        admission: RankedSessionClientAdmissionV1,
    ) -> Result<(), RankedSessionError> {
        admission.validate().map_err(invalid_document)?;
        if !matches!(self, Self::AwaitingPreparedInputs) {
            return Err(RankedSessionError::InvalidDocument(
                "ranked client admission can only be installed once before simulation".to_string(),
            ));
        }
        *self = Self::ClientAdmissionPending(Box::new(admission));
        Ok(())
    }

    pub fn accept_ranked_client(
        &mut self,
        local_seat: u16,
        session_genesis: ReplaySessionGenesisV1,
        participant_claims: Vec<ParticipantClaimV1>,
    ) -> Result<(), RankedSessionError> {
        let Self::ClientAdmissionPending(admission) = self else {
            return Err(RankedSessionError::InvalidDocument(
                "ranked client acceptance has no pending local admission".to_string(),
            ));
        };
        let client = RankedSessionClientV1 {
            admission: (**admission).clone(),
            local_seat,
            session_genesis,
            participant_claims,
        };
        client.validate().map_err(invalid_document)?;
        *self = Self::RankedClient(Box::new(client));
        Ok(())
    }

    pub fn update_ranked_client_roster(
        &mut self,
        session_genesis: &ReplaySessionGenesisV1,
        participant_claims: Vec<ParticipantClaimV1>,
    ) -> Result<(), RankedSessionError> {
        let Self::RankedClient(client) = self else {
            return Err(RankedSessionError::InvalidDocument(
                "ranked roster update has no accepted client session".to_string(),
            ));
        };
        if &client.session_genesis != session_genesis
            || !client
                .participant_claims
                .iter()
                .all(|old| participant_claims.iter().any(|new| new == old))
        {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "ranked roster update replaced authenticated session evidence".to_string(),
            ));
        }
        let updated = RankedSessionClientV1 {
            admission: client.admission.clone(),
            local_seat: client.local_seat,
            session_genesis: client.session_genesis.clone(),
            participant_claims,
        };
        updated.validate().map_err(invalid_document)?;
        **client = updated;
        Ok(())
    }

    pub fn is_awaiting_prepared_inputs(&self) -> bool {
        matches!(self, Self::AwaitingPreparedInputs)
    }

    pub fn ranked_mut(&mut self) -> Option<&mut RankedSessionHost> {
        match self {
            Self::Ranked(session) => Some(session),
            Self::AwaitingPreparedInputs
            | Self::ClientAdmissionPending(_)
            | Self::RankedClient(_)
            | Self::BrowseOnly { .. } => None,
        }
    }

    pub fn ranked_session(&self) -> Option<&RankedSessionHost> {
        match self {
            Self::Ranked(session) => Some(session),
            Self::AwaitingPreparedInputs
            | Self::ClientAdmissionPending(_)
            | Self::RankedClient(_)
            | Self::BrowseOnly { .. } => None,
        }
    }

    pub fn ranked_client(&self) -> Option<&RankedSessionClientV1> {
        match self {
            Self::RankedClient(client) => Some(client),
            Self::AwaitingPreparedInputs
            | Self::ClientAdmissionPending(_)
            | Self::Ranked(_)
            | Self::BrowseOnly { .. } => None,
        }
    }

    pub fn client_admission(&self) -> Option<&RankedSessionClientAdmissionV1> {
        match self {
            Self::ClientAdmissionPending(admission) => Some(admission),
            Self::AwaitingPreparedInputs
            | Self::Ranked(_)
            | Self::RankedClient(_)
            | Self::BrowseOnly { .. } => None,
        }
    }

    pub fn evidence_for_replay(
        &self,
        replay: &ReplayData,
    ) -> Result<Option<RankedSessionEvidenceV1>, RankedSessionError> {
        let session = match self {
            Self::Ranked(session) => session,
            Self::BrowseOnly { .. } => return Ok(None),
            Self::AwaitingPreparedInputs | Self::ClientAdmissionPending(_) => {
                return Err(RankedSessionError::InvalidDocument(
                    "ranked session was never resolved before replay finalization".to_string(),
                ));
            }
            Self::RankedClient(_) => {
                return Err(RankedSessionError::InvalidDocument(
                    "ranked client cannot author host submission evidence".to_string(),
                ));
            }
        };
        Ok(Some(RankedSessionEvidenceV1 {
            session_genesis: session.genesis.clone(),
            participant_claims: session.participant_claims(),
            replay_session_transcript: session.transcript_for_replay(replay)?,
        }))
    }
}

impl RankedSessionHost {
    pub fn prepare_fresh_run_preflight_claim(
        host_public_key: PublicKey32,
        ranked_session: RankedSessionConfigV1,
        scope: FreshRunScopeV1,
    ) -> Result<FreshRunPreflightRequestClaimV1, RankedSessionError> {
        ranked_session.validate().map_err(invalid_document)?;
        if host_public_key.is_zero() {
            return Err(RankedSessionError::InvalidDocument(
                "fresh-run preflight host identity is zero".to_string(),
            ));
        }
        let claim = FreshRunPreflightRequestClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            request_nonce: random_nonce(),
            host_public_key,
            replay_session_id: random_digest(),
            host_participant_instance_id: random_digest(),
            host_nonce: random_nonce(),
            scope,
            starting_campaign: ArtifactRefV1 {
                sha256: ranked_session.starting_campaign_sha256,
                byte_length: ranked_session.starting_campaign_byte_length,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_string(),
            },
            ranked_session,
        };
        claim.validate().map_err(invalid_document)?;
        Ok(claim)
    }

    pub fn prepare_campaign_continuation_preflight_claim(
        host_public_key: PublicKey32,
        ranked_session: RankedSessionConfigV1,
        continuation: CampaignContinuationPreflightSetupV1,
    ) -> Result<CampaignContinuationPreflightRequestClaimV1, RankedSessionError> {
        ranked_session.validate().map_err(invalid_document)?;
        let claim = CampaignContinuationPreflightRequestClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            request_nonce: random_nonce(),
            host_public_key,
            campaign_controller_public_key: continuation.campaign_controller_public_key,
            replay_session_id: random_digest(),
            host_participant_instance_id: random_digest(),
            host_nonce: random_nonce(),
            max_concurrent_players: continuation.max_concurrent_players,
            participant_public_keys: continuation.participant_public_keys,
            chain_id: continuation.chain_id,
            predecessor_run_id: continuation.predecessor_run_id,
            predecessor_verification_sha256: continuation.predecessor_verification_sha256,
            starting_campaign: ArtifactRefV1 {
                sha256: ranked_session.starting_campaign_sha256,
                byte_length: ranked_session.starting_campaign_byte_length,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_string(),
            },
            ranked_session,
        };
        claim.validate().map_err(invalid_document)?;
        Ok(claim)
    }
    /// Build the exact claim an isolated durable-identity signer must sign.
    /// No secret key crosses this boundary. The session identifiers have
    /// already been committed by the signed preflight request and are copied
    /// exactly rather than regenerated after the authority grants admission.
    pub fn prepare_official_genesis_claim(
        host_public_key: PublicKey32,
        network_protocol_version: u32,
        setup: OfficialRankedSessionSetupV1,
    ) -> Result<ReplaySessionGenesisClaimV1, RankedSessionError> {
        setup.validate().map_err(invalid_document)?;
        if setup.run_preflight.host_public_key() != host_public_key {
            return Err(RankedSessionError::TransportIdentityMismatch);
        }
        if network_protocol_version == 0 {
            return Err(RankedSessionError::InvalidDocument(
                "network protocol version is zero".to_string(),
            ));
        }
        let (replay_session_id, host_participant_instance_id, host_nonce) =
            setup.run_preflight.session_identity();
        let (fresh_run_preflight_grant, campaign_continuation_preflight_grant) =
            match setup.run_preflight {
                RankedRunPreflightAdmissionV1::Fresh { grant, .. } => (Some(grant), None),
                RankedRunPreflightAdmissionV1::CampaignContinuation { grant, .. } => {
                    (None, Some(grant))
                }
            };
        let claim = ReplaySessionGenesisClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            network_protocol_version,
            host_public_key,
            replay_session_id,
            host_participant_instance_id,
            host_nonce,
            ranked_session: setup.ranked_session,
            fresh_run_preflight_grant,
            campaign_continuation_preflight_grant,
            competition_run_grant: None,
        };
        claim.validate().map_err(invalid_document)?;
        Ok(claim)
    }
    /// Bootstrap a host only from explicit official-content eligibility. Game
    /// setup should use this entry point; `new` remains the lower-level signed
    /// document constructor used after this gate and by focused protocol tests.
    pub fn new_official(
        host_key: &SigningKey,
        network_protocol_version: u32,
        setup: OfficialRankedSessionSetupV1,
    ) -> Result<Self, RankedSessionError> {
        setup.validate().map_err(invalid_document)?;
        let claim = Self::prepare_official_genesis_claim(
            public_key(host_key),
            network_protocol_version,
            setup.clone(),
        )?;
        let genesis = ReplaySessionGenesisV1 {
            host_signature: signature(host_key, &claim.signing_bytes().map_err(invalid_document)?),
            claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        validate_official_session_genesis(&genesis, host_key.verifying_key().to_bytes(), &setup)?;
        Self::from_signed_genesis(genesis)
    }

    pub fn new(
        host_key: &SigningKey,
        network_protocol_version: u32,
        ranked_session: RankedSessionConfigV1,
    ) -> Result<Self, RankedSessionError> {
        Self::new_with_competition_grant(host_key, network_protocol_version, ranked_session, None)
    }

    pub fn new_with_competition_grant(
        host_key: &SigningKey,
        network_protocol_version: u32,
        ranked_session: RankedSessionConfigV1,
        competition_run_grant: Option<CompetitionRunGrantV1>,
    ) -> Result<Self, RankedSessionError> {
        let claim = Self::prepare_genesis_claim(
            public_key(host_key),
            network_protocol_version,
            ranked_session,
            competition_run_grant,
        )?;
        let genesis = ReplaySessionGenesisV1 {
            host_signature: signature(host_key, &claim.signing_bytes().map_err(invalid_document)?),
            claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        Self::from_signed_genesis(genesis)
    }

    fn prepare_genesis_claim(
        host_public_key: PublicKey32,
        network_protocol_version: u32,
        ranked_session: RankedSessionConfigV1,
        competition_run_grant: Option<CompetitionRunGrantV1>,
    ) -> Result<ReplaySessionGenesisClaimV1, RankedSessionError> {
        ranked_session.validate().map_err(invalid_document)?;
        if network_protocol_version == 0 {
            return Err(RankedSessionError::InvalidDocument(
                "network protocol version is zero".to_string(),
            ));
        }
        let host_participant_instance_id = random_digest();
        let claim = ReplaySessionGenesisClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            network_protocol_version,
            host_public_key,
            replay_session_id: random_digest(),
            host_participant_instance_id,
            host_nonce: random_nonce(),
            ranked_session,
            fresh_run_preflight_grant: None,
            campaign_continuation_preflight_grant: None,
            competition_run_grant,
        };
        claim.validate().map_err(invalid_document)?;
        Ok(claim)
    }

    /// Construct host lifecycle state from a typed signature returned by an
    /// isolated durable signer. This verifies the signature before creating
    /// the host participant or recording any lifecycle observation.
    pub fn from_signed_genesis(
        genesis: ReplaySessionGenesisV1,
    ) -> Result<Self, RankedSessionError> {
        validate_session_genesis(
            &genesis,
            *genesis.claim.host_public_key.as_bytes(),
            &genesis.claim.ranked_session,
        )?;
        let host_public_key = genesis.claim.host_public_key;
        let host_participant_instance_id = genesis.claim.host_participant_instance_id;
        let host_claim = ParticipantClaimV1 {
            seat: 0,
            participant_instance_id: host_participant_instance_id,
            public_key: host_public_key,
            public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
            join_attestation: None,
        };
        let mut participants = BTreeMap::new();
        participants.insert(
            0,
            ParticipantState {
                claim: host_claim,
                last_connection_epoch: 0,
                connected: true,
            },
        );
        Ok(Self {
            genesis,
            participants,
            owner_seats: BTreeMap::from([(host_public_key, 0)]),
            observations: vec![LifecycleObservation {
                seat: 0,
                participant_instance_id: host_participant_instance_id,
                lifecycle: ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: 0,
                },
            }],
            pending_admission: None,
        })
    }

    pub fn genesis(&self) -> &ReplaySessionGenesisV1 {
        &self.genesis
    }

    pub fn participant_claims(&self) -> Vec<ParticipantClaimV1> {
        self.participants
            .values()
            .map(|participant| participant.claim.clone())
            .collect()
    }

    /// Prepare the only claim a newly assigned seat may sign. The caller must
    /// not publish the deterministic ConnectSeat command until `admit_join`
    /// succeeds for the returned claim.
    pub fn prepare_join(
        &mut self,
        seat: u16,
        durable_public_key: PublicKey32,
        authenticated_transport_endpoint: PublicKey32,
        host_endpoint_id: PublicKey32,
    ) -> Result<NamedSeatJoinClaimV1, RankedSessionError> {
        if self.pending_admission.is_some() {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "another ranked join admission is already pending".to_string(),
            ));
        }
        if seat == 0 || self.participants.contains_key(&seat) {
            return Err(RankedSessionError::ParticipantNotAdmitted(format!(
                "seat {seat} is already reserved"
            )));
        }
        if self.owner_seats.contains_key(&durable_public_key) {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "durable identity already owns a seat".to_string(),
            ));
        }
        if let Some(grant) = &self.genesis.claim.campaign_continuation_preflight_grant
            && grant
                .claim
                .participant_public_keys
                .binary_search(&durable_public_key)
                .is_err()
        {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "durable identity is absent from the authority-admitted continuation roster"
                    .to_string(),
            ));
        }
        if durable_public_key.is_zero()
            || authenticated_transport_endpoint.is_zero()
            || host_endpoint_id != self.genesis.claim.host_public_key
        {
            return Err(RankedSessionError::TransportIdentityMismatch);
        }
        let join_event_ordinal = u32::try_from(self.observations.len())
            .map_err(|_| RankedSessionError::CounterOverflow("join event ordinal"))?;
        let claim = NamedSeatJoinClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            session_genesis_sha256: self.genesis.canonical_digest().map_err(invalid_document)?,
            public_key: durable_public_key,
            transport_endpoint_id: authenticated_transport_endpoint,
            host_endpoint_id,
            replay_session_id: self.genesis.claim.replay_session_id,
            participant_instance_id: random_digest(),
            seat,
            connection_epoch: 0,
            join_event_ordinal,
            mission_id: self.genesis.claim.ranked_session.mission_id.clone(),
            content_manifest_sha256: self.genesis.claim.ranked_session.content_manifest_sha256,
            rules_config_sha256: self.genesis.claim.ranked_session.rules_config_sha256,
            ruleset_manifest_sha256: self.genesis.claim.ranked_session.ruleset_manifest_sha256,
            competition_manifest_sha256: self
                .genesis
                .claim
                .ranked_session
                .competition_manifest_sha256,
            host_nonce: self.genesis.claim.host_nonce,
        };
        claim.validate().map_err(invalid_document)?;
        self.pending_admission = Some((PendingAdmissionKind::Fresh, claim.clone()));
        Ok(claim)
    }

    /// Prepare an authenticated reconnect for the same durable seat owner.
    /// The participant instance remains stable while the connection epoch and
    /// replay lifecycle ordinal advance exactly once.
    pub fn prepare_reconnect(
        &mut self,
        seat: u16,
        durable_public_key: PublicKey32,
        authenticated_transport_endpoint: PublicKey32,
        host_endpoint_id: PublicKey32,
    ) -> Result<NamedSeatJoinClaimV1, RankedSessionError> {
        if self.pending_admission.is_some() {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "another ranked admission is already pending".to_string(),
            ));
        }
        let participant = self.participants.get(&seat).ok_or_else(|| {
            RankedSessionError::ParticipantNotAdmitted(format!("unknown seat {seat}"))
        })?;
        if seat == 0
            || participant.connected
            || participant.claim.public_key != durable_public_key
            || authenticated_transport_endpoint.is_zero()
            || host_endpoint_id != self.genesis.claim.host_public_key
        {
            return Err(RankedSessionError::TransportIdentityMismatch);
        }
        let connection_epoch = participant
            .last_connection_epoch
            .checked_add(1)
            .ok_or(RankedSessionError::CounterOverflow("connection epoch"))?;
        let join_event_ordinal = u32::try_from(self.observations.len())
            .map_err(|_| RankedSessionError::CounterOverflow("join event ordinal"))?;
        let claim = NamedSeatJoinClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            session_genesis_sha256: self.genesis.canonical_digest().map_err(invalid_document)?,
            public_key: durable_public_key,
            transport_endpoint_id: authenticated_transport_endpoint,
            host_endpoint_id,
            replay_session_id: self.genesis.claim.replay_session_id,
            participant_instance_id: participant.claim.participant_instance_id,
            seat,
            connection_epoch,
            join_event_ordinal,
            mission_id: self.genesis.claim.ranked_session.mission_id.clone(),
            content_manifest_sha256: self.genesis.claim.ranked_session.content_manifest_sha256,
            rules_config_sha256: self.genesis.claim.ranked_session.rules_config_sha256,
            ruleset_manifest_sha256: self.genesis.claim.ranked_session.ruleset_manifest_sha256,
            competition_manifest_sha256: self
                .genesis
                .claim
                .ranked_session
                .competition_manifest_sha256,
            host_nonce: self.genesis.claim.host_nonce,
        };
        claim.validate().map_err(invalid_document)?;
        self.pending_admission = Some((PendingAdmissionKind::Reconnect, claim.clone()));
        Ok(claim)
    }

    /// Cancel the one serialized pending admission. Transport callers use
    /// this before irreversibly downgrading to browse-only after an unavailable
    /// identity or rejected attestation.
    pub fn cancel_pending_join(&mut self) {
        self.pending_admission = None;
    }

    pub fn admit_join(
        &mut self,
        attestation: NamedSeatJoinAttestationV1,
        authenticated_transport_endpoint: [u8; 32],
        public_disclosure: ParticipantPublicDisclosureV1,
    ) -> Result<(), RankedSessionError> {
        verify_named_seat_join(&attestation, &authenticated_transport_endpoint)?;
        let claim = &attestation.claim;
        if !matches!(
            self.pending_admission.as_ref(),
            Some((PendingAdmissionKind::Fresh, pending)) if pending == claim
        ) {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "join attestation does not consume the exact pending challenge".to_string(),
            ));
        }
        let expected_genesis = self.genesis.canonical_digest().map_err(invalid_document)?;
        if claim.session_genesis_sha256 != expected_genesis
            || claim.host_endpoint_id != self.genesis.claim.host_public_key
            || claim.replay_session_id != self.genesis.claim.replay_session_id
            || claim.host_nonce != self.genesis.claim.host_nonce
            || claim.join_event_ordinal
                != u32::try_from(self.observations.len()).unwrap_or(u32::MAX)
            || claim.connection_epoch != 0
            || claim.mission_id != self.genesis.claim.ranked_session.mission_id
            || claim.content_manifest_sha256
                != self.genesis.claim.ranked_session.content_manifest_sha256
            || claim.rules_config_sha256 != self.genesis.claim.ranked_session.rules_config_sha256
            || claim.ruleset_manifest_sha256
                != self.genesis.claim.ranked_session.ruleset_manifest_sha256
            || claim.competition_manifest_sha256
                != self
                    .genesis
                    .claim
                    .ranked_session
                    .competition_manifest_sha256
            || claim.seat == 0
            || self.participants.contains_key(&claim.seat)
            || self.owner_seats.contains_key(&claim.public_key)
        {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "join claim does not match the current ranked session boundary".to_string(),
            ));
        }
        let participant = ParticipantClaimV1 {
            seat: claim.seat,
            participant_instance_id: claim.participant_instance_id,
            public_key: claim.public_key,
            public_disclosure,
            join_attestation: Some(attestation),
        };
        self.owner_seats
            .insert(participant.public_key, participant.seat);
        self.observations.push(LifecycleObservation {
            seat: participant.seat,
            participant_instance_id: participant.participant_instance_id,
            lifecycle: ReplaySeatLifecycleKindV1::Connected {
                connection_epoch: 0,
            },
        });
        self.participants.insert(
            participant.seat,
            ParticipantState {
                claim: participant,
                last_connection_epoch: 0,
                connected: true,
            },
        );
        self.pending_admission = None;
        Ok(())
    }

    pub fn admit_reconnect(
        &mut self,
        attestation: NamedSeatJoinAttestationV1,
        authenticated_transport_endpoint: [u8; 32],
    ) -> Result<(), RankedSessionError> {
        verify_named_seat_join(&attestation, &authenticated_transport_endpoint)?;
        let claim = &attestation.claim;
        if !matches!(
            self.pending_admission.as_ref(),
            Some((PendingAdmissionKind::Reconnect, pending)) if pending == claim
        ) {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "reconnect attestation does not consume the exact pending challenge".to_string(),
            ));
        }
        let expected_genesis = self.genesis.canonical_digest().map_err(invalid_document)?;
        let participant = self.participants.get_mut(&claim.seat).ok_or_else(|| {
            RankedSessionError::ParticipantNotAdmitted(format!("unknown seat {}", claim.seat))
        })?;
        let expected_epoch = participant
            .last_connection_epoch
            .checked_add(1)
            .ok_or(RankedSessionError::CounterOverflow("connection epoch"))?;
        if participant.connected
            || claim.seat == 0
            || claim.public_key != participant.claim.public_key
            || claim.participant_instance_id != participant.claim.participant_instance_id
            || claim.connection_epoch != expected_epoch
            || claim.join_event_ordinal
                != u32::try_from(self.observations.len()).unwrap_or(u32::MAX)
            || claim.session_genesis_sha256 != expected_genesis
            || claim.host_endpoint_id != self.genesis.claim.host_public_key
            || claim.replay_session_id != self.genesis.claim.replay_session_id
            || claim.host_nonce != self.genesis.claim.host_nonce
            || claim.mission_id != self.genesis.claim.ranked_session.mission_id
            || claim.content_manifest_sha256
                != self.genesis.claim.ranked_session.content_manifest_sha256
            || claim.rules_config_sha256 != self.genesis.claim.ranked_session.rules_config_sha256
            || claim.ruleset_manifest_sha256
                != self.genesis.claim.ranked_session.ruleset_manifest_sha256
            || claim.competition_manifest_sha256
                != self
                    .genesis
                    .claim
                    .ranked_session
                    .competition_manifest_sha256
        {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "reconnect claim does not match the current ranked session boundary".to_string(),
            ));
        }
        participant.last_connection_epoch = expected_epoch;
        participant.connected = true;
        self.observations.push(LifecycleObservation {
            seat: claim.seat,
            participant_instance_id: claim.participant_instance_id,
            lifecycle: ReplaySeatLifecycleKindV1::Connected {
                connection_epoch: expected_epoch,
            },
        });
        self.pending_admission = None;
        Ok(())
    }

    /// Record an authenticated transport disconnect before publishing the
    /// matching host-authored DisconnectSeat command.
    pub fn observe_disconnect(&mut self, seat: u16) -> Result<(), RankedSessionError> {
        let participant = self.participants.get_mut(&seat).ok_or_else(|| {
            RankedSessionError::ParticipantNotAdmitted(format!("unknown seat {seat}"))
        })?;
        if seat == 0 || !participant.connected {
            return Err(RankedSessionError::ParticipantNotAdmitted(format!(
                "seat {seat} cannot disconnect at this boundary"
            )));
        }
        participant.connected = false;
        self.observations.push(LifecycleObservation {
            seat,
            participant_instance_id: participant.claim.participant_instance_id,
            lifecycle: ReplaySeatLifecycleKindV1::Disconnected,
        });
        Ok(())
    }

    /// Bind the transport observations to the exact dense replay ordinals.
    /// Any missing, extra, reordered, or differently targeted lifecycle
    /// command makes the run unrankable instead of fabricating a transcript.
    pub fn transcript_for_replay(
        &self,
        replay: &ReplayData,
    ) -> Result<ReplaySessionTranscriptV1, RankedSessionError> {
        let mut commands = Vec::new();
        for replay_ordinal in 0..replay.frame_count() {
            let frame = replay.frame(replay_ordinal).ok_or_else(|| {
                RankedSessionError::ReplayLifecycleMismatch(format!(
                    "replay frame {replay_ordinal} is absent"
                ))
            })?;
            for input in frame
                .input
                .commands
                .iter()
                .chain(&frame.input.post_commands)
            {
                match &input.player_input().command {
                    PlayerCommand::ConnectSeat { player_id, .. } => {
                        commands.push((replay_ordinal, u16::from(player_id.0), true))
                    }
                    PlayerCommand::DisconnectSeat { player_id } => {
                        commands.push((replay_ordinal, u16::from(player_id.0), false))
                    }
                    _ => {}
                }
            }
        }
        let expected = self.observations.iter().skip(1).collect::<Vec<_>>();
        if commands.len() != expected.len() {
            return Err(RankedSessionError::ReplayLifecycleMismatch(format!(
                "replay has {} guest lifecycle commands, authenticated transport observed {}",
                commands.len(),
                expected.len()
            )));
        }
        let mut events = vec![ReplaySeatLifecycleEventV1 {
            event_ordinal: 0,
            replay_ordinal: 0,
            seat: 0,
            participant_instance_id: self.genesis.claim.host_participant_instance_id,
            lifecycle: ReplaySeatLifecycleKindV1::Connected {
                connection_epoch: 0,
            },
        }];
        for (index, ((replay_ordinal, seat, connected), observation)) in
            commands.into_iter().zip(expected).enumerate()
        {
            let observation_connected = matches!(
                observation.lifecycle,
                ReplaySeatLifecycleKindV1::Connected { .. }
            );
            if seat != observation.seat || connected != observation_connected {
                return Err(RankedSessionError::ReplayLifecycleMismatch(format!(
                    "lifecycle command {} targets seat {seat} connected={connected}, authenticated observation targets seat {} connected={observation_connected}",
                    index + 1,
                    observation.seat
                )));
            }
            events.push(ReplaySeatLifecycleEventV1 {
                event_ordinal: u32::try_from(index + 1)
                    .map_err(|_| RankedSessionError::CounterOverflow("event ordinal"))?,
                replay_ordinal,
                seat,
                participant_instance_id: observation.participant_instance_id,
                lifecycle: observation.lifecycle,
            });
        }
        let participant_instance_count = u16::try_from(self.participants.len())
            .map_err(|_| RankedSessionError::CounterOverflow("participant instance count"))?;
        let max_concurrent_players = derive_max_concurrent(&events)?;
        let transcript = ReplaySessionTranscriptV1 {
            schema_version: SCHEMA_VERSION_V1,
            session_genesis_sha256: self.genesis.canonical_digest().map_err(invalid_document)?,
            replay_session_id: self.genesis.claim.replay_session_id,
            host_participant_instance_id: self.genesis.claim.host_participant_instance_id,
            participant_instance_count,
            max_concurrent_players,
            events,
        };
        transcript.validate().map_err(invalid_document)?;
        replay
            .validate_ranked_command_admission(&transcript)
            .map_err(RankedSessionError::ReplayLifecycleMismatch)?;
        Ok(transcript)
    }
}

fn derive_max_concurrent(events: &[ReplaySeatLifecycleEventV1]) -> Result<u16, RankedSessionError> {
    let mut occupied = BTreeSet::new();
    let mut maximum = 0usize;
    for event in events {
        match event.lifecycle {
            ReplaySeatLifecycleKindV1::Connected { .. } => {
                if !occupied.insert(event.seat) {
                    return Err(RankedSessionError::ReplayLifecycleMismatch(format!(
                        "seat {} connected twice",
                        event.seat
                    )));
                }
                maximum = maximum.max(occupied.len());
            }
            ReplaySeatLifecycleKindV1::Disconnected => {
                if !occupied.remove(&event.seat) {
                    return Err(RankedSessionError::ReplayLifecycleMismatch(format!(
                        "seat {} disconnected while vacant",
                        event.seat
                    )));
                }
            }
        }
    }
    u16::try_from(maximum).map_err(|_| RankedSessionError::CounterOverflow("concurrent players"))
}

/// Verify every durable participant claim against the signed genesis. The
/// authenticated transport endpoint for the local client is checked earlier;
/// this validates the complete roster each peer later binds to its replay.
pub fn validate_participant_roster(
    genesis: &ReplaySessionGenesisV1,
    participant_claims: &[ParticipantClaimV1],
) -> Result<(), RankedSessionError> {
    genesis.validate().map_err(invalid_document)?;
    verify_signature(
        genesis.claim.host_public_key,
        &genesis.signing_bytes().map_err(invalid_document)?,
        genesis.host_signature,
    )?;
    if participant_claims.is_empty() {
        return Err(RankedSessionError::ParticipantNotAdmitted(
            "ranked participant roster is empty".to_string(),
        ));
    }
    if !participant_claims
        .windows(2)
        .all(|pair| pair[0].seat < pair[1].seat)
    {
        return Err(RankedSessionError::ParticipantNotAdmitted(
            "ranked participant roster is not in strict seat order".to_string(),
        ));
    }
    let genesis_digest = genesis.canonical_digest().map_err(invalid_document)?;
    let mut seats = BTreeSet::new();
    let mut public_keys = BTreeSet::new();
    let mut instances = BTreeSet::new();
    for participant in participant_claims {
        if !seats.insert(participant.seat)
            || !public_keys.insert(participant.public_key)
            || !instances.insert(participant.participant_instance_id)
        {
            return Err(RankedSessionError::ParticipantNotAdmitted(
                "ranked participant roster contains a duplicate seat, key, or instance".to_string(),
            ));
        }
        if participant.seat == 0 {
            if participant.public_key != genesis.claim.host_public_key
                || participant.participant_instance_id != genesis.claim.host_participant_instance_id
                || participant.join_attestation.is_some()
            {
                return Err(RankedSessionError::ParticipantNotAdmitted(
                    "ranked host roster claim does not match genesis".to_string(),
                ));
            }
            continue;
        }
        let attestation = participant.join_attestation.as_ref().ok_or_else(|| {
            RankedSessionError::ParticipantNotAdmitted(format!(
                "ranked guest seat {} has no join attestation",
                participant.seat
            ))
        })?;
        verify_named_seat_join(
            attestation,
            attestation.claim.transport_endpoint_id.as_bytes(),
        )?;
        let claim = &attestation.claim;
        if participant.seat != claim.seat
            || participant.public_key != claim.public_key
            || participant.participant_instance_id != claim.participant_instance_id
            || claim.connection_epoch != 0
            || claim.session_genesis_sha256 != genesis_digest
            || claim.host_endpoint_id != genesis.claim.host_public_key
            || claim.replay_session_id != genesis.claim.replay_session_id
            || claim.host_nonce != genesis.claim.host_nonce
            || claim.mission_id != genesis.claim.ranked_session.mission_id
            || claim.content_manifest_sha256 != genesis.claim.ranked_session.content_manifest_sha256
            || claim.rules_config_sha256 != genesis.claim.ranked_session.rules_config_sha256
            || claim.ruleset_manifest_sha256 != genesis.claim.ranked_session.ruleset_manifest_sha256
            || claim.competition_manifest_sha256
                != genesis.claim.ranked_session.competition_manifest_sha256
        {
            return Err(RankedSessionError::ParticipantNotAdmitted(format!(
                "ranked guest seat {} does not match genesis",
                participant.seat
            )));
        }
    }
    if !seats.contains(&0) {
        return Err(RankedSessionError::ParticipantNotAdmitted(
            "ranked participant roster has no host seat".to_string(),
        ));
    }
    Ok(())
}

/// Independently bind a received transcript to this peer's locally recorded
/// authoritative replay. Participant identities come from the authenticated
/// roster; the replay itself remains identity-free.
pub fn validate_transcript_against_local_replay(
    replay: &ReplayData,
    genesis: &ReplaySessionGenesisV1,
    participant_claims: &[ParticipantClaimV1],
    transcript: &ReplaySessionTranscriptV1,
) -> Result<(), RankedSessionError> {
    validate_participant_roster(genesis, participant_claims)?;
    transcript.validate().map_err(invalid_document)?;
    let genesis_digest = genesis.canonical_digest().map_err(invalid_document)?;
    if transcript.session_genesis_sha256 != genesis_digest
        || transcript.replay_session_id != genesis.claim.replay_session_id
        || transcript.host_participant_instance_id != genesis.claim.host_participant_instance_id
        || transcript.participant_instance_count
            != u16::try_from(participant_claims.len()).unwrap_or(u16::MAX)
    {
        return Err(RankedSessionError::ReplayLifecycleMismatch(
            "ranked transcript does not match the locally admitted roster".to_string(),
        ));
    }
    let participants_by_seat = participant_claims
        .iter()
        .map(|claim| (claim.seat, claim.participant_instance_id))
        .collect::<BTreeMap<_, _>>();
    for event in &transcript.events {
        if participants_by_seat.get(&event.seat) != Some(&event.participant_instance_id) {
            return Err(RankedSessionError::ReplayLifecycleMismatch(format!(
                "transcript event {} is not owned by its authenticated seat",
                event.event_ordinal
            )));
        }
    }

    let mut local_commands = Vec::new();
    for replay_ordinal in 0..replay.frame_count() {
        let frame = replay.frame(replay_ordinal).ok_or_else(|| {
            RankedSessionError::ReplayLifecycleMismatch(format!(
                "local replay frame {replay_ordinal} is absent"
            ))
        })?;
        for input in frame
            .input
            .commands
            .iter()
            .chain(&frame.input.post_commands)
        {
            match &input.player_input().command {
                PlayerCommand::ConnectSeat { player_id, .. } => {
                    local_commands.push((replay_ordinal, u16::from(player_id.0), true));
                }
                PlayerCommand::DisconnectSeat { player_id } => {
                    local_commands.push((replay_ordinal, u16::from(player_id.0), false));
                }
                _ => {}
            }
        }
    }
    let guest_events = transcript.events.iter().skip(1).collect::<Vec<_>>();
    if local_commands.len() != guest_events.len() {
        return Err(RankedSessionError::ReplayLifecycleMismatch(format!(
            "local replay has {} guest lifecycle commands but transcript has {}",
            local_commands.len(),
            guest_events.len()
        )));
    }
    for (index, ((replay_ordinal, seat, connected), event)) in
        local_commands.into_iter().zip(guest_events).enumerate()
    {
        let transcript_connected =
            matches!(event.lifecycle, ReplaySeatLifecycleKindV1::Connected { .. });
        if event.event_ordinal != u32::try_from(index + 1).unwrap_or(u32::MAX)
            || event.replay_ordinal != replay_ordinal
            || event.seat != seat
            || transcript_connected != connected
        {
            return Err(RankedSessionError::ReplayLifecycleMismatch(format!(
                "local lifecycle command {} does not equal transcript event {}",
                index + 1,
                event.event_ordinal
            )));
        }
    }
    replay
        .validate_ranked_command_admission(transcript)
        .map_err(RankedSessionError::ReplayLifecycleMismatch)
}

fn offer_matches_request(request: &SubmissionOfferRequestV1, offer: &SubmissionOfferV1) -> bool {
    robin_run_protocol::validate_offer_binding(request, offer).is_ok()
}

/// Exact local facts required before a campaign controller may arm the closed
/// continuation request. The server offer remains server-authored; everything
/// that affects the continuation claim is reconstructed locally.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedLocalContinuationEvidenceV1 {
    pub offer_request: SubmissionOfferRequestV1,
    pub continuation_claim: CampaignContinuationAuthorizationClaimV1,
    pub local_public_key: PublicKey32,
}

impl robin_run_protocol::Validate for RankedLocalContinuationEvidenceV1 {
    fn validate(&self) -> Result<(), robin_run_protocol::ValidationError> {
        self.offer_request.validate()?;
        self.continuation_claim.validate()?;
        if self.local_public_key.is_zero()
            || self.continuation_claim.campaign_controller_public_key != self.local_public_key
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "ranked_local_continuation.controller",
            });
        }
        Ok(())
    }
}

/// Bounded continuation precursor broadcast before any durable identity is
/// asked to sign. It contains no arbitrary signing bytes.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedContinuationContextV1 {
    pub offer_request: SubmissionOfferRequestV1,
    pub offer: SubmissionOfferV1,
    pub continuation_claim: CampaignContinuationAuthorizationClaimV1,
}

impl robin_run_protocol::Validate for RankedContinuationContextV1 {
    fn validate(&self) -> Result<(), robin_run_protocol::ValidationError> {
        self.offer_request.validate()?;
        self.offer.validate()?;
        self.continuation_claim.validate()?;
        if !offer_matches_request(&self.offer_request, &self.offer)
            || self
                .continuation_claim
                .co_sign_request(&self.offer)
                .is_err()
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "ranked_continuation_context.binding",
            });
        }
        Ok(())
    }
}

impl RankedContinuationContextV1 {
    pub fn validate_and_co_sign_request(
        &self,
        expected: &RankedLocalContinuationEvidenceV1,
    ) -> Result<robin_run_protocol::LeaderboardCoSignRequestV1, RankedSessionError> {
        self.validate().map_err(invalid_document)?;
        expected.validate().map_err(invalid_document)?;
        if self.offer_request != expected.offer_request
            || self.continuation_claim != expected.continuation_claim
        {
            return Err(RankedSessionError::InvalidDocument(
                "continuation context differs from locally retained campaign evidence".to_string(),
            ));
        }
        self.continuation_claim
            .co_sign_request(&self.offer)
            .map_err(invalid_document)
    }
}

/// Complete locally derived facts required before any participant may arm the
/// final submission request.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedLocalSubmissionEvidenceV1 {
    pub offer_request: SubmissionOfferRequestV1,
    pub replay_session_transcript: ReplaySessionTranscriptV1,
    pub artifacts: SubmissionArtifactsV1,
    pub campaign_aggregation_consent: CampaignAggregationConsentV1,
    pub campaign_continuation_claim: Option<CampaignContinuationAuthorizationClaimV1>,
    pub requested_metrics: Vec<BoardMetricV1>,
    pub local_public_key: PublicKey32,
}

impl robin_run_protocol::Validate for RankedLocalSubmissionEvidenceV1 {
    fn validate(&self) -> Result<(), robin_run_protocol::ValidationError> {
        self.offer_request.validate()?;
        self.replay_session_transcript.validate()?;
        self.artifacts.validate()?;
        if self.local_public_key.is_zero()
            || !self
                .offer_request
                .participant_claims
                .iter()
                .any(|claim| claim.public_key == self.local_public_key)
            || self.requested_metrics.is_empty()
            || !self
                .requested_metrics
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "ranked_local_submission.identity_or_metrics",
            });
        }
        if let Some(claim) = &self.campaign_continuation_claim {
            claim.validate()?;
            if claim.next_artifacts != self.artifacts {
                return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                    field: "ranked_local_submission.continuation_artifacts",
                });
            }
        }
        let continuation_scope = matches!(
            self.offer_request.scope_request,
            ScopeRequestV1::CampaignContinuation { .. }
        );
        if continuation_scope != self.campaign_continuation_claim.is_some() {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "ranked_local_submission.continuation_scope",
            });
        }
        Ok(())
    }
}

/// Exact server-issued facts a client must validate before locally arming the
/// closed final co-sign request. This is not a generic signing payload.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedSubmissionContextV1 {
    pub offer_request: SubmissionOfferRequestV1,
    pub replay_session_transcript: ReplaySessionTranscriptV1,
    pub submission: SubmissionEnvelopeV1,
}

impl robin_run_protocol::Validate for RankedSubmissionContextV1 {
    fn validate(&self) -> Result<(), robin_run_protocol::ValidationError> {
        self.offer_request.validate()?;
        self.replay_session_transcript.validate()?;
        self.submission.validate()?;
        if !offer_matches_request(&self.offer_request, &self.submission.offer)
            || self.submission.replay_session_transcript != self.replay_session_transcript
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "ranked_submission_context.binding",
            });
        }
        if let Some(authorization) = &self.submission.campaign_continuation_authorization {
            let signing_bytes = authorization
                .signing_bytes(&self.submission.offer)
                .map_err(|_| robin_run_protocol::ValidationError::ClaimMismatch {
                    field: "ranked_submission_context.continuation_signature",
                })?;
            verify_signature(
                authorization.claim.campaign_controller_public_key,
                &signing_bytes,
                authorization.signature,
            )
            .map_err(|_| robin_run_protocol::ValidationError::ClaimMismatch {
                field: "ranked_submission_context.continuation_signature",
            })?;
        }
        Ok(())
    }
}

impl RankedSubmissionContextV1 {
    pub fn validate_and_co_sign_request(
        &self,
        expected: &RankedLocalSubmissionEvidenceV1,
        local_replay: &ReplayData,
    ) -> Result<robin_run_protocol::LeaderboardCoSignRequestV1, RankedSessionError> {
        self.validate().map_err(invalid_document)?;
        expected.validate().map_err(invalid_document)?;
        let received_continuation_claim = self
            .submission
            .campaign_continuation_authorization
            .as_ref()
            .map(|authorization| authorization.claim.clone());
        if self.offer_request != expected.offer_request
            || self.replay_session_transcript != expected.replay_session_transcript
            || self.submission.artifacts != expected.artifacts
            || self.submission.campaign_aggregation_consent != expected.campaign_aggregation_consent
            || received_continuation_claim != expected.campaign_continuation_claim
            || self.submission.requested_metrics != expected.requested_metrics
        {
            return Err(RankedSessionError::InvalidDocument(
                "submission context differs from locally retained run evidence".to_string(),
            ));
        }
        validate_transcript_against_local_replay(
            local_replay,
            &expected.offer_request.session_genesis,
            &expected.offer_request.participant_claims,
            &expected.replay_session_transcript,
        )?;
        self.submission.co_sign_request().map_err(invalid_document)
    }
}

/// The only two ranked control documents a multiplayer host may broadcast to
/// the closed co-sign responder. The wire transports canonical bounded bytes
/// of this enum and never accepts a generic signing domain or payload.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RankedCoSignContextV1 {
    CampaignContinuation(RankedContinuationContextV1),
    Submission(RankedSubmissionContextV1),
}

impl robin_run_protocol::Validate for RankedCoSignContextV1 {
    fn validate(&self) -> Result<(), robin_run_protocol::ValidationError> {
        match self {
            Self::CampaignContinuation(context) => context.validate(),
            Self::Submission(context) => context.validate(),
        }
    }
}

impl RankedCoSignContextV1 {
    /// Derive the only closed request represented by this fully validated
    /// context. Host transport uses this to publish context and request as one
    /// inseparable operation; clients still use the stricter local-evidence
    /// methods above before arming the result.
    pub fn co_sign_request(
        &self,
    ) -> Result<robin_run_protocol::LeaderboardCoSignRequestV1, RankedSessionError> {
        self.validate().map_err(invalid_document)?;
        match self {
            Self::CampaignContinuation(context) => context
                .continuation_claim
                .co_sign_request(&context.offer)
                .map_err(invalid_document),
            Self::Submission(context) => context
                .submission
                .co_sign_request()
                .map_err(invalid_document),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::campaign::Campaign;
    use robin_engine::engine::{SimConfig, SimulationFrameInput};
    use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
    use robin_engine::replay::{ReplayFile, ReplayFrame, ReplayHeader};
    use robin_engine::replay_rankability::ReplayRankability;
    use robin_run_protocol::{
        ArtifactRefV1, CanonicalCampaignStateKindV1, CanonicalCampaignStateRequirementV1,
        ChallengeNonce32, FreshRunPreflightGrantClaimV1, FreshRunPreflightGrantV1,
        FreshRunPreflightRequestClaimV1, FreshRunPreflightRequestV1, FreshRunScopeV1,
        OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId,
        RANKED_CAMPAIGN_MEDIA_TYPE_V1, RANKED_REPLAY_MEDIA_TYPE_V1, ReplayArtifactV1,
        ResourceLocaleRootV1, SimulationSeed64, SpeechTimingAuthorityV1,
    };
    use std::collections::BTreeMap;

    fn digest(byte: u8) -> Digest32 {
        Digest32::from_bytes([byte; 32])
    }

    fn signing_key(byte: u8) -> SigningKey {
        SigningKey::from_bytes(&[byte; 32])
    }

    fn ranked() -> RankedSessionConfigV1 {
        RankedSessionConfigV1 {
            schema_version: SCHEMA_VERSION_V1,
            mission_id: "Dem_Lei_MP".to_string(),
            content_edition: OfficialContentEditionV1::Demo,
            content_subject: OfficialContentSubjectV1::FieldMission {
                mission_id: "Dem_Lei_MP".to_string(),
            },
            simulation_seed: SimulationSeed64::new(7),
            starting_campaign_sha256: digest(1),
            starting_campaign_byte_length: 1,
            prepared_inputs_projection_sha256: digest(2),
            prepared_mission_inputs_seal_sha256: digest(3),
            build_manifest_sha256: digest(4),
            content_manifest_sha256: digest(5),
            campaign_content_manifest_sha256: None,
            rules_config_sha256: digest(6),
            ruleset_manifest_sha256: digest(7),
            competition_manifest_sha256: None,
            spellforge_content_sha256: None,
            resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
            speech_timing: SpeechTimingAuthorityV1::BaseInstallation,
        }
    }

    fn official_setup(
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

    fn replay_with_lifecycle(commands: Vec<PlayerCommand>) -> ReplayData {
        let input = SimulationFrameInput {
            commands: commands
                .into_iter()
                .map(|command| PlayerInput::new(PlayerId::HOST, command).into())
                .collect(),
            ..Default::default()
        };
        ReplayFile {
            header: ReplayHeader {
                mission_id: "Dem_Lei_MP".to_string(),
                mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    "Dem_Lei_MP",
                    "Dem_Lei_MP",
                    "Dem_Lei_MP",
                )
                .expect("valid built-in ranked-session test descriptor"),
                rng_seed: 7,
                sim_config: SimConfig::default(),
                spellforge_package: None,
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                rankability: ReplayRankability::rankable(),
                campaign: bitcode::encode(&Campaign::new()),
            },
            frames: BTreeMap::from([(
                0,
                ReplayFrame {
                    timeline_before: 0,
                    timeline_after: 1,
                    input,
                    host_controls: Vec::new(),
                },
            )]),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
        .try_into()
        .expect("valid replay fixture")
    }

    fn submission_context(
        host_key: &SigningKey,
    ) -> (
        RankedSubmissionContextV1,
        RankedLocalSubmissionEvidenceV1,
        ReplayData,
    ) {
        let session = RankedSessionHost::new(host_key, 31, ranked()).unwrap();
        let replay = replay_with_lifecycle(Vec::new());
        let mut transcript = session.transcript_for_replay(&replay).unwrap();
        let mut genesis = session.genesis().clone();
        genesis.claim.fresh_run_preflight_grant = Some(FreshRunPreflightGrantV1 {
            claim: FreshRunPreflightGrantClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                grant_id: OpaqueId::new("fresh-grant-1").unwrap(),
                grant_nonce: robin_run_protocol::ChallengeNonce32::from_bytes([32; 32]),
                grant_authority_public_key: PublicKey32::from_bytes([33; 32]),
                host_public_key: genesis.claim.host_public_key,
                grant_request_sha256: digest(34),
                ranked_session_sha256: genesis.claim.ranked_session.canonical_digest().unwrap(),
                replay_session_id: genesis.claim.replay_session_id,
                host_participant_instance_id: genesis.claim.host_participant_instance_id,
                host_nonce: genesis.claim.host_nonce,
                scope: FreshRunScopeV1::IndividualLevel,
                starting_campaign: ArtifactRefV1 {
                    sha256: digest(1),
                    byte_length: 1,
                    media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_string(),
                },
                admitted_at_unix_ms: 1,
                expires_at_unix_ms: 1_800_000_000_000,
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            authority_signature: Signature64::from_bytes([35; 64]),
        });
        genesis.host_signature = signature(
            host_key,
            &genesis
                .claim
                .signing_bytes()
                .expect("test genesis signable"),
        );
        transcript.session_genesis_sha256 = genesis.canonical_digest().unwrap();
        let participant_claims = session.participant_claims();
        let offer_request = SubmissionOfferRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            max_concurrent_players: 1,
            participant_instance_count: 1,
            participant_claims: participant_claims.clone(),
            session_genesis: genesis.clone(),
            mission_id: ranked().mission_id,
            scope_request: ScopeRequestV1::IndividualLevel,
            ruleset_manifest_sha256: digest(7),
            competition_manifest_sha256: None,
        };
        let offer = SubmissionOfferV1 {
            schema_version: SCHEMA_VERSION_V1,
            upload_challenge_id: OpaqueId::new("challenge-1").unwrap(),
            upload_challenge_nonce: robin_run_protocol::ChallengeNonce32::from_bytes([31; 32]),
            expires_at_unix_ms: 1_800_000_000_000,
            max_concurrent_players: 1,
            participant_instance_count: 1,
            participant_claims,
            session_genesis: genesis,
            mission_id: ranked().mission_id,
            competition_manifest_sha256: None,
            build_manifest_sha256: digest(4),
            content_manifest_sha256: digest(5),
            rules_config_sha256: digest(6),
            ruleset_manifest_sha256: digest(7),
            starting_state: InitialStateExpectationV1::IndividualLevel {
                template_id: OpaqueId::new("demo-template").unwrap(),
                campaign_state_requirement: CanonicalCampaignStateRequirementV1 {
                    edition: OfficialContentEditionV1::Demo,
                    kind: CanonicalCampaignStateKindV1::IndividualTemplate,
                    rules_config_sha256: digest(6),
                },
                campaign_sha256: digest(1),
                starting_campaign_byte_length: 1,
            },
            allowed_metrics: vec![BoardMetricV1::OriginalScore],
        };
        let artifacts = SubmissionArtifactsV1 {
            replay: ReplayArtifactV1 {
                artifact: ArtifactRefV1 {
                    sha256: digest(40),
                    byte_length: 100,
                    media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_string(),
                },
                replay_schema_version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
            },
            starting_campaign: ArtifactRefV1 {
                sha256: digest(1),
                byte_length: 1,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_string(),
            },
        };
        let submission = SubmissionEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            offer,
            replay_session_transcript: transcript.clone(),
            artifacts: artifacts.clone(),
            campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
            campaign_continuation_authorization: None,
            requested_metrics: vec![BoardMetricV1::OriginalScore],
        };
        let context = RankedSubmissionContextV1 {
            offer_request: offer_request.clone(),
            replay_session_transcript: transcript.clone(),
            submission,
        };
        let local = RankedLocalSubmissionEvidenceV1 {
            offer_request,
            replay_session_transcript: transcript,
            artifacts,
            campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
            campaign_continuation_claim: None,
            requested_metrics: vec![BoardMetricV1::OriginalScore],
            local_public_key: public_key(host_key),
        };
        (context, local, replay)
    }

    #[test]
    fn signs_genesis_and_maps_authenticated_join_to_replay_ordinal() {
        let host_key = signing_key(0x10);
        let guest_key = signing_key(0x11);
        let mut session = RankedSessionHost::new(&host_key, 30, ranked()).unwrap();
        validate_session_genesis(
            session.genesis(),
            host_key.verifying_key().to_bytes(),
            &ranked(),
        )
        .unwrap();
        let claim = session
            .prepare_join(
                1,
                public_key(&guest_key),
                public_key(&guest_key),
                public_key(&host_key),
            )
            .unwrap();
        let attestation = sign_named_seat_join(&guest_key, claim).unwrap();
        session
            .admit_join(
                attestation,
                guest_key.verifying_key().to_bytes(),
                ParticipantPublicDisclosureV1::NamedProfile,
            )
            .unwrap();
        let replay = replay_with_lifecycle(vec![PlayerCommand::ConnectSeat {
            player_id: PlayerId(1),
            nickname: "guest".to_string(),
        }]);
        let transcript = session.transcript_for_replay(&replay).unwrap();
        assert_eq!(transcript.events[1].replay_ordinal, 0);
        assert_eq!(transcript.participant_instance_count, 2);
        assert_eq!(transcript.max_concurrent_players, 2);
    }

    #[test]
    fn mismatched_or_missing_lifecycle_fails_closed() {
        let host_key = signing_key(0x12);
        let guest_key = signing_key(0x13);
        let mut session = RankedSessionHost::new(&host_key, 30, ranked()).unwrap();
        let claim = session
            .prepare_join(
                1,
                public_key(&guest_key),
                public_key(&guest_key),
                public_key(&host_key),
            )
            .unwrap();
        session
            .admit_join(
                sign_named_seat_join(&guest_key, claim).unwrap(),
                guest_key.verifying_key().to_bytes(),
                ParticipantPublicDisclosureV1::Anonymous,
            )
            .unwrap();
        assert!(
            session
                .transcript_for_replay(&replay_with_lifecycle(Vec::new()))
                .is_err()
        );
        assert!(
            session
                .transcript_for_replay(&replay_with_lifecycle(vec![PlayerCommand::ConnectSeat {
                    player_id: PlayerId(2),
                    nickname: "wrong".to_string(),
                },]))
                .is_err()
        );
    }

    #[test]
    fn join_signature_is_transport_bound() {
        let host_key = signing_key(0x14);
        let guest_key = signing_key(0x15);
        let mut session = RankedSessionHost::new(&host_key, 30, ranked()).unwrap();
        let claim = session
            .prepare_join(
                1,
                public_key(&guest_key),
                public_key(&guest_key),
                public_key(&host_key),
            )
            .unwrap();
        let attestation = sign_named_seat_join(&guest_key, claim).unwrap();
        assert!(verify_named_seat_join(&attestation, &[0x55; 32]).is_err());
    }

    #[test]
    fn pending_join_is_serialized_and_exact() {
        let host_key = signing_key(0x16);
        let first_key = signing_key(0x17);
        let second_key = signing_key(0x18);
        let mut session = RankedSessionHost::new(&host_key, 30, ranked()).unwrap();
        let first = session
            .prepare_join(
                1,
                public_key(&first_key),
                public_key(&first_key),
                public_key(&host_key),
            )
            .unwrap();
        assert!(
            session
                .prepare_join(
                    2,
                    public_key(&second_key),
                    public_key(&second_key),
                    public_key(&host_key),
                )
                .is_err()
        );
        let mut substituted = first.clone();
        substituted.participant_instance_id = digest(0xee);
        assert!(
            session
                .admit_join(
                    sign_named_seat_join(&first_key, substituted).unwrap(),
                    first_key.verifying_key().to_bytes(),
                    ParticipantPublicDisclosureV1::NamedProfile,
                )
                .is_err()
        );
        session.cancel_pending_join();
        assert!(
            session
                .prepare_join(
                    2,
                    public_key(&second_key),
                    public_key(&second_key),
                    public_key(&host_key),
                )
                .is_ok()
        );
    }

    #[test]
    fn wire_documents_require_canonical_json_and_a_strict_bound() {
        let host_key = signing_key(0x19);
        let session = RankedSessionHost::new(&host_key, 30, ranked()).unwrap();
        let bytes = encode_ranked_wire_document(session.genesis()).unwrap();
        let decoded: ReplaySessionGenesisV1 = decode_ranked_wire_document(&bytes).unwrap();
        assert_eq!(&decoded, session.genesis());

        let mut padded = bytes.clone();
        padded.push(b'\n');
        assert!(matches!(
            decode_ranked_wire_document::<ReplaySessionGenesisV1>(&padded),
            Err(RankedSessionError::NonCanonicalDocument)
        ));
        assert!(matches!(
            decode_ranked_wire_document::<ReplaySessionGenesisV1>(&vec![
                b' ';
                MAX_RANKED_SESSION_WIRE_DOCUMENT_BYTES
                    + 1
            ]),
            Err(RankedSessionError::DocumentTooLarge { .. })
        ));
    }

    #[test]
    fn browse_only_downgrade_is_irreversible_and_has_no_evidence() {
        let host_key = signing_key(0x1a);
        let mut lifecycle = RankedSessionLifecycle::ranked(
            RankedSessionHost::new(&host_key, 30, ranked()).unwrap(),
        );
        lifecycle.downgrade("participant durable identity unavailable");
        lifecycle.downgrade("later replacement must not erase first cause");
        assert_eq!(
            lifecycle.browse_only_reason(),
            Some("participant durable identity unavailable")
        );
        assert!(
            lifecycle
                .evidence_for_replay(&replay_with_lifecycle(Vec::new()))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reconnect_requires_a_fresh_durable_signature_and_maps_exactly() {
        let host_key = signing_key(0x1b);
        let guest_key = signing_key(0x1c);
        let first_transport = signing_key(0x1d);
        let second_transport = signing_key(0x1e);
        let mut session = RankedSessionHost::new(&host_key, 31, ranked()).unwrap();
        let join = session
            .prepare_join(
                1,
                public_key(&guest_key),
                public_key(&first_transport),
                public_key(&host_key),
            )
            .unwrap();
        session
            .admit_join(
                sign_named_seat_join(&guest_key, join).unwrap(),
                first_transport.verifying_key().to_bytes(),
                ParticipantPublicDisclosureV1::Anonymous,
            )
            .unwrap();
        session.observe_disconnect(1).unwrap();
        let reconnect = session
            .prepare_reconnect(
                1,
                public_key(&guest_key),
                public_key(&second_transport),
                public_key(&host_key),
            )
            .unwrap();
        assert_eq!(reconnect.connection_epoch, 1);
        session
            .admit_reconnect(
                sign_named_seat_join(&guest_key, reconnect).unwrap(),
                second_transport.verifying_key().to_bytes(),
            )
            .unwrap();
        let transcript = session
            .transcript_for_replay(&replay_with_lifecycle(vec![
                PlayerCommand::ConnectSeat {
                    player_id: PlayerId(1),
                    nickname: "guest".to_string(),
                },
                PlayerCommand::DisconnectSeat {
                    player_id: PlayerId(1),
                },
                PlayerCommand::ConnectSeat {
                    player_id: PlayerId(1),
                    nickname: "guest".to_string(),
                },
            ]))
            .unwrap();
        assert_eq!(transcript.events.len(), 4);
        assert_eq!(
            transcript.events[3].lifecycle,
            ReplaySeatLifecycleKindV1::Connected {
                connection_epoch: 1
            }
        );
    }

    #[test]
    fn submission_context_requires_local_artifacts_and_local_replay() {
        let host_key = signing_key(0x1f);
        let (context, local, replay) = submission_context(&host_key);
        assert_eq!(
            context
                .validate_and_co_sign_request(&local, &replay)
                .unwrap(),
            context.submission.co_sign_request().unwrap()
        );

        let mut wrong_artifact = local.clone();
        wrong_artifact.artifacts.replay.artifact.sha256 = digest(0xee);
        assert!(
            context
                .validate_and_co_sign_request(&wrong_artifact, &replay)
                .unwrap_err()
                .to_string()
                .contains("locally retained run evidence")
        );

        let replay_with_foreign_command = replay_with_lifecycle(vec![PlayerCommand::ConnectSeat {
            player_id: PlayerId(1),
            nickname: "unattested".to_string(),
        }]);
        assert!(
            context
                .validate_and_co_sign_request(&local, &replay_with_foreign_command)
                .unwrap_err()
                .to_string()
                .contains("lifecycle")
        );
    }

    #[test]
    fn co_sign_context_wire_is_typed_canonical_and_bounded() {
        let host_key = signing_key(0x20);
        let (submission, _, _) = submission_context(&host_key);
        let expected_request = submission.submission.co_sign_request().unwrap();
        let context = RankedCoSignContextV1::Submission(submission);
        assert_eq!(context.co_sign_request().unwrap(), expected_request);
        let bytes = encode_ranked_wire_document(&context).unwrap();
        let decoded: RankedCoSignContextV1 = decode_ranked_wire_document(&bytes).unwrap();
        assert_eq!(decoded, context);

        let mut noncanonical = bytes;
        noncanonical.push(b'\n');
        assert!(matches!(
            decode_ranked_wire_document::<RankedCoSignContextV1>(&noncanonical),
            Err(RankedSessionError::NonCanonicalDocument)
        ));
    }

    #[test]
    fn unresolved_ranked_lifecycle_never_yields_fake_evidence() {
        let lifecycle = RankedSessionLifecycle::awaiting_prepared_inputs();
        assert!(
            lifecycle
                .evidence_for_replay(&replay_with_lifecycle(Vec::new()))
                .unwrap_err()
                .to_string()
                .contains("never resolved")
        );
    }

    #[test]
    fn client_lifecycle_becomes_ranked_only_after_exact_admission_ack() {
        let host_key = signing_key(0x21);
        let guest_key = signing_key(0x22);
        let transport_key = signing_key(0x23);
        let setup = official_setup(&host_key, ranked(), false);
        let mut host = RankedSessionHost::new_official(&host_key, 31, setup.clone()).unwrap();
        let join = host
            .prepare_join(
                1,
                public_key(&guest_key),
                public_key(&transport_key),
                public_key(&host_key),
            )
            .unwrap();
        host.admit_join(
            sign_named_seat_join(&guest_key, join).unwrap(),
            transport_key.verifying_key().to_bytes(),
            ParticipantPublicDisclosureV1::NamedProfile,
        )
        .unwrap();

        let admission = RankedSessionClientAdmissionV1::new_official(
            setup,
            public_key(&host_key),
            public_key(&guest_key),
            public_key(&transport_key),
        )
        .unwrap();
        let mut lifecycle = RankedSessionLifecycle::awaiting_prepared_inputs();
        lifecycle.install_client_admission(admission).unwrap();
        assert!(lifecycle.ranked_client().is_none());

        assert!(
            lifecycle
                .accept_ranked_client(2, host.genesis().clone(), host.participant_claims())
                .is_err()
        );
        assert!(lifecycle.client_admission().is_some());
        lifecycle
            .accept_ranked_client(1, host.genesis().clone(), host.participant_claims())
            .unwrap();
        let client = lifecycle.ranked_client().unwrap();
        assert_eq!(client.local_seat, 1);
        assert_eq!(client.participant_claims, host.participant_claims());
    }

    #[test]
    fn official_ranked_bootstrap_rejects_any_custom_package_presence() {
        let host_key = signing_key(0x24);
        let setup = official_setup(&host_key, ranked(), true);
        assert!(
            RankedSessionHost::new_official(&host_key, 31, setup.clone())
                .err()
                .unwrap()
                .to_string()
                .contains("custom_package_present")
        );
        assert!(
            RankedSessionClientAdmissionV1::new_official(
                setup,
                public_key(&host_key),
                public_key(&host_key),
                public_key(&host_key),
            )
            .unwrap_err()
            .to_string()
            .contains("custom_package_present")
        );
    }
    #[test]
    fn peer_rejects_expired_grant_despite_host_in_window_setup_time() {
        let host_key = signing_key(0x6a);
        let host_setup = official_setup(&host_key, ranked(), false);
        assert_eq!(host_setup.trusted_now_unix_ms, 1_500);
        host_setup.validate().unwrap();
        let wire = OfficialRankedSessionWireSetupV1::from_local_setup(&host_setup).unwrap();
        let expectation = OfficialRankedSessionExpectationV1 {
            ranked_session: host_setup.ranked_session.clone(),
            custom_package_present: false,
            run_preflight_grant_public_key: host_setup.run_preflight_grant_public_key,
        };
        assert!(
            wire.clone()
                .prepare_for_authenticated_peer(
                    &expectation,
                    public_key(&host_key),
                    public_key(&host_key),
                    1_500,
                )
                .is_ok()
        );
        let error = wire
            .prepare_for_authenticated_peer(
                &expectation,
                public_key(&host_key),
                public_key(&host_key),
                2_001,
            )
            .unwrap_err();
        assert!(error.to_string().contains("trusted setup time"));
    }

    #[test]
    fn official_ranked_bootstrap_rejects_a_non_shipping_subject() {
        let host_key = signing_key(0x25);
        let mut ranked_session = ranked();
        ranked_session.mission_id = "custom_mission".to_string();
        ranked_session.content_subject = OfficialContentSubjectV1::FieldMission {
            mission_id: ranked_session.mission_id.clone(),
        };
        let setup = official_setup(&host_key, ranked_session, false);
        assert!(
            RankedSessionHost::new_official(&host_key, 31, setup.clone())
                .err()
                .unwrap()
                .to_string()
                .contains("content_subject")
        );
        assert!(
            RankedSessionClientAdmissionV1::new_official(
                setup,
                public_key(&host_key),
                public_key(&host_key),
                public_key(&host_key),
            )
            .unwrap_err()
            .to_string()
            .contains("content_subject")
        );
    }

    #[test]
    fn participant_roster_rejects_a_forged_host_genesis() {
        let host_key = signing_key(0x26);
        let host = RankedSessionHost::new(&host_key, 31, ranked()).unwrap();
        let mut forged = host.genesis().clone();
        forged.host_signature = Signature64::from_bytes([0x55; 64]);
        assert!(matches!(
            validate_participant_roster(&forged, &host.participant_claims()),
            Err(RankedSessionError::InvalidSignature)
        ));
    }

    #[test]
    fn isolated_signer_genesis_builder_never_accepts_a_forged_result() {
        let key = signing_key(0x27);
        let setup = official_setup(&key, ranked(), false);
        let claim =
            RankedSessionHost::prepare_official_genesis_claim(public_key(&key), 31, setup.clone())
                .unwrap();
        let mut genesis = ReplaySessionGenesisV1 {
            host_signature: signature(&key, &claim.signing_bytes().unwrap()),
            claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        validate_official_session_genesis(&genesis, key.verifying_key().to_bytes(), &setup)
            .unwrap();
        let session = RankedSessionHost::from_signed_genesis(genesis.clone()).unwrap();
        assert_eq!(session.genesis(), &genesis);

        genesis.host_signature = Signature64::from_bytes([0x77; 64]);
        assert!(matches!(
            RankedSessionHost::from_signed_genesis(genesis),
            Err(RankedSessionError::InvalidSignature)
        ));
    }
}
