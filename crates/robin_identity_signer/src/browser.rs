//! Typed operations in the isolated signer document; never linked into the game.
use crate::{LeaderboardSigningError, authorize_context, decode_json};
use robin_run_protocol::{
    DeletionRequestEnvelopeV1, PublicKey32, Signature64, SignatureAlgorithmV1, SignedSubmissionV2,
    SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1, SubmissionV2,
    UsernameUpdateEnvelopeV1, Validate,
};

const MAX_USERNAME_BYTES: usize = 4 * 1024;
const MAX_DELETION_BYTES: usize = 8 * 1024;
const MAX_OWNER_STATUS_BYTES: usize = 8 * 1024;
const MAX_SUBMISSION_BYTES: usize = 128 * 1024;

/// Uploader signature over `SignedSubmissionV2::signing_bytes`. The JSON
/// shape is `{ "public_key": hex, "signature": hex }`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmissionSignature {
    public_key: PublicKey32,
    signature: Signature64,
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

/// Sign one exact `SubmissionV2` as its uploader. Returns
/// `{ "public_key", "signature" }` over `SignedSubmissionV2::signing_bytes`.
pub async fn browser_sign_submission_claim(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    authorize_browser_call(&parent_origin).map_err(wasm_error)?;
    let submission: SubmissionV2 = decode_json(&json, MAX_SUBMISSION_BYTES).map_err(wasm_error)?;
    let bytes = SignedSubmissionV2::signing_bytes(&submission)
        .map_err(canonical_document)
        .map_err(wasm_error)?;
    let public_key = secure_public_key().await.map_err(wasm_error)?;
    if submission.uploader_public_key != public_key {
        return Err(wasm_error(LeaderboardSigningError::WrongIdentity));
    }
    let signed = SubmissionSignature {
        public_key,
        signature: secure_signature("submission", &bytes)
            .await
            .map_err(wasm_error)?,
    };
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
