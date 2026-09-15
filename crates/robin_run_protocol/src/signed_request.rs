//! Player-signed requests with a signed timestamp instead of a server nonce.
//!
//! Every player operation (submission, username update, deletion, private
//! submission status) is a claim `T` carrying the player's public key and
//! `signed_at_unix_ms`. The player signs `T::DOMAIN || canonical_json(T)` with
//! Ed25519. The per-operation domain keeps a signature for one operation from
//! being accepted for another; the timestamp bounds how long a captured request
//! stays usable. Operations stay safe under replay within the window:
//! submissions are deduplicated by replay hash, deletion is idempotent, username
//! updates must be newer than the last accepted update, and a private status
//! read only returns what the key owner can already see.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{PublicKey32, Signature64, Validate, ValidationError};

/// Default maximum age of a signed request when it reaches the server.
pub const SIGNED_REQUEST_DEFAULT_MAX_AGE_MS: u64 = 5 * 60 * 1000;
/// Default tolerance for a client clock running ahead of the server.
pub const SIGNED_REQUEST_DEFAULT_MAX_FUTURE_SKEW_MS: u64 = 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureAlgorithmV1 {
    Ed25519,
}

/// A claim a player signs directly.
pub trait SignedRequestClaim: Validate + Serialize + DeserializeOwned {
    /// NUL-terminated per-operation signing domain.
    const DOMAIN: &'static [u8];

    fn signer_public_key(&self) -> PublicKey32;

    fn signed_at_unix_ms(&self) -> u64;
}

/// Server acceptance window for `signed_at_unix_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRequestWindowV1 {
    pub max_age_ms: u64,
    pub max_future_skew_ms: u64,
}

impl Default for SignedRequestWindowV1 {
    fn default() -> Self {
        Self {
            max_age_ms: SIGNED_REQUEST_DEFAULT_MAX_AGE_MS,
            max_future_skew_ms: SIGNED_REQUEST_DEFAULT_MAX_FUTURE_SKEW_MS,
        }
    }
}

impl SignedRequestWindowV1 {
    /// Accept only `now - max_age <= signed_at <= now + max_future_skew`.
    pub fn check(
        self,
        signed_at_unix_ms: u64,
        now_unix_ms: u64,
    ) -> Result<(), SignedRequestFreshnessError> {
        if signed_at_unix_ms.saturating_add(self.max_age_ms) < now_unix_ms {
            return Err(SignedRequestFreshnessError::Expired {
                signed_at_unix_ms,
                now_unix_ms,
            });
        }
        if signed_at_unix_ms > now_unix_ms.saturating_add(self.max_future_skew_ms) {
            return Err(SignedRequestFreshnessError::FromTheFuture {
                signed_at_unix_ms,
                now_unix_ms,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignedRequestFreshnessError {
    #[error("signed request from {signed_at_unix_ms} is too old at {now_unix_ms}")]
    Expired {
        signed_at_unix_ms: u64,
        now_unix_ms: u64,
    },
    #[error("signed request from {signed_at_unix_ms} is ahead of server time {now_unix_ms}")]
    FromTheFuture {
        signed_at_unix_ms: u64,
        now_unix_ms: u64,
    },
}

/// A claim plus the Ed25519 signature of its signer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(bound(deserialize = "T: SignedRequestClaim"))]
pub struct SignedRequestV2<T: SignedRequestClaim> {
    pub schema_version: u32,
    pub request: T,
    pub algorithm: SignatureAlgorithmV1,
    pub signature: Signature64,
}

impl<T: SignedRequestClaim> SignedRequestV2<T> {
    /// Bytes the player signs: the domain-separated canonical claim.
    pub fn signing_bytes(request: &T) -> Result<Vec<u8>, crate::CanonicalDocumentError> {
        request.validate()?;
        Ok(crate::canonical::domain_separated_bytes(
            T::DOMAIN,
            request,
        )?)
    }

    /// Validate, check freshness against the server clock and verify the
    /// signature by the claim's own public key.
    #[cfg(feature = "authentication")]
    pub fn verify(
        &self,
        window: SignedRequestWindowV1,
        now_unix_ms: u64,
    ) -> Result<(), SignedRequestError> {
        self.validate().map_err(SignedRequestError::Invalid)?;
        window
            .check(self.request.signed_at_unix_ms(), now_unix_ms)
            .map_err(SignedRequestError::Freshness)?;
        let bytes = Self::signing_bytes(&self.request).map_err(|_| {
            SignedRequestError::Signature(crate::SignatureVerificationError::InvalidSignature)
        })?;
        crate::verify_ed25519_strict(
            self.request.signer_public_key().as_bytes(),
            self.signature.as_bytes(),
            &bytes,
        )
        .map_err(SignedRequestError::Signature)
    }
}

impl<T: SignedRequestClaim> Validate for SignedRequestV2<T> {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "SignedRequestV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        self.request.validate()?;
        crate::validation::nonzero(
            "signed_request.signer_public_key",
            &self.request.signer_public_key(),
        )?;
        crate::validation::nonzero(
            "signed_request.signed_at_unix_ms",
            &self.request.signed_at_unix_ms(),
        )?;
        crate::validation::nonzero("signed_request.signature", &self.signature)
    }
}

#[cfg(feature = "authentication")]
#[derive(Debug, thiserror::Error)]
pub enum SignedRequestError {
    #[error("signed request is invalid: {0}")]
    Invalid(ValidationError),
    #[error(transparent)]
    Freshness(SignedRequestFreshnessError),
    #[error(transparent)]
    Signature(crate::SignatureVerificationError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_accepts_recent_and_small_future_skew_only() {
        let window = SignedRequestWindowV1::default();
        let now = 10_000_000;
        assert!(window.check(now, now).is_ok());
        assert!(
            window
                .check(now - SIGNED_REQUEST_DEFAULT_MAX_AGE_MS, now)
                .is_ok()
        );
        assert!(matches!(
            window.check(now - SIGNED_REQUEST_DEFAULT_MAX_AGE_MS - 1, now),
            Err(SignedRequestFreshnessError::Expired { .. })
        ));
        assert!(
            window
                .check(now + SIGNED_REQUEST_DEFAULT_MAX_FUTURE_SKEW_MS, now)
                .is_ok()
        );
        assert!(matches!(
            window.check(now + SIGNED_REQUEST_DEFAULT_MAX_FUTURE_SKEW_MS + 1, now),
            Err(SignedRequestFreshnessError::FromTheFuture { .. })
        ));
    }
}
