//! Cryptographic policy shared by independent ranked trust boundaries.
//! Callers must still validate document shape, authorization and local evidence.

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignatureVerificationError {
    #[error("invalid Ed25519 public key")]
    InvalidPublicKey,
    #[error("invalid Ed25519 signature")]
    InvalidSignature,
}

pub fn verify_ed25519_strict(
    public_key: &[u8; 32],
    signature: &[u8; 64],
    message: &[u8],
) -> Result<(), SignatureVerificationError> {
    let key = ed25519_dalek::VerifyingKey::from_bytes(public_key)
        .map_err(|_| SignatureVerificationError::InvalidPublicKey)?;
    key.verify_strict(message, &ed25519_dalek::Signature::from_bytes(signature))
        .map_err(|_| SignatureVerificationError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    #[test]
    fn strict_policy_binds_exact_message_and_rejects_weak_identity_key() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let signature = key.sign(b"exact").to_bytes();
        assert!(
            verify_ed25519_strict(&key.verifying_key().to_bytes(), &signature, b"exact").is_ok()
        );
        assert!(
            verify_ed25519_strict(&key.verifying_key().to_bytes(), &signature, b"changed").is_err()
        );
        let mut identity = [0; 32];
        identity[0] = 1;
        let mut weak_signature = [0; 64];
        weak_signature[..32].copy_from_slice(&identity);
        assert_eq!(
            verify_ed25519_strict(&identity, &weak_signature, b"exact"),
            Err(SignatureVerificationError::InvalidSignature)
        );
    }
}
