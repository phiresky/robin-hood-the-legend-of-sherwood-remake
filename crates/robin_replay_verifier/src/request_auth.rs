//! Cryptographic admission for one signed verification request.
//!
//! Protocol validation checks shape and cross-field identities. This module
//! additionally verifies every Ed25519 proof before any replay or campaign
//! bytes reach an allocation-capable decoder.

use robin_run_protocol::{
    CanonicalDocument as _, Digest32, PublicKey32, Signature64, Validate as _,
    VerificationRequestV1,
};

/// A request whose protocol invariants, genesis signature, named guest join
/// attestations, and final participant co-signatures all verified.
///
/// Fields are private so callers cannot construct this capability after
/// skipping authentication.
#[derive(Debug)]
pub struct AuthenticatedVerificationRequest {
    request: VerificationRequestV1,
    canonical_sha256: Digest32,
    session_genesis_sha256: Digest32,
}

impl AuthenticatedVerificationRequest {
    pub const fn request(&self) -> &VerificationRequestV1 {
        &self.request
    }

    pub const fn canonical_sha256(&self) -> Digest32 {
        self.canonical_sha256
    }

    pub const fn session_genesis_sha256(&self) -> Digest32 {
        self.session_genesis_sha256
    }

    pub fn into_request(self) -> VerificationRequestV1 {
        self.request
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RequestAuthenticationError {
    #[error("verification request protocol validation failed: {0}")]
    InvalidProtocol(String),
    #[error("verification request canonicalization failed: {0}")]
    Canonicalization(String),
    #[error("invalid Ed25519 public key for {proof}")]
    InvalidPublicKey { proof: &'static str },
    #[error("invalid Ed25519 signature for {proof}")]
    InvalidSignature { proof: &'static str },
}

/// Verify all durable-key proofs bound into a request.
///
/// Final submission signatures prove named participants consented to the
/// exact replay envelope. Genesis and join signatures prove only the typed
/// signed statements; the service must already have checked each guest join
/// signature against the live transport EndpointId when accepting the join.
/// The replay verifier separately cross-binds those statements to the
/// anonymous-safe resimulated seat transcript.
pub fn authenticate_verification_request(
    request: VerificationRequestV1,
) -> Result<AuthenticatedVerificationRequest, RequestAuthenticationError> {
    request
        .validate()
        .map_err(|error| RequestAuthenticationError::InvalidProtocol(error.to_string()))?;
    let canonical_sha256 = request
        .canonical_digest()
        .map_err(|error| RequestAuthenticationError::Canonicalization(error.to_string()))?;

    let signed = &request.submission;
    let offer = &signed.submission.offer;
    let genesis = &offer.session_genesis;
    let genesis_bytes = genesis
        .signing_bytes()
        .map_err(|error| RequestAuthenticationError::Canonicalization(error.to_string()))?;
    verify_ed25519(
        "session_genesis",
        genesis.claim.host_public_key,
        genesis.host_signature,
        &genesis_bytes,
    )?;
    let session_genesis_sha256 = genesis
        .claim
        .canonical_digest()
        .map_err(|error| RequestAuthenticationError::Canonicalization(error.to_string()))?;

    for claim in offer.participant_claims.iter().skip(1) {
        let attestation = claim.join_attestation.as_ref().ok_or_else(|| {
            RequestAuthenticationError::InvalidProtocol(
                "named guest is missing its join attestation".into(),
            )
        })?;
        let bytes = attestation
            .signing_bytes()
            .map_err(|error| RequestAuthenticationError::Canonicalization(error.to_string()))?;
        verify_ed25519(
            "named_seat_join",
            claim.public_key,
            attestation.signature,
            &bytes,
        )?;
    }

    let submission_bytes = signed
        .signing_bytes()
        .map_err(|error| RequestAuthenticationError::Canonicalization(error.to_string()))?;
    for participant in &signed.participant_signatures {
        verify_ed25519(
            "submission_envelope",
            participant.public_key,
            participant.signature,
            &submission_bytes,
        )?;
    }

    Ok(AuthenticatedVerificationRequest {
        request,
        canonical_sha256,
        session_genesis_sha256,
    })
}

fn verify_ed25519(
    proof: &'static str,
    public_key: PublicKey32,
    signature: Signature64,
    message: &[u8],
) -> Result<(), RequestAuthenticationError> {
    robin_run_protocol::verify_ed25519_strict(public_key.as_bytes(), signature.as_bytes(), message)
        .map_err(|error| match error {
            robin_run_protocol::SignatureVerificationError::InvalidPublicKey => {
                RequestAuthenticationError::InvalidPublicKey { proof }
            }
            robin_run_protocol::SignatureVerificationError::InvalidSignature => {
                RequestAuthenticationError::InvalidSignature { proof }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    #[test]
    fn strict_ed25519_verification_binds_key_message_and_signature() {
        let signing_key = SigningKey::from_bytes(&rand::random());
        let other_key = SigningKey::from_bytes(&rand::random());
        let message = b"domain-separated canonical request bytes";
        let signature = signing_key.sign(message);
        let public_key = PublicKey32::from_bytes(signing_key.verifying_key().to_bytes());
        let signature = Signature64::from_bytes(signature.to_bytes());

        verify_ed25519("test", public_key, signature, message).unwrap();
        assert!(matches!(
            verify_ed25519("test", public_key, signature, b"different"),
            Err(RequestAuthenticationError::InvalidSignature { .. })
        ));
        assert!(matches!(
            verify_ed25519(
                "test",
                PublicKey32::from_bytes(other_key.verifying_key().to_bytes()),
                signature,
                message,
            ),
            Err(RequestAuthenticationError::InvalidSignature { .. })
        ));
    }

    #[test]
    fn malformed_key_and_signature_proof_fails_closed() {
        let error = verify_ed25519(
            "test",
            PublicKey32::from_bytes([0xff; 32]),
            Signature64::from_bytes([1; 64]),
            b"message",
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RequestAuthenticationError::InvalidPublicKey { proof: "test" }
                | RequestAuthenticationError::InvalidSignature { proof: "test" }
        ));
    }
}
