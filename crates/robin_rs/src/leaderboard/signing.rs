//! Closed, typed signing adapter for the durable game identity.
//!
//! Native builds reuse the persistent iroh game key. Browser builds delegate
//! to the isolated signer origin, whose IndexedDB record is shared with the
//! Feature 38 multiplayer seat-proof protocol. Neither path exposes a raw or
//! generic signing API to callers.

use crate::leaderboard_ranked_session::{OfficialRankedSessionSetupV1, RankedSessionHost};
#[cfg(not(target_arch = "wasm32"))]
use ed25519_dalek::{Signer, SigningKey};
#[cfg(all(test, not(target_arch = "wasm32")))]
use robin_run_protocol::LeaderboardCoSignPurposeV1;
use robin_run_protocol::{
    CampaignContinuationAuthorizationClaimV1, CampaignContinuationAuthorizationV1,
    CampaignContinuationPreflightRequestClaimV1, CampaignContinuationPreflightRequestV1,
    CompetitionRunGrantRequestClaimV1, CompetitionRunGrantRequestV1, DeletionRequestEnvelopeV1,
    FreshRunPreflightRequestClaimV1, FreshRunPreflightRequestV1, LeaderboardCoSignRequestV1,
    ParticipantSignatureV1, PublicKey32, Signature64, SignatureAlgorithmV1, SubmissionEnvelopeV1,
    SubmissionOfferV1, SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1,
    UsernameUpdateEnvelopeV1, Validate,
};
#[cfg(target_arch = "wasm32")]
use robin_run_protocol::{
    NamedSeatJoinAttestationV1, NamedSeatJoinClaimV1, ReplaySessionGenesisClaimV1,
    ReplaySessionGenesisV1,
};

#[cfg(target_arch = "wasm32")]
const MAX_RANKED_DOCUMENT_BYTES: usize = 128 * 1024;

#[cfg(target_arch = "wasm32")]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserCampaignContinuationSigningInput {
    offer: SubmissionOfferV1,
    claim: CampaignContinuationAuthorizationClaimV1,
}

pub const LEADERBOARD_WEB_ORIGIN_ENV: &str = "ROBINHOOD_LEADERBOARD_WEB_ORIGIN";
pub const IDENTITY_SIGNER_ORIGIN_ENV: &str = "ROBINHOOD_IDENTITY_SIGNER_ORIGIN";

#[cfg(target_arch = "wasm32")]
const DEFAULT_SIGNER_ORIGIN: &str = "https://identity.robinhood.phiresky.xyz";
#[cfg(target_arch = "wasm32")]
const DEPLOYMENT_SIGNER_ORIGIN: Option<&str> = option_env!("ROBINHOOD_IDENTITY_SIGNER_ORIGIN");

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LeaderboardSigningError {
    #[error("durable game identity is unavailable: {0}")]
    Identity(String),
    #[error("leaderboard signing claim is invalid: {0}")]
    InvalidClaim(String),
    #[error("leaderboard signing claim names a different public key")]
    WrongIdentity,
    #[error("leaderboard submission does not claim this public key")]
    IdentityNotClaimed,
    #[error("canonical leaderboard signing failed: {0}")]
    Canonical(String),
    #[error("leaderboard bridge document exceeds {maximum} bytes")]
    DocumentTooLarge { maximum: usize },
    #[error("leaderboard bridge document is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("leaderboard bridge caller origin does not match this deployment")]
    OriginNotAuthorized,
    #[error("the leaderboard identity signer must run in its isolated embedded document")]
    SignerContext,
}

#[cfg(not(target_arch = "wasm32"))]
fn native_key() -> Result<SigningKey, LeaderboardSigningError> {
    let seed = crate::native_game_identity::durable_game_identity_seed()
        .map_err(LeaderboardSigningError::Identity)?;
    Ok(SigningKey::from_bytes(&seed))
}

/// Create the signed official ranked-session lifecycle with the durable game
/// identity without exposing that secret key to mission bootstrap code.
#[cfg(not(target_arch = "wasm32"))]
pub fn create_native_official_ranked_session(
    network_protocol_version: u32,
    setup: OfficialRankedSessionSetupV1,
) -> Result<RankedSessionHost, LeaderboardSigningError> {
    RankedSessionHost::new_official(&native_key()?, network_protocol_version, setup)
        .map_err(invalid_claim)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn local_public_key() -> Result<PublicKey32, LeaderboardSigningError> {
    let key = native_key()?;
    Ok(native_public_key(&key))
}

#[cfg(not(target_arch = "wasm32"))]
fn native_public_key(key: &SigningKey) -> PublicKey32 {
    PublicKey32::from_bytes(key.verifying_key().to_bytes())
}

#[cfg(not(target_arch = "wasm32"))]
fn native_signature(key: &SigningKey, bytes: &[u8]) -> Signature64 {
    Signature64::from_bytes(key.sign(bytes).to_bytes())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn sign_username_update(
    envelope: UsernameUpdateEnvelopeV1,
) -> Result<UsernameUpdateEnvelopeV1, LeaderboardSigningError> {
    sign_username_update_with_key(envelope, &native_key()?)
}

#[cfg(not(target_arch = "wasm32"))]
fn sign_username_update_with_key(
    mut envelope: UsernameUpdateEnvelopeV1,
    key: &SigningKey,
) -> Result<UsernameUpdateEnvelopeV1, LeaderboardSigningError> {
    envelope.validate_signing_claim().map_err(invalid_claim)?;
    if envelope.public_key != native_public_key(key) {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    envelope.signature = native_signature(key, &canonical(envelope.signing_bytes())?);
    envelope.validate().map_err(invalid_claim)?;
    Ok(envelope)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn sign_submission_claim(
    envelope: &SubmissionEnvelopeV1,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    envelope.validate().map_err(invalid_claim)?;
    let key = native_key()?;
    let public_key = native_public_key(&key);
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

/// Sign the closed fixed-size request received from the multiplayer transport.
/// The transport is responsible for accepting only the exact request installed
/// by the locally validated mission-end flow. This API cannot sign caller-
/// supplied bytes or a purpose outside the V1 co-sign contract.
#[cfg(not(target_arch = "wasm32"))]
pub fn sign_multiplayer_leaderboard_request(
    request: &LeaderboardCoSignRequestV1,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    sign_co_sign_request_with_key(request, &native_key()?)
}

#[cfg(not(target_arch = "wasm32"))]
fn sign_co_sign_request_with_key(
    request: &LeaderboardCoSignRequestV1,
    key: &SigningKey,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    let bytes = request.signing_bytes().map_err(invalid_claim)?;
    Ok(ParticipantSignatureV1 {
        public_key: native_public_key(key),
        signature: native_signature(key, &bytes),
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn sign_competition_run_grant_request(
    claim: CompetitionRunGrantRequestClaimV1,
) -> Result<CompetitionRunGrantRequestV1, LeaderboardSigningError> {
    claim.validate().map_err(invalid_claim)?;
    let key = native_key()?;
    if claim.host_public_key != native_public_key(&key) {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    let host_signature = native_signature(&key, &canonical(claim.signing_bytes())?);
    let signed = CompetitionRunGrantRequestV1 {
        claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
        host_signature,
    };
    signed.validate().map_err(invalid_claim)?;
    Ok(signed)
}

/// Sign the only host-authored fresh-run preflight claim. Callers cannot
/// supply a domain or arbitrary bytes, and the claim must name this install's
/// durable game identity.
#[cfg(not(target_arch = "wasm32"))]
pub fn sign_fresh_run_preflight_request(
    claim: FreshRunPreflightRequestClaimV1,
) -> Result<FreshRunPreflightRequestV1, LeaderboardSigningError> {
    claim.validate().map_err(invalid_claim)?;
    let key = native_key()?;
    if claim.host_public_key != native_public_key(&key) {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    let signed = FreshRunPreflightRequestV1 {
        host_signature: native_signature(&key, &canonical(claim.signing_bytes())?),
        claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
    };
    signed.validate().map_err(invalid_claim)?;
    Ok(signed)
}

#[cfg(not(target_arch = "wasm32"))]
fn sign_campaign_continuation_preflight_claim_with_key(
    claim: &CampaignContinuationPreflightRequestClaimV1,
    controller: bool,
    key: &SigningKey,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    claim.validate().map_err(invalid_claim)?;
    let public_key = native_public_key(key);
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
        signature: native_signature(key, &canonical(bytes)?),
    })
}

/// Host half of the dual-authorized campaign continuation preflight request.
#[cfg(not(target_arch = "wasm32"))]
pub fn sign_campaign_continuation_preflight_as_host(
    claim: &CampaignContinuationPreflightRequestClaimV1,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    sign_campaign_continuation_preflight_claim_with_key(claim, false, &native_key()?)
}

/// Controller half of the dual-authorized campaign continuation preflight
/// request. This is the only operation exposed to a remote controller peer.
#[cfg(not(target_arch = "wasm32"))]
pub fn sign_campaign_continuation_preflight_as_controller(
    claim: &CampaignContinuationPreflightRequestClaimV1,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    sign_campaign_continuation_preflight_claim_with_key(claim, true, &native_key()?)
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

#[cfg(not(target_arch = "wasm32"))]
pub fn sign_campaign_continuation(
    offer: &SubmissionOfferV1,
    claim: CampaignContinuationAuthorizationClaimV1,
) -> Result<CampaignContinuationAuthorizationV1, LeaderboardSigningError> {
    let request = claim.co_sign_request(offer).map_err(canonical_document)?;
    let key = native_key()?;
    if claim.campaign_controller_public_key != native_public_key(&key) {
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

#[cfg(not(target_arch = "wasm32"))]
pub fn sign_submission_owner_status(
    challenge: SubmissionOwnerStatusChallengeV1,
) -> Result<SubmissionOwnerStatusEnvelopeV1, LeaderboardSigningError> {
    challenge.validate().map_err(invalid_claim)?;
    let key = native_key()?;
    if challenge.controller_public_key != native_public_key(&key) {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    let mut envelope = SubmissionOwnerStatusEnvelopeV1 {
        schema_version: challenge.schema_version,
        challenge,
        algorithm: SignatureAlgorithmV1::Ed25519,
        signature: Signature64::from_bytes([0; 64]),
    };
    envelope.validate_signing_claim().map_err(invalid_claim)?;
    envelope.signature = native_signature(&key, &canonical(envelope.signing_bytes())?);
    envelope.validate().map_err(invalid_claim)?;
    Ok(envelope)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn sign_deletion_request(
    envelope: DeletionRequestEnvelopeV1,
) -> Result<DeletionRequestEnvelopeV1, LeaderboardSigningError> {
    sign_deletion_request_with_key(envelope, &native_key()?)
}

#[cfg(not(target_arch = "wasm32"))]
fn sign_deletion_request_with_key(
    mut envelope: DeletionRequestEnvelopeV1,
    key: &SigningKey,
) -> Result<DeletionRequestEnvelopeV1, LeaderboardSigningError> {
    envelope.validate_signing_claim().map_err(invalid_claim)?;
    if envelope.challenge.public_key != native_public_key(key) {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    envelope.signature = native_signature(key, &canonical(envelope.signing_bytes())?);
    envelope.validate().map_err(invalid_claim)?;
    Ok(envelope)
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

#[cfg(not(target_arch = "wasm32"))]
fn canonical_document(
    error: robin_run_protocol::CanonicalDocumentError,
) -> LeaderboardSigningError {
    LeaderboardSigningError::Canonical(error.to_string())
}

fn invalid_claim(error: impl std::fmt::Display) -> LeaderboardSigningError {
    LeaderboardSigningError::InvalidClaim(error.to_string())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(module = "/js/browser_identity_client.js")]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(
        catch,
        js_name = robinhoodRequestLeaderboardIdentity
    )]
    async fn request_browser_identity(
        signer_origin: &str,
        operation: &str,
        payload_json: wasm_bindgen::JsValue,
    ) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
}

#[cfg(target_arch = "wasm32")]
fn js_error(action: &str, value: wasm_bindgen::JsValue) -> LeaderboardSigningError {
    LeaderboardSigningError::Identity(format!(
        "{action}: {}",
        value.as_string().unwrap_or_else(|| format!("{value:?}"))
    ))
}

#[cfg(target_arch = "wasm32")]
fn required_js_string(
    action: &str,
    value: wasm_bindgen::JsValue,
) -> Result<String, LeaderboardSigningError> {
    value.as_string().ok_or_else(|| {
        LeaderboardSigningError::Identity(format!("{action} returned a non-string value"))
    })
}

#[cfg(target_arch = "wasm32")]
fn decode_json<T: serde::de::DeserializeOwned>(
    json: &str,
    maximum: usize,
) -> Result<T, LeaderboardSigningError> {
    if json.len() > maximum {
        return Err(LeaderboardSigningError::DocumentTooLarge { maximum });
    }
    serde_json::from_str(json)
        .map_err(|error| LeaderboardSigningError::InvalidJson(error.to_string()))
}

#[cfg(target_arch = "wasm32")]
#[cfg(target_arch = "wasm32")]
fn expected_signer_origin() -> &'static str {
    DEPLOYMENT_SIGNER_ORIGIN.unwrap_or(DEFAULT_SIGNER_ORIGIN)
}

#[cfg(target_arch = "wasm32")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserPublicKeyResult {
    kind: String,
    #[serde(rename = "publicKey")]
    public_key: String,
}

#[cfg(target_arch = "wasm32")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserSignedDocumentResult {
    kind: String,
    #[serde(rename = "documentJson")]
    document_json: String,
}

#[cfg(target_arch = "wasm32")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserParticipantSignatureResult {
    kind: String,
    #[serde(rename = "participantSignatureJson")]
    participant_signature_json: String,
}

#[cfg(target_arch = "wasm32")]
async fn request_browser_operation(
    operation: &str,
    payload_json: Option<String>,
) -> Result<String, LeaderboardSigningError> {
    let signer_origin = expected_signer_origin();
    let payload = payload_json.map_or(
        wasm_bindgen::JsValue::UNDEFINED,
        wasm_bindgen::JsValue::from,
    );
    let value = request_browser_identity(signer_origin, operation, payload)
        .await
        .map_err(|error| js_error(operation, error))?;
    required_js_string(operation, value)
}

#[cfg(target_arch = "wasm32")]
fn decode_browser_result<T: serde::de::DeserializeOwned>(
    operation: &str,
    json: &str,
) -> Result<T, LeaderboardSigningError> {
    decode_json(json, MAX_RANKED_DOCUMENT_BYTES).map_err(|error| {
        LeaderboardSigningError::Identity(format!("{operation} signer response: {error}"))
    })
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_public_key() -> Result<PublicKey32, LeaderboardSigningError> {
    let response = request_browser_operation("public_key", None).await?;
    let result: BrowserPublicKeyResult = decode_browser_result("public_key", &response)?;
    if result.kind != "public_key" {
        return Err(LeaderboardSigningError::Identity(
            "public_key response has the wrong kind".to_owned(),
        ));
    }
    result
        .public_key
        .parse()
        .map_err(|error| LeaderboardSigningError::Identity(format!("invalid public key: {error}")))
}

#[cfg(target_arch = "wasm32")]
async fn browser_signed_document<T>(
    operation: &str,
    value: &impl serde::Serialize,
) -> Result<T, LeaderboardSigningError>
where
    T: serde::de::DeserializeOwned + Validate,
{
    let payload = serde_json::to_string(value)
        .map_err(|error| LeaderboardSigningError::InvalidJson(error.to_string()))?;
    let response = request_browser_operation(operation, Some(payload)).await?;
    let result: BrowserSignedDocumentResult = decode_browser_result(operation, &response)?;
    if result.kind != "signed_document" {
        return Err(LeaderboardSigningError::Identity(format!(
            "{operation} response has the wrong kind"
        )));
    }
    let result: T = decode_browser_result(operation, &result.document_json)?;
    result.validate().map_err(invalid_claim)?;
    Ok(result)
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_username_update(
    envelope: &UsernameUpdateEnvelopeV1,
) -> Result<UsernameUpdateEnvelopeV1, LeaderboardSigningError> {
    let signed = browser_signed_document("sign_username_update", envelope).await?;
    let signed: UsernameUpdateEnvelopeV1 = signed;
    if signed.schema_version != envelope.schema_version
        || signed.username_challenge_id != envelope.username_challenge_id
        || signed.username_challenge_nonce != envelope.username_challenge_nonce
        || signed.public_key != envelope.public_key
        || signed.username != envelope.username
    {
        return Err(LeaderboardSigningError::InvalidClaim(
            "signer changed the username update claim".to_owned(),
        ));
    }
    Ok(signed)
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_competition_run_grant_request(
    claim: &CompetitionRunGrantRequestClaimV1,
) -> Result<CompetitionRunGrantRequestV1, LeaderboardSigningError> {
    let signed = browser_signed_document("sign_competition_run_grant_request", claim).await?;
    let signed: CompetitionRunGrantRequestV1 = signed;
    if &signed.claim != claim {
        return Err(LeaderboardSigningError::InvalidClaim(
            "signer changed the competition grant request".to_owned(),
        ));
    }
    Ok(signed)
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_fresh_run_preflight_request(
    claim: &FreshRunPreflightRequestClaimV1,
) -> Result<FreshRunPreflightRequestV1, LeaderboardSigningError> {
    let signed = browser_signed_document("sign_fresh_run_preflight_request", claim).await?;
    let signed: FreshRunPreflightRequestV1 = signed;
    if &signed.claim != claim {
        return Err(LeaderboardSigningError::InvalidClaim(
            "signer changed the fresh-run preflight claim".to_owned(),
        ));
    }
    verify_typed_signature(
        signed.claim.host_public_key,
        &canonical(signed.claim.signing_bytes())?,
        signed.host_signature,
    )?;
    Ok(signed)
}

#[cfg(target_arch = "wasm32")]
async fn browser_game_sign_campaign_continuation_preflight(
    operation: &str,
    claim: &CampaignContinuationPreflightRequestClaimV1,
    controller: bool,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    claim.validate().map_err(invalid_claim)?;
    let payload = serde_json::to_string(claim)
        .map_err(|error| LeaderboardSigningError::InvalidJson(error.to_string()))?;
    let response = request_browser_operation(operation, Some(payload)).await?;
    let result: BrowserParticipantSignatureResult = decode_browser_result(operation, &response)?;
    if result.kind != "participant_signature" {
        return Err(LeaderboardSigningError::Identity(format!(
            "{operation} response has the wrong kind"
        )));
    }
    let signed: ParticipantSignatureV1 =
        decode_browser_result(operation, &result.participant_signature_json)?;
    let (expected_key, bytes) = if controller {
        (
            claim.campaign_controller_public_key,
            claim.controller_signing_bytes(),
        )
    } else {
        (claim.host_public_key, claim.host_signing_bytes())
    };
    if signed.public_key != expected_key {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    verify_typed_signature(signed.public_key, &canonical(bytes)?, signed.signature)?;
    Ok(signed)
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_campaign_continuation_preflight_as_host(
    claim: &CampaignContinuationPreflightRequestClaimV1,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    browser_game_sign_campaign_continuation_preflight(
        "sign_campaign_continuation_preflight_as_host",
        claim,
        false,
    )
    .await
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_campaign_continuation_preflight_as_controller(
    claim: &CampaignContinuationPreflightRequestClaimV1,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    browser_game_sign_campaign_continuation_preflight(
        "sign_campaign_continuation_preflight_as_controller",
        claim,
        true,
    )
    .await
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_campaign_continuation(
    offer: &SubmissionOfferV1,
    claim: &CampaignContinuationAuthorizationClaimV1,
) -> Result<CampaignContinuationAuthorizationV1, LeaderboardSigningError> {
    let input = BrowserCampaignContinuationSigningInput {
        offer: offer.clone(),
        claim: claim.clone(),
    };
    let signed = browser_signed_document("sign_campaign_continuation", &input).await?;
    let signed: CampaignContinuationAuthorizationV1 = signed;
    if &signed.claim != claim {
        return Err(LeaderboardSigningError::InvalidClaim(
            "signer changed the campaign continuation claim".to_owned(),
        ));
    }
    Ok(signed)
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_submission_owner_status(
    challenge: &SubmissionOwnerStatusChallengeV1,
) -> Result<SubmissionOwnerStatusEnvelopeV1, LeaderboardSigningError> {
    let signed = browser_signed_document("sign_submission_owner_status", challenge).await?;
    let signed: SubmissionOwnerStatusEnvelopeV1 = signed;
    if &signed.challenge != challenge {
        return Err(LeaderboardSigningError::InvalidClaim(
            "signer changed the owner-status challenge".to_owned(),
        ));
    }
    Ok(signed)
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_deletion_request(
    envelope: &DeletionRequestEnvelopeV1,
) -> Result<DeletionRequestEnvelopeV1, LeaderboardSigningError> {
    let signed = browser_signed_document("sign_deletion_request", envelope).await?;
    let signed: DeletionRequestEnvelopeV1 = signed;
    if signed.schema_version != envelope.schema_version || signed.challenge != envelope.challenge {
        return Err(LeaderboardSigningError::InvalidClaim(
            "signer changed the deletion request claim".to_owned(),
        ));
    }
    Ok(signed)
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_submission_claim(
    envelope: &SubmissionEnvelopeV1,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    let payload = serde_json::to_string(envelope)
        .map_err(|error| LeaderboardSigningError::InvalidJson(error.to_string()))?;
    let response = request_browser_operation("sign_submission", Some(payload)).await?;
    let result: BrowserParticipantSignatureResult =
        decode_browser_result("sign_submission", &response)?;
    if result.kind != "participant_signature" {
        return Err(LeaderboardSigningError::Identity(
            "sign_submission response has the wrong kind".to_owned(),
        ));
    }
    let participant: ParticipantSignatureV1 =
        decode_browser_result("sign_submission", &result.participant_signature_json)?;
    if participant.public_key != browser_game_public_key().await? || participant.signature.is_zero()
    {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    Ok(participant)
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_multiplayer_leaderboard_request(
    request: &LeaderboardCoSignRequestV1,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    request.validate().map_err(invalid_claim)?;
    let payload = serde_json::to_string(request)
        .map_err(|error| LeaderboardSigningError::InvalidJson(error.to_string()))?;
    let response =
        request_browser_operation("sign_multiplayer_leaderboard_request", Some(payload)).await?;
    let result: BrowserParticipantSignatureResult =
        decode_browser_result("sign_multiplayer_leaderboard_request", &response)?;
    if result.kind != "participant_signature" {
        return Err(LeaderboardSigningError::Identity(
            "sign_multiplayer_leaderboard_request response has the wrong kind".to_owned(),
        ));
    }
    let participant: ParticipantSignatureV1 = decode_browser_result(
        "sign_multiplayer_leaderboard_request",
        &result.participant_signature_json,
    )?;
    if participant.public_key != browser_game_public_key().await? || participant.signature.is_zero()
    {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    Ok(participant)
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_named_seat_join(
    claim: &NamedSeatJoinClaimV1,
) -> Result<NamedSeatJoinAttestationV1, LeaderboardSigningError> {
    claim.validate().map_err(invalid_claim)?;
    let signed = browser_signed_document("sign_named_seat_join", claim).await?;
    let signed: NamedSeatJoinAttestationV1 = signed;
    if &signed.claim != claim {
        return Err(LeaderboardSigningError::InvalidClaim(
            "signer changed the named-seat join claim".to_owned(),
        ));
    }
    if signed.claim.public_key != browser_game_public_key().await? {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    verify_typed_signature(
        signed.claim.public_key,
        &canonical(signed.claim.signing_bytes())?,
        signed.signature,
    )?;
    Ok(signed)
}

#[cfg(target_arch = "wasm32")]
pub async fn browser_game_sign_replay_session_genesis(
    claim: &ReplaySessionGenesisClaimV1,
) -> Result<ReplaySessionGenesisV1, LeaderboardSigningError> {
    claim.validate().map_err(invalid_claim)?;
    let signed = browser_signed_document("sign_replay_session_genesis", claim).await?;
    let signed: ReplaySessionGenesisV1 = signed;
    if &signed.claim != claim {
        return Err(LeaderboardSigningError::InvalidClaim(
            "signer changed the replay session genesis claim".to_owned(),
        ));
    }
    if signed.claim.host_public_key != browser_game_public_key().await? {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    crate::leaderboard_ranked_session::validate_session_genesis(
        &signed,
        *claim.host_public_key.as_bytes(),
        &claim.ranked_session,
    )
    .map_err(|error| LeaderboardSigningError::InvalidClaim(error.to_string()))?;
    Ok(signed)
}

/// Create a browser-hosted official ranked session through the one closed
/// genesis operation. Bootstrap code never sees raw key material and cannot
/// replace the claim between construction, signing, and verification.
#[cfg(target_arch = "wasm32")]
pub async fn create_browser_official_ranked_session(
    network_protocol_version: u32,
    setup: OfficialRankedSessionSetupV1,
) -> Result<RankedSessionHost, LeaderboardSigningError> {
    let claim = RankedSessionHost::prepare_official_genesis_claim(
        browser_game_public_key().await?,
        network_protocol_version,
        setup.clone(),
    )
    .map_err(invalid_claim)?;
    let genesis = browser_game_sign_replay_session_genesis(&claim).await?;
    crate::leaderboard_ranked_session::validate_official_session_genesis(
        &genesis,
        *claim.host_public_key.as_bytes(),
        &setup,
    )
    .map_err(invalid_claim)?;
    RankedSessionHost::from_signed_genesis(genesis).map_err(invalid_claim)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use robin_run_protocol::{
        ChallengeNonce32, DeletionChallengeV1, DeletionTargetV1, Digest32,
        LeaderboardCoSignInstanceV1, OpaqueId,
    };

    fn id(value: &str) -> OpaqueId {
        OpaqueId::new(value).unwrap()
    }

    #[test]
    fn native_username_and_deletion_use_one_durable_key() {
        let secret = SigningKey::from_bytes(&[0x41; 32]);
        let key = native_public_key(&secret);
        let username = UsernameUpdateEnvelopeV1 {
            schema_version: 1,
            username_challenge_id: id("username"),
            username_challenge_nonce: ChallengeNonce32::from_bytes([7; 32]),
            public_key: key,
            username: "Robin".to_owned(),
            signature: Signature64::from_bytes([0; 64]),
        };
        let signed_username = sign_username_update_with_key(username, &secret).unwrap();
        assert_eq!(signed_username.public_key, key);
        assert!(!signed_username.signature.is_zero());

        let deletion = DeletionRequestEnvelopeV1 {
            schema_version: 1,
            challenge: DeletionChallengeV1 {
                schema_version: 1,
                deletion_challenge_id: id("deletion"),
                deletion_challenge_nonce: ChallengeNonce32::from_bytes([8; 32]),
                expires_at_unix_ms: 1,
                public_key: key,
                target: DeletionTargetV1::Run { run_id: id("run") },
            },
            signature: Signature64::from_bytes([0; 64]),
        };
        let signed_deletion = sign_deletion_request_with_key(deletion, &secret).unwrap();
        assert_eq!(signed_deletion.challenge.public_key, key);
        assert!(!signed_deletion.signature.is_zero());
    }

    #[test]
    fn native_signing_rejects_another_identity() {
        let secret = SigningKey::from_bytes(&[0x42; 32]);
        let envelope = UsernameUpdateEnvelopeV1 {
            schema_version: 1,
            username_challenge_id: id("username"),
            username_challenge_nonce: ChallengeNonce32::from_bytes([7; 32]),
            public_key: PublicKey32::from_bytes([99; 32]),
            username: "Marian".to_owned(),
            signature: Signature64::from_bytes([0; 64]),
        };
        assert_eq!(
            sign_username_update_with_key(envelope, &secret),
            Err(LeaderboardSigningError::WrongIdentity)
        );
    }

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
        assert_eq!(signed.public_key, native_public_key(&secret));
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
