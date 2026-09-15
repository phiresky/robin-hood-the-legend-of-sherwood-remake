//! Closed, typed signing adapter for the durable game identity.
//!
//! Native builds reuse the persistent iroh game key (`native.rs`). Browser
//! builds delegate to the isolated signer origin (`browser.rs`), whose
//! IndexedDB record is shared with the Feature 38 multiplayer seat-proof
//! protocol. Neither path exposes a raw or generic signing API to callers.
//!
//! Both platforms implement the one async [`GameIdentitySigner`] surface.
//! Native futures never suspend; browser futures complete on a later frame.
//!
//! Every player request is a [`SignedRequestClaim`] carrying the player key and
//! the wall-clock `signed_at_unix_ms`; callers build a fresh claim for each
//! network attempt because the server only accepts recently signed requests.

use robin_run_protocol::{
    PublicKey32, Signature64, SignatureAlgorithmV1, SignedRequestClaim, SignedRequestV2,
    SignedSubmissionOwnerStatusRequestV2, SignedSubmissionV3, SubmissionOwnerStatusRequestV2,
    SubmissionV3, Validate,
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
    /// Only the in-process native signer canonicalizes documents; the
    /// browser signer origin does this itself.
    #[cfg(not(target_arch = "wasm32"))]
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
            Self::WrongIdentity | Self::InvalidClaim(_) => false,
            #[cfg(not(target_arch = "wasm32"))]
            Self::Canonical(_) => false,
            #[cfg(target_arch = "wasm32")]
            Self::DocumentTooLarge { .. } | Self::InvalidJson(_) => false,
        }
    }
}

/// Every closed operation the durable game identity performs for the game.
/// Implemented once per platform; callers use [`PlatformSigner`].
pub(crate) trait GameIdentitySigner {
    async fn public_key() -> Result<PublicKey32, LeaderboardSigningError>;

    async fn sign_username_update(
        request: robin_run_protocol::UsernameUpdateV2,
    ) -> Result<robin_run_protocol::SignedUsernameUpdateV2, LeaderboardSigningError>;

    /// Sign one replay submission whose `uploader_public_key` is this identity.
    async fn sign_submission(
        submission: SubmissionV3,
    ) -> Result<SignedSubmissionV3, LeaderboardSigningError>;

    /// Sign one private status read whose `public_key` is this identity.
    async fn sign_submission_owner_status(
        request: SubmissionOwnerStatusRequestV2,
    ) -> Result<SignedSubmissionOwnerStatusRequestV2, LeaderboardSigningError>;
}

/// Assemble a signed request and verify the signature locally, so a
/// misbehaving signer can never hand an unverifiable request to the network.
fn assemble_signed_request<T: SignedRequestClaim>(
    request: T,
    signer_public_key: PublicKey32,
    signature: Signature64,
) -> Result<SignedRequestV2<T>, LeaderboardSigningError> {
    if signer_public_key != request.signer_public_key() {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    let signed = SignedRequestV2 {
        schema_version: robin_run_protocol::SCHEMA_VERSION_V2,
        request,
        algorithm: SignatureAlgorithmV1::Ed25519,
        signature,
    };
    verify_signed_request(&signed)?;
    Ok(signed)
}

/// Structural validation plus a strict Ed25519 check by the claim's own key.
/// Freshness is the server's decision and is deliberately not checked here.
fn verify_signed_request<T: SignedRequestClaim>(
    signed: &SignedRequestV2<T>,
) -> Result<(), LeaderboardSigningError> {
    signed.validate().map_err(invalid_claim)?;
    let bytes = SignedRequestV2::<T>::signing_bytes(&signed.request).map_err(invalid_claim)?;
    robin_run_protocol::verify_ed25519_strict(
        signed.request.signer_public_key().as_bytes(),
        signed.signature.as_bytes(),
        &bytes,
    )
    .map_err(invalid_claim)
}

fn invalid_claim(error: impl std::fmt::Display) -> LeaderboardSigningError {
    LeaderboardSigningError::InvalidClaim(error.to_string())
}

#[cfg(test)]
pub(crate) fn verify_signed_request_for_tests<T: SignedRequestClaim>(
    signed: &SignedRequestV2<T>,
) -> Result<(), LeaderboardSigningError> {
    verify_signed_request(signed)
}
