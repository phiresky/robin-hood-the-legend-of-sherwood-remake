//! Native durable identity: the persistent iroh game key signs in-process.
//! Every operation completes without suspending.

use super::{GameIdentitySigner, LeaderboardSigningError, canonical, invalid_claim};
use crate::leaderboard_ranked_session::{
    self as ranked, OfficialRankedSessionSetupV1, RankedSessionHost,
};
use ed25519_dalek::SigningKey;
use robin_run_protocol::{
    CampaignContinuationAuthorizationClaimV1, CampaignContinuationAuthorizationV1,
    CampaignContinuationPreflightRequestClaimV1, FreshRunPreflightRequestClaimV1,
    FreshRunPreflightRequestV1, LeaderboardCoSignRequestV1, ParticipantSignatureV1, PublicKey32,
    Signature64, SignatureAlgorithmV1, SubmissionEnvelopeV1, SubmissionOfferV1,
    SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1, Validate,
};

pub struct NativeSigner;

fn native_key() -> Result<SigningKey, LeaderboardSigningError> {
    let seed = crate::native_game_identity::durable_game_identity_seed()
        .map_err(LeaderboardSigningError::Identity)?;
    Ok(SigningKey::from_bytes(&seed))
}

impl GameIdentitySigner for NativeSigner {
    async fn public_key() -> Result<PublicKey32, LeaderboardSigningError> {
        let key = native_key()?;
        Ok(ranked::public_key(&key))
    }

    async fn create_official_ranked_session(
        network_protocol_version: u32,
        setup: OfficialRankedSessionSetupV1,
    ) -> Result<RankedSessionHost, LeaderboardSigningError> {
        RankedSessionHost::new_official(&native_key()?, network_protocol_version, setup)
            .map_err(invalid_claim)
    }

    async fn sign_submission_claim(
        envelope: &SubmissionEnvelopeV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
        envelope.validate().map_err(invalid_claim)?;
        let key = native_key()?;
        let public_key = ranked::public_key(&key);
        if !envelope
            .offer
            .participant_claims
            .iter()
            .any(|claim| claim.public_key == public_key)
        {
            return Err(LeaderboardSigningError::IdentityNotClaimed);
        }
        sign_co_sign_request_with_key(
            &envelope.co_sign_request().map_err(canonical_document)?,
            &key,
        )
    }

    async fn sign_multiplayer_leaderboard_request(
        request: &LeaderboardCoSignRequestV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
        sign_co_sign_request_with_key(request, &native_key()?)
    }

    async fn sign_fresh_run_preflight_request(
        claim: FreshRunPreflightRequestClaimV1,
    ) -> Result<FreshRunPreflightRequestV1, LeaderboardSigningError> {
        claim.validate().map_err(invalid_claim)?;
        let key = native_key()?;
        if claim.host_public_key != ranked::public_key(&key) {
            return Err(LeaderboardSigningError::WrongIdentity);
        }
        let signed = FreshRunPreflightRequestV1 {
            host_signature: ranked::signature(&key, &canonical(claim.signing_bytes())?),
            claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        signed.validate().map_err(invalid_claim)?;
        Ok(signed)
    }

    async fn sign_campaign_continuation_preflight_as_host(
        claim: &CampaignContinuationPreflightRequestClaimV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
        sign_campaign_continuation_preflight_claim_with_key(claim, false, &native_key()?)
    }

    async fn sign_campaign_continuation_preflight_as_controller(
        claim: &CampaignContinuationPreflightRequestClaimV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
        sign_campaign_continuation_preflight_claim_with_key(claim, true, &native_key()?)
    }

    async fn sign_campaign_continuation(
        offer: &SubmissionOfferV1,
        claim: CampaignContinuationAuthorizationClaimV1,
    ) -> Result<CampaignContinuationAuthorizationV1, LeaderboardSigningError> {
        let request = claim.co_sign_request(offer).map_err(canonical_document)?;
        let key = native_key()?;
        if claim.campaign_controller_public_key != ranked::public_key(&key) {
            return Err(LeaderboardSigningError::WrongIdentity);
        }
        let signature = sign_co_sign_request_with_key(&request, &key)?.signature;
        let signed = CampaignContinuationAuthorizationV1 {
            claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
            signature,
        };
        signed.validate().map_err(invalid_claim)?;
        Ok(signed)
    }

    async fn sign_submission_owner_status(
        challenge: SubmissionOwnerStatusChallengeV1,
    ) -> Result<SubmissionOwnerStatusEnvelopeV1, LeaderboardSigningError> {
        challenge.validate().map_err(invalid_claim)?;
        let key = native_key()?;
        if challenge.controller_public_key != ranked::public_key(&key) {
            return Err(LeaderboardSigningError::WrongIdentity);
        }
        let mut envelope = SubmissionOwnerStatusEnvelopeV1 {
            schema_version: challenge.schema_version,
            challenge,
            algorithm: SignatureAlgorithmV1::Ed25519,
            signature: Signature64::from_bytes([0; 64]),
        };
        envelope.validate_signing_claim().map_err(invalid_claim)?;
        envelope.signature = ranked::signature(&key, &canonical(envelope.signing_bytes())?);
        envelope.validate().map_err(invalid_claim)?;
        Ok(envelope)
    }
}

fn sign_co_sign_request_with_key(
    request: &LeaderboardCoSignRequestV1,
    key: &SigningKey,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    let bytes = request.signing_bytes().map_err(invalid_claim)?;
    Ok(ParticipantSignatureV1 {
        public_key: ranked::public_key(key),
        signature: ranked::signature(key, &bytes),
    })
}

fn sign_campaign_continuation_preflight_claim_with_key(
    claim: &CampaignContinuationPreflightRequestClaimV1,
    controller: bool,
    key: &SigningKey,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    claim.validate().map_err(invalid_claim)?;
    let public_key = ranked::public_key(key);
    let (expected_key, bytes) = if controller {
        (
            claim.campaign_controller_public_key,
            claim.controller_signing_bytes(),
        )
    } else {
        (claim.host_public_key, claim.host_signing_bytes())
    };
    if public_key != expected_key {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    Ok(ParticipantSignatureV1 {
        public_key,
        signature: ranked::signature(key, &canonical(bytes)?),
    })
}

fn canonical_document(
    error: robin_run_protocol::CanonicalDocumentError,
) -> LeaderboardSigningError {
    LeaderboardSigningError::Canonical(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::{Digest32, LeaderboardCoSignInstanceV1, LeaderboardCoSignPurposeV1};

    #[test]
    fn native_multiplayer_signer_signs_the_protocol_payload_verbatim() {
        let secret = SigningKey::from_bytes(&[0x43; 32]);
        let request = LeaderboardCoSignRequestV1 {
            instance: LeaderboardCoSignInstanceV1 {
                purpose: LeaderboardCoSignPurposeV1::Submission,
                replay_session_id: Digest32::from_bytes([0x81; 32]),
                submission_offer_sha256: Digest32::from_bytes([0x82; 32]),
            },
            run_digest: Digest32::from_bytes([0x83; 32]),
        };
        let signed = sign_co_sign_request_with_key(&request, &secret).unwrap();
        assert_eq!(signed.public_key, ranked::public_key(&secret));
        let signature = ed25519_dalek::Signature::from_bytes(signed.signature.as_bytes());
        secret
            .verifying_key()
            .verify_strict(&request.signing_bytes().unwrap(), &signature)
            .unwrap();

        let mut substituted = request;
        substituted.instance.purpose = LeaderboardCoSignPurposeV1::CampaignContinuation;
        assert!(
            secret
                .verifying_key()
                .verify_strict(&substituted.signing_bytes().unwrap(), &signature)
                .is_err()
        );
    }
}
