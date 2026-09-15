//! Closed, typed signing adapter for the durable game identity.
//!
//! Native builds reuse the persistent iroh game key (`native.rs`). Browser
//! builds delegate to the isolated signer origin (`browser.rs`), whose
//! IndexedDB record is shared with the Feature 38 multiplayer seat-proof
//! protocol. Neither path exposes a raw or generic signing API to callers.
//!
//! Both platforms implement the one async [`GameIdentitySigner`] surface.
//! Native futures never suspend; browser futures complete on a later frame.

use robin_run_protocol::{
    PublicKey32, Signature64, SignatureAlgorithmV1, SignedSubmissionV2,
    SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1, SubmissionV2, Validate,
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
            #[cfg(target_arch = "wasm32")]
            Self::DocumentTooLarge { .. } | Self::InvalidJson(_) => false,
        }
    }
}

/// Every closed operation the durable game identity performs for the game.
/// Implemented once per platform; callers use [`PlatformSigner`].
pub(crate) trait GameIdentitySigner {
    async fn public_key() -> Result<PublicKey32, LeaderboardSigningError>;

    /// Sign one replay submission whose `uploader_public_key` is this identity.
    async fn sign_submission(
        submission: SubmissionV2,
    ) -> Result<SignedSubmissionV2, LeaderboardSigningError>;

    async fn sign_submission_owner_status(
        challenge: SubmissionOwnerStatusChallengeV1,
    ) -> Result<SubmissionOwnerStatusEnvelopeV1, LeaderboardSigningError>;
}

/// Bytes the uploader signs for `submission`, validating the claim first.
fn submission_signing_bytes(submission: &SubmissionV2) -> Result<Vec<u8>, LeaderboardSigningError> {
    SignedSubmissionV2::signing_bytes(submission)
        .map_err(|error| LeaderboardSigningError::Canonical(error.to_string()))
}

/// Assemble a signed submission and verify the signature locally, so a
/// misbehaving signer can never hand an unverifiable upload to the network.
fn assemble_signed_submission(
    submission: SubmissionV2,
    signer_public_key: PublicKey32,
    signature: Signature64,
) -> Result<SignedSubmissionV2, LeaderboardSigningError> {
    if signer_public_key != submission.uploader_public_key {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    let signed = SignedSubmissionV2 {
        schema_version: robin_run_protocol::SCHEMA_VERSION_V2,
        submission,
        algorithm: SignatureAlgorithmV1::Ed25519,
        signature,
    };
    signed.validate().map_err(invalid_claim)?;
    signed.verify_signature().map_err(invalid_claim)?;
    Ok(signed)
}

fn invalid_claim(error: impl std::fmt::Display) -> LeaderboardSigningError {
    LeaderboardSigningError::InvalidClaim(error.to_string())
}
