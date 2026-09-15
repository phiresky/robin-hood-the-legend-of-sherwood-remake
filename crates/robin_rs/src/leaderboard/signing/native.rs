//! Native durable identity: the persistent iroh game key signs in-process.
//! Every operation completes without suspending.

use super::{
    GameIdentitySigner, LeaderboardSigningError, assemble_signed_submission, invalid_claim,
};
use ed25519_dalek::{Signer as _, SigningKey};
use robin_run_protocol::{
    PublicKey32, Signature64, SignatureAlgorithmV1, SignedSubmissionV2,
    SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1, SubmissionV2, Validate,
};

pub struct NativeSigner;

fn native_key() -> Result<SigningKey, LeaderboardSigningError> {
    let seed = crate::native_game_identity::durable_game_identity_seed()
        .map_err(LeaderboardSigningError::Identity)?;
    Ok(SigningKey::from_bytes(&seed))
}

fn public_key(key: &SigningKey) -> PublicKey32 {
    PublicKey32::from_bytes(key.verifying_key().to_bytes())
}

fn signature(key: &SigningKey, bytes: &[u8]) -> Signature64 {
    Signature64::from_bytes(key.sign(bytes).to_bytes())
}

/// Bytes the uploader signs for `submission`, validating the claim first.
fn submission_signing_bytes(submission: &SubmissionV2) -> Result<Vec<u8>, LeaderboardSigningError> {
    SignedSubmissionV2::signing_bytes(submission)
        .map_err(|error| LeaderboardSigningError::Canonical(error.to_string()))
}

fn sign_submission_with_key(
    submission: SubmissionV2,
    key: &SigningKey,
) -> Result<SignedSubmissionV2, LeaderboardSigningError> {
    if submission.uploader_public_key != public_key(key) {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    let signature = signature(key, &submission_signing_bytes(&submission)?);
    assemble_signed_submission(submission, public_key(key), signature)
}

impl GameIdentitySigner for NativeSigner {
    async fn public_key() -> Result<PublicKey32, LeaderboardSigningError> {
        Ok(public_key(&native_key()?))
    }

    async fn sign_submission(
        submission: SubmissionV2,
    ) -> Result<SignedSubmissionV2, LeaderboardSigningError> {
        sign_submission_with_key(submission, &native_key()?)
    }

    async fn sign_submission_owner_status(
        challenge: SubmissionOwnerStatusChallengeV1,
    ) -> Result<SubmissionOwnerStatusEnvelopeV1, LeaderboardSigningError> {
        challenge.validate().map_err(invalid_claim)?;
        let key = native_key()?;
        if challenge.controller_public_key != public_key(&key) {
            return Err(LeaderboardSigningError::WrongIdentity);
        }
        let mut envelope = SubmissionOwnerStatusEnvelopeV1 {
            schema_version: challenge.schema_version,
            challenge,
            algorithm: SignatureAlgorithmV1::Ed25519,
            signature: Signature64::from_bytes([0; 64]),
        };
        envelope.validate_signing_claim().map_err(invalid_claim)?;
        let bytes = envelope
            .signing_bytes()
            .map_err(|error| LeaderboardSigningError::Canonical(error.to_string()))?;
        envelope.signature = signature(&key, &bytes);
        envelope.validate().map_err(invalid_claim)?;
        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leaderboard::test_fixtures::submission;

    #[test]
    fn native_submission_signature_binds_the_exact_v2_submission() {
        let key = SigningKey::from_bytes(&[0x43; 32]);
        let signed = sign_submission_with_key(submission(public_key(&key)), &key).unwrap();
        signed.verify_signature().unwrap();
        let mut substituted = signed.clone();
        substituted.submission.mission_id = "Demo_Lin".to_owned();
        assert!(substituted.verify_signature().is_err());
    }

    #[test]
    fn native_signer_refuses_a_submission_for_another_uploader() {
        let key = SigningKey::from_bytes(&[0x43; 32]);
        let other = PublicKey32::from_bytes([0x44; 32]);
        assert_eq!(
            sign_submission_with_key(submission(other), &key),
            Err(LeaderboardSigningError::WrongIdentity)
        );
    }
}
