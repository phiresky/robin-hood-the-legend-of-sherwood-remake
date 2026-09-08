//! Typed operations in the isolated signer document; never linked into the game.
use crate::{LeaderboardSigningError, authorize_context, decode_json};
use robin_run_protocol::LeaderboardCoSignPurposeV1;
use robin_run_protocol::{
    CampaignContinuationAuthorizationClaimV1, CampaignContinuationAuthorizationV1,
    CampaignContinuationPreflightRequestClaimV1, CompetitionRunGrantRequestClaimV1,
    CompetitionRunGrantRequestV1, DeletionRequestEnvelopeV1, FreshRunPreflightRequestClaimV1,
    FreshRunPreflightRequestV1, LeaderboardCoSignRequestV1, ParticipantSignatureV1, PublicKey32,
    Signature64, SignatureAlgorithmV1, SubmissionEnvelopeV1, SubmissionOfferV1,
    SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1, UsernameUpdateEnvelopeV1,
    Validate,
};
use robin_run_protocol::{
    NamedSeatJoinAttestationV1, NamedSeatJoinClaimV1, ReplaySessionGenesisClaimV1,
    ReplaySessionGenesisV1,
};

const MAX_USERNAME_BYTES: usize = 4 * 1024;
const MAX_DELETION_BYTES: usize = 8 * 1024;
const MAX_OWNER_STATUS_BYTES: usize = 8 * 1024;
const MAX_RANKED_DOCUMENT_BYTES: usize = 128 * 1024;
const MAX_NAMED_SEAT_JOIN_BYTES: usize = 8 * 1024;
const MAX_REPLAY_SESSION_GENESIS_BYTES: usize = 64 * 1024;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserCampaignContinuationSigningInput {
    offer: SubmissionOfferV1,
    claim: CampaignContinuationAuthorizationClaimV1,
}

pub const LEADERBOARD_WEB_ORIGIN_ENV: &str = "ROBINHOOD_LEADERBOARD_WEB_ORIGIN";
pub const IDENTITY_SIGNER_ORIGIN_ENV: &str = "ROBINHOOD_IDENTITY_SIGNER_ORIGIN";

const DEFAULT_WEB_ORIGIN: &str = "https://robinhood.phiresky.xyz";
const DEFAULT_SIGNER_ORIGIN: &str = "https://identity.robinhood.phiresky.xyz";
const DEPLOYMENT_WEB_ORIGIN: Option<&str> = option_env!("ROBINHOOD_LEADERBOARD_WEB_ORIGIN");
const DEPLOYMENT_SIGNER_ORIGIN: Option<&str> = option_env!("ROBINHOOD_IDENTITY_SIGNER_ORIGIN");
fn canonical(
    result: Result<Vec<u8>, robin_run_protocol::CanonicalError>,
) -> Result<Vec<u8>, LeaderboardSigningError> {
    result.map_err(|error| LeaderboardSigningError::Canonical(error.to_string()))
}

fn canonical_document(
    error: robin_run_protocol::CanonicalDocumentError,
) -> LeaderboardSigningError {
    LeaderboardSigningError::Canonical(error.to_string())
}

fn invalid_claim(error: impl std::fmt::Display) -> LeaderboardSigningError {
    LeaderboardSigningError::InvalidClaim(error.to_string())
}

#[wasm_bindgen::prelude::wasm_bindgen(module = "/js/browser_identity_vault.js")]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(
        catch,
        js_name = robinhoodLeaderboardIdentityStatus
    )]
    async fn webcrypto_identity_status() -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;

    #[wasm_bindgen::prelude::wasm_bindgen(
        catch,
        js_name = robinhoodLeaderboardIdentityPublicKey
    )]
    async fn webcrypto_public_key() -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;

    #[wasm_bindgen::prelude::wasm_bindgen(
        catch,
        js_name = robinhoodLeaderboardIdentitySign
    )]
    async fn webcrypto_sign(
        operation: &str,
        message: &[u8],
    ) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
}
fn js_error(action: &str, value: wasm_bindgen::JsValue) -> LeaderboardSigningError {
    LeaderboardSigningError::Identity(format!(
        "{action}: {}",
        value.as_string().unwrap_or_else(|| format!("{value:?}"))
    ))
}

fn required_js_string(
    action: &str,
    value: wasm_bindgen::JsValue,
) -> Result<String, LeaderboardSigningError> {
    value.as_string().ok_or_else(|| {
        LeaderboardSigningError::Identity(format!("{action} returned a non-string value"))
    })
}

async fn secure_public_key() -> Result<PublicKey32, LeaderboardSigningError> {
    let value = webcrypto_public_key()
        .await
        .map_err(|error| js_error("obtain browser public key", error))?;
    required_js_string("obtain browser public key", value)?
        .parse()
        .map_err(|error| LeaderboardSigningError::Identity(format!("invalid public key: {error}")))
}

async fn secure_signature(
    operation: &str,
    bytes: &[u8],
) -> Result<Signature64, LeaderboardSigningError> {
    let value = webcrypto_sign(operation, bytes)
        .await
        .map_err(|error| js_error(operation, error))?;
    let bytes = js_sys::Uint8Array::new(&value).to_vec();
    let bytes: [u8; 64] = bytes.try_into().map_err(|bytes: Vec<u8>| {
        LeaderboardSigningError::Identity(format!(
            "browser signature has {} bytes; expected 64",
            bytes.len()
        ))
    })?;
    let signature = Signature64::from_bytes(bytes);
    if signature.is_zero() {
        return Err(LeaderboardSigningError::Identity(
            "browser identity returned an all-zero signature".to_owned(),
        ));
    }
    Ok(signature)
}

fn encode_json(value: &impl serde::Serialize) -> Result<String, wasm_bindgen::JsValue> {
    serde_json::to_string(value)
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}

fn wasm_error(error: LeaderboardSigningError) -> wasm_bindgen::JsValue {
    wasm_bindgen::JsValue::from_str(&error.to_string())
}

fn expected_web_origin() -> &'static str {
    DEPLOYMENT_WEB_ORIGIN.unwrap_or(DEFAULT_WEB_ORIGIN)
}

fn expected_signer_origin() -> &'static str {
    DEPLOYMENT_SIGNER_ORIGIN.unwrap_or(DEFAULT_SIGNER_ORIGIN)
}

fn authorize_browser_call(parent_origin: &str) -> Result<(), LeaderboardSigningError> {
    if parent_origin != expected_web_origin() {
        return Err(LeaderboardSigningError::OriginNotAuthorized);
    }
    let window = web_sys::window().ok_or(LeaderboardSigningError::SignerContext)?;
    let actual_origin = window
        .location()
        .origin()
        .map_err(|_| LeaderboardSigningError::SignerContext)?;
    let top = window
        .top()
        .map_err(|_| LeaderboardSigningError::SignerContext)?
        .ok_or(LeaderboardSigningError::SignerContext)?;
    authorize_context(
        parent_origin,
        expected_web_origin(),
        &actual_origin,
        expected_signer_origin(),
        !js_sys::Object::is(top.as_ref(), window.as_ref()),
    )
}

pub fn browser_authorize_parent(parent_origin: String) -> Result<(), wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)
}

pub async fn browser_identity_status(
    parent_origin: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let value = webcrypto_identity_status()
        .await
        .map_err(|error| wasm_error(js_error("read browser identity status", error)))?;
    required_js_string("read browser identity status", value).map_err(wasm_error)
}

pub async fn browser_public_key(parent_origin: String) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    secure_public_key()
        .await
        .map(|key| key.to_string())
        .map_err(wasm_error)
}

pub async fn browser_sign_username_update(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let mut envelope: UsernameUpdateEnvelopeV1 =
        decode_json(&json, MAX_USERNAME_BYTES).map_err(wasm_error)?;
    envelope
        .validate_signing_claim()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    if envelope.public_key != secure_public_key().await.map_err(wasm_error)? {
        return Err(wasm_error(LeaderboardSigningError::WrongIdentity));
    }
    envelope.signature = secure_signature(
        "username_update",
        &canonical(envelope.signing_bytes()).map_err(wasm_error)?,
    )
    .await
    .map_err(wasm_error)?;
    envelope
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    encode_json(&envelope)
}

pub async fn browser_sign_competition_run_grant_request(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let claim: CompetitionRunGrantRequestClaimV1 =
        decode_json(&json, MAX_RANKED_DOCUMENT_BYTES).map_err(wasm_error)?;
    claim
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    if claim.host_public_key != secure_public_key().await.map_err(wasm_error)? {
        return Err(wasm_error(LeaderboardSigningError::WrongIdentity));
    }
    let host_signature = secure_signature(
        "competition_run_grant_request",
        &canonical(claim.signing_bytes()).map_err(wasm_error)?,
    )
    .await
    .map_err(wasm_error)?;
    let signed = CompetitionRunGrantRequestV1 {
        claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
        host_signature,
    };
    signed
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    encode_json(&signed)
}

pub async fn browser_sign_fresh_run_preflight_request(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let claim: FreshRunPreflightRequestClaimV1 =
        decode_json(&json, MAX_RANKED_DOCUMENT_BYTES).map_err(wasm_error)?;
    claim
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    if claim.host_public_key != secure_public_key().await.map_err(wasm_error)? {
        return Err(wasm_error(LeaderboardSigningError::WrongIdentity));
    }
    let signed = FreshRunPreflightRequestV1 {
        host_signature: secure_signature(
            "fresh_run_preflight_request",
            &canonical(claim.signing_bytes()).map_err(wasm_error)?,
        )
        .await
        .map_err(wasm_error)?,
        claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
    };
    signed
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    encode_json(&signed)
}

async fn browser_sign_campaign_continuation_preflight(
    parent_origin: String,
    json: String,
    controller: bool,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let claim: CampaignContinuationPreflightRequestClaimV1 =
        decode_json(&json, MAX_RANKED_DOCUMENT_BYTES).map_err(wasm_error)?;
    claim
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    let public_key = secure_public_key().await.map_err(wasm_error)?;
    let (expected_key, operation, bytes) = if controller {
        (
            claim.campaign_controller_public_key,
            "campaign_continuation_preflight_controller",
            claim.controller_signing_bytes(),
        )
    } else {
        (
            claim.host_public_key,
            "campaign_continuation_preflight_host",
            claim.host_signing_bytes(),
        )
    };
    if expected_key != public_key {
        return Err(wasm_error(LeaderboardSigningError::WrongIdentity));
    }
    let signed = ParticipantSignatureV1 {
        public_key,
        signature: secure_signature(operation, &canonical(bytes).map_err(wasm_error)?)
            .await
            .map_err(wasm_error)?,
    };
    encode_json(&signed)
}

pub async fn browser_sign_campaign_continuation_preflight_as_host(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    browser_sign_campaign_continuation_preflight(parent_origin, json, false).await
}

pub async fn browser_sign_campaign_continuation_preflight_as_controller(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    browser_sign_campaign_continuation_preflight(parent_origin, json, true).await
}

pub async fn browser_sign_submission_claim(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let envelope: SubmissionEnvelopeV1 =
        decode_json(&json, MAX_RANKED_DOCUMENT_BYTES).map_err(wasm_error)?;
    envelope
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    let public_key = secure_public_key().await.map_err(wasm_error)?;
    if !envelope
        .offer
        .participant_claims
        .iter()
        .any(|claim| claim.public_key == public_key)
    {
        return Err(wasm_error(LeaderboardSigningError::IdentityNotClaimed));
    }
    let participant = ParticipantSignatureV1 {
        public_key,
        signature: secure_signature(
            "submission",
            &envelope
                .signing_bytes()
                .map_err(canonical_document)
                .map_err(wasm_error)?,
        )
        .await
        .map_err(wasm_error)?,
    };
    encode_json(&participant)
}

/// Isolated-browser counterpart to `sign_multiplayer_leaderboard_request`.
/// It accepts only the closed protocol request and signs its fixed payload.
pub async fn browser_sign_multiplayer_leaderboard_request(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let request: LeaderboardCoSignRequestV1 =
        decode_json(&json, MAX_RANKED_DOCUMENT_BYTES).map_err(wasm_error)?;
    let bytes = request
        .signing_bytes()
        .map_err(invalid_claim)
        .map_err(wasm_error)?;
    let operation = match request.instance.purpose {
        LeaderboardCoSignPurposeV1::CampaignContinuation => "multiplayer_campaign_continuation",
        LeaderboardCoSignPurposeV1::Submission => "multiplayer_submission",
    };
    let participant = ParticipantSignatureV1 {
        public_key: secure_public_key().await.map_err(wasm_error)?,
        signature: secure_signature(operation, &bytes)
            .await
            .map_err(wasm_error)?,
    };
    encode_json(&participant)
}

/// Sign one exact named-seat join claim in the isolated durable-identity
/// document. Rust owns canonicalization and the closed claim schema; neither
/// the game document nor TypeScript receives a generic signing primitive.
pub async fn browser_sign_named_seat_join(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let claim: NamedSeatJoinClaimV1 =
        decode_json(&json, MAX_NAMED_SEAT_JOIN_BYTES).map_err(wasm_error)?;
    claim
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    if claim.public_key != secure_public_key().await.map_err(wasm_error)? {
        return Err(wasm_error(LeaderboardSigningError::WrongIdentity));
    }
    let signed = NamedSeatJoinAttestationV1 {
        signature: secure_signature(
            "named_seat_join",
            &canonical(claim.signing_bytes()).map_err(wasm_error)?,
        )
        .await
        .map_err(wasm_error)?,
        claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
    };
    signed
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    encode_json(&signed)
}

/// Sign only the closed, validated host genesis claim. The isolated document
/// never accepts a caller-supplied domain or arbitrary message.
pub async fn browser_sign_replay_session_genesis(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let claim: ReplaySessionGenesisClaimV1 =
        decode_json(&json, MAX_REPLAY_SESSION_GENESIS_BYTES).map_err(wasm_error)?;
    claim
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    if claim.host_public_key != secure_public_key().await.map_err(wasm_error)? {
        return Err(wasm_error(LeaderboardSigningError::WrongIdentity));
    }
    let signed = ReplaySessionGenesisV1 {
        host_signature: secure_signature(
            "replay_session_genesis",
            &canonical(claim.signing_bytes()).map_err(wasm_error)?,
        )
        .await
        .map_err(wasm_error)?,
        claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
    };
    signed
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    encode_json(&signed)
}

pub async fn browser_sign_campaign_continuation(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let input: BrowserCampaignContinuationSigningInput =
        decode_json(&json, MAX_RANKED_DOCUMENT_BYTES).map_err(wasm_error)?;
    let request = input
        .claim
        .co_sign_request(&input.offer)
        .map_err(canonical_document)
        .map_err(wasm_error)?;
    if input.claim.campaign_controller_public_key
        != secure_public_key().await.map_err(wasm_error)?
    {
        return Err(wasm_error(LeaderboardSigningError::WrongIdentity));
    }
    let signature = secure_signature(
        "campaign_continuation",
        &request
            .signing_bytes()
            .map_err(invalid_claim)
            .map_err(wasm_error)?,
    )
    .await
    .map_err(wasm_error)?;
    let signed = CampaignContinuationAuthorizationV1 {
        claim: input.claim,
        algorithm: SignatureAlgorithmV1::Ed25519,
        signature,
    };
    signed
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    encode_json(&signed)
}

pub async fn browser_sign_submission_owner_status(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let challenge: SubmissionOwnerStatusChallengeV1 =
        decode_json(&json, MAX_OWNER_STATUS_BYTES).map_err(wasm_error)?;
    challenge
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    if challenge.controller_public_key != secure_public_key().await.map_err(wasm_error)? {
        return Err(wasm_error(LeaderboardSigningError::WrongIdentity));
    }
    let mut envelope = SubmissionOwnerStatusEnvelopeV1 {
        schema_version: challenge.schema_version,
        challenge,
        algorithm: SignatureAlgorithmV1::Ed25519,
        signature: Signature64::from_bytes([0; 64]),
    };
    envelope
        .validate_signing_claim()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    envelope.signature = secure_signature(
        "submission_owner_status",
        &canonical(envelope.signing_bytes()).map_err(wasm_error)?,
    )
    .await
    .map_err(wasm_error)?;
    envelope
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    encode_json(&envelope)
}

pub async fn browser_sign_deletion_request(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let mut envelope: DeletionRequestEnvelopeV1 =
        decode_json(&json, MAX_DELETION_BYTES).map_err(wasm_error)?;
    envelope
        .validate_signing_claim()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    if envelope.challenge.public_key != secure_public_key().await.map_err(wasm_error)? {
        return Err(wasm_error(LeaderboardSigningError::WrongIdentity));
    }
    envelope.signature = secure_signature(
        "deletion_request",
        &canonical(envelope.signing_bytes()).map_err(wasm_error)?,
    )
    .await
    .map_err(wasm_error)?;
    envelope
        .validate()
        .map_err(|error| wasm_error(invalid_claim(error)))?;
    encode_json(&envelope)
}
