//! Closed, typed signing adapter for the durable game identity.
//!
//! Native builds reuse the persistent iroh game key (`native.rs`). Browser
//! builds delegate to the isolated signer origin (`browser.rs`), whose
//! IndexedDB record is shared with the Feature 38 multiplayer seat-proof
//! protocol. Neither path exposes a raw or generic signing API to callers.
//!
//! Both platforms implement the one async [`GameIdentitySigner`] surface.
//! Native futures never suspend, so callers drive them through
//! [`crate::leaderboard::task::PollTask::start`] and see the result on the
//! same frame; browser futures complete on a later frame.

use crate::leaderboard_ranked_session::{OfficialRankedSessionSetupV1, RankedSessionHost};
use robin_run_protocol::{
    CampaignContinuationAuthorizationClaimV1, CampaignContinuationAuthorizationV1,
    CampaignContinuationPreflightRequestClaimV1, CampaignContinuationPreflightRequestV1,
    FreshRunPreflightRequestClaimV1, FreshRunPreflightRequestV1, LeaderboardCoSignRequestV1,
    ParticipantSignatureV1, PublicKey32, Signature64, SignatureAlgorithmV1, SubmissionEnvelopeV1,
    SubmissionOfferV1, SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1, Validate,
};

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;

/// The durable game identity of the running platform.
#[cfg(target_arch = "wasm32")]
pub use browser::BrowserSigner as PlatformSigner;
/// The durable game identity of the running platform.
#[cfg(not(target_arch = "wasm32"))]
pub use native::NativeSigner as PlatformSigner;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LeaderboardSigningError {
    #[error("durable game identity is unavailable: {0}")]
    Identity(String),
    #[error("leaderboard signing claim is invalid: {0}")]
    InvalidClaim(String),
    #[error("leaderboard signing claim names a different public key")]
    WrongIdentity,
    /// Only the in-process native signer inspects the participant claims
    /// before signing; the browser signer origin enforces this itself.
    #[cfg(not(target_arch = "wasm32"))]
    #[error("leaderboard submission does not claim this public key")]
    IdentityNotClaimed,
    #[error("canonical leaderboard signing failed: {0}")]
    Canonical(String),
    #[cfg(target_arch = "wasm32")]
    #[error("leaderboard bridge document exceeds {maximum} bytes")]
    DocumentTooLarge { maximum: usize },
    #[cfg(target_arch = "wasm32")]
    #[error("leaderboard bridge document is not valid JSON: {0}")]
    InvalidJson(String),
}

impl LeaderboardSigningError {
    /// Only an unavailable identity store may succeed on retry; every claim,
    /// identity or document failure is permanent for the same input.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Identity(_) => true,
            Self::WrongIdentity | Self::InvalidClaim(_) | Self::Canonical(_) => false,
            #[cfg(not(target_arch = "wasm32"))]
            Self::IdentityNotClaimed => false,
            #[cfg(target_arch = "wasm32")]
            Self::DocumentTooLarge { .. } | Self::InvalidJson(_) => false,
        }
    }
}

/// Every closed operation the durable game identity performs. Implemented
/// once per platform; callers use [`PlatformSigner`].
pub(crate) trait GameIdentitySigner {
    async fn public_key() -> Result<PublicKey32, LeaderboardSigningError>;

    /// Create the signed official ranked-session lifecycle without exposing
    /// secret key material to mission bootstrap code.
    async fn create_official_ranked_session(
        network_protocol_version: u32,
        setup: OfficialRankedSessionSetupV1,
    ) -> Result<RankedSessionHost, LeaderboardSigningError>;

    async fn sign_submission_claim(
        envelope: &SubmissionEnvelopeV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError>;

    /// Sign the closed fixed-size request received from the multiplayer
    /// transport. The transport is responsible for accepting only the exact
    /// request installed by the locally validated mission-end flow. This API
    /// cannot sign caller-supplied bytes or a purpose outside the V1 co-sign
    /// contract.
    async fn sign_multiplayer_leaderboard_request(
        request: &LeaderboardCoSignRequestV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError>;

    /// Sign the only host-authored fresh-run preflight claim. Callers cannot
    /// supply a domain or arbitrary bytes, and the claim must name this
    /// install's durable game identity.
    async fn sign_fresh_run_preflight_request(
        claim: FreshRunPreflightRequestClaimV1,
    ) -> Result<FreshRunPreflightRequestV1, LeaderboardSigningError>;

    /// Host half of the dual-authorized campaign continuation preflight request.
    async fn sign_campaign_continuation_preflight_as_host(
        claim: &CampaignContinuationPreflightRequestClaimV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError>;

    /// Controller half of the dual-authorized campaign continuation preflight
    /// request. This is the only operation exposed to a remote controller peer.
    async fn sign_campaign_continuation_preflight_as_controller(
        claim: &CampaignContinuationPreflightRequestClaimV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError>;

    async fn sign_campaign_continuation(
        offer: &SubmissionOfferV1,
        claim: CampaignContinuationAuthorizationClaimV1,
    ) -> Result<CampaignContinuationAuthorizationV1, LeaderboardSigningError>;

    async fn sign_submission_owner_status(
        challenge: SubmissionOwnerStatusChallengeV1,
    ) -> Result<SubmissionOwnerStatusEnvelopeV1, LeaderboardSigningError>;
}

pub fn assemble_campaign_continuation_preflight_request(
    claim: CampaignContinuationPreflightRequestClaimV1,
    host: ParticipantSignatureV1,
    controller: ParticipantSignatureV1,
) -> Result<CampaignContinuationPreflightRequestV1, LeaderboardSigningError> {
    claim.validate().map_err(invalid_claim)?;
    if host.public_key != claim.host_public_key
        || controller.public_key != claim.campaign_controller_public_key
    {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    verify_typed_signature(
        host.public_key,
        &canonical(claim.host_signing_bytes())?,
        host.signature,
    )?;
    verify_typed_signature(
        controller.public_key,
        &canonical(claim.controller_signing_bytes())?,
        controller.signature,
    )?;
    let request = CampaignContinuationPreflightRequestV1 {
        claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
        host_signature: host.signature,
        controller_signature: controller.signature,
    };
    request.validate().map_err(invalid_claim)?;
    Ok(request)
}

fn canonical(
    result: Result<Vec<u8>, robin_run_protocol::CanonicalError>,
) -> Result<Vec<u8>, LeaderboardSigningError> {
    result.map_err(|error| LeaderboardSigningError::Canonical(error.to_string()))
}

fn verify_typed_signature(
    public_key: PublicKey32,
    bytes: &[u8],
    signature: Signature64,
) -> Result<(), LeaderboardSigningError> {
    robin_run_protocol::verify_ed25519_strict(public_key.as_bytes(), signature.as_bytes(), bytes)
        .map_err(|error| LeaderboardSigningError::InvalidClaim(error.to_string()))
}

fn invalid_claim(error: impl std::fmt::Display) -> LeaderboardSigningError {
    LeaderboardSigningError::InvalidClaim(error.to_string())
}
