//! Native durable identity: the persistent iroh game key signs in-process.
//! Every operation completes without suspending.

use super::{GameIdentitySigner, LeaderboardSigningError, assemble_signed_request};
use ed25519_dalek::{Signer as _, SigningKey};
use robin_run_protocol::{
    PublicKey32, Signature64, SignedRequestClaim, SignedRequestV2,
    SignedSubmissionOwnerStatusRequestV2, SignedSubmissionV3, SubmissionOwnerStatusRequestV2,
    SubmissionV3,
};

pub struct NativeSigner;

impl NativeSigner {
    pub(crate) fn sign_username_update(
        request: robin_run_protocol::UsernameUpdateV2,
    ) -> Result<robin_run_protocol::SignedUsernameUpdateV2, LeaderboardSigningError> {
        sign_with_key(request, &native_key()?)
    }
}

fn native_key() -> Result<SigningKey, LeaderboardSigningError> {
    let seed = crate::native_game_identity::durable_game_identity_seed()
        .map_err(LeaderboardSigningError::Identity)?;
    Ok(SigningKey::from_bytes(&seed))
}

fn public_key(key: &SigningKey) -> PublicKey32 {
    PublicKey32::from_bytes(key.verifying_key().to_bytes())
}

/// Sign `request` with `key`, refusing a claim that names another identity.
fn sign_with_key<T: SignedRequestClaim>(
    request: T,
    key: &SigningKey,
) -> Result<SignedRequestV2<T>, LeaderboardSigningError> {
    if request.signer_public_key() != public_key(key) {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    let bytes = SignedRequestV2::<T>::signing_bytes(&request).map_err(|error| match error {
        robin_run_protocol::CanonicalDocumentError::Validation(error) => {
            LeaderboardSigningError::InvalidClaim(error.to_string())
        }
        other => LeaderboardSigningError::Canonical(other.to_string()),
    })?;
    let signature = Signature64::from_bytes(key.sign(&bytes).to_bytes());
    assemble_signed_request(request, public_key(key), signature)
}

impl GameIdentitySigner for NativeSigner {
    async fn public_key() -> Result<PublicKey32, LeaderboardSigningError> {
        Ok(public_key(&native_key()?))
    }

    async fn sign_submission(
        submission: SubmissionV3,
    ) -> Result<SignedSubmissionV3, LeaderboardSigningError> {
        sign_with_key(submission, &native_key()?)
    }

    async fn sign_submission_owner_status(
        request: SubmissionOwnerStatusRequestV2,
    ) -> Result<SignedSubmissionOwnerStatusRequestV2, LeaderboardSigningError> {
        sign_with_key(request, &native_key()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leaderboard::signing::verify_signed_request_for_tests;
    use crate::leaderboard::test_fixtures::submission;
    use robin_run_protocol::{OpaqueId, SCHEMA_VERSION_V2};

    #[test]
    fn username_registration_is_signed_by_the_same_native_identity() {
        let key = SigningKey::from_bytes(&[0x43; 32]);
        let request = robin_run_protocol::UsernameUpdateV2 {
            schema_version: SCHEMA_VERSION_V2,
            public_key: public_key(&key),
            signed_at_unix_ms: 1,
            username: "Robin".into(),
        };
        let mut signed = sign_with_key(request.clone(), &key).unwrap();
        verify_signed_request_for_tests(&signed).unwrap();
        signed.request.username = "Another name".into();
        assert!(verify_signed_request_for_tests(&signed).is_err());
        assert_eq!(
            sign_with_key(request, &SigningKey::from_bytes(&[0x44; 32])),
            Err(LeaderboardSigningError::WrongIdentity)
        );
    }

    #[test]
    fn native_submission_signature_binds_the_exact_v2_submission() {
        let key = SigningKey::from_bytes(&[0x43; 32]);
        let signed = sign_with_key(submission(public_key(&key)), &key).unwrap();
        verify_signed_request_for_tests(&signed).unwrap();
        let mut substituted = signed.clone();
        substituted.request.mission_id = "Demo_Lin".to_owned();
        assert!(verify_signed_request_for_tests(&substituted).is_err());
        let mut retimed = signed;
        retimed.request.signed_at_unix_ms += 1;
        assert!(verify_signed_request_for_tests(&retimed).is_err());
    }

    #[test]
    fn native_signer_refuses_a_submission_for_another_uploader() {
        let key = SigningKey::from_bytes(&[0x43; 32]);
        let other = PublicKey32::from_bytes([0x44; 32]);
        assert_eq!(
            sign_with_key(submission(other), &key),
            Err(LeaderboardSigningError::WrongIdentity)
        );
    }

    #[test]
    fn native_owner_status_signature_is_domain_separated_from_submissions() {
        let key = SigningKey::from_bytes(&[0x43; 32]);
        let request = SubmissionOwnerStatusRequestV2 {
            schema_version: SCHEMA_VERSION_V2,
            public_key: public_key(&key),
            signed_at_unix_ms: 1_800_000_000_000,
            submission_id: OpaqueId::new("submission-1").unwrap(),
        };
        let signed = sign_with_key(request, &key).unwrap();
        verify_signed_request_for_tests(&signed).unwrap();
        let mut submission = submission(public_key(&key));
        submission.signed_at_unix_ms = signed.request.signed_at_unix_ms;
        let foreign = SignedRequestV2 {
            schema_version: SCHEMA_VERSION_V2,
            request: submission,
            algorithm: signed.algorithm,
            signature: signed.signature,
        };
        assert!(verify_signed_request_for_tests(&foreign).is_err());
    }
}
