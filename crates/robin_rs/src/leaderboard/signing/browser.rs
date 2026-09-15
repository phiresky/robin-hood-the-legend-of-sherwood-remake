//! Browser durable identity: every operation is delegated to the isolated
//! signer origin, and each response is size-bounded, decoded and re-verified
//! locally before it is returned.

use super::{
    GameIdentitySigner, LeaderboardSigningError, assemble_signed_submission, invalid_claim,
};
use robin_run_protocol::{
    PublicKey32, Signature64, SignedSubmissionV2, SubmissionOwnerStatusChallengeV1,
    SubmissionOwnerStatusEnvelopeV1, SubmissionV2, Validate,
};

const MAX_SIGNER_DOCUMENT_BYTES: usize = 128 * 1024;

const DEFAULT_SIGNER_ORIGIN: &str = "https://identity.robinhood.phiresky.xyz";
const DEPLOYMENT_SIGNER_ORIGIN: Option<&str> = option_env!("ROBINHOOD_IDENTITY_SIGNER_ORIGIN");

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

fn expected_signer_origin() -> &'static str {
    DEPLOYMENT_SIGNER_ORIGIN.unwrap_or(DEFAULT_SIGNER_ORIGIN)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserPublicKeyResult {
    kind: String,
    #[serde(rename = "publicKey")]
    public_key: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserSignedDocumentResult {
    kind: String,
    #[serde(rename = "documentJson")]
    document_json: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserParticipantSignatureResult {
    kind: String,
    #[serde(rename = "participantSignatureJson")]
    participant_signature_json: String,
}

/// Signature over one submission, as returned by the signer's
/// `sign_submission` operation.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserSubmissionSignature {
    public_key: PublicKey32,
    signature: Signature64,
}

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

fn decode_browser_result<T: serde::de::DeserializeOwned>(
    operation: &str,
    json: &str,
) -> Result<T, LeaderboardSigningError> {
    decode_json(json, MAX_SIGNER_DOCUMENT_BYTES).map_err(|error| {
        LeaderboardSigningError::Identity(format!("{operation} signer response: {error}"))
    })
}

fn encode_payload(value: &impl serde::Serialize) -> Result<String, LeaderboardSigningError> {
    serde_json::to_string(value)
        .map_err(|error| LeaderboardSigningError::InvalidJson(error.to_string()))
}

pub struct BrowserSigner;

impl GameIdentitySigner for BrowserSigner {
    async fn public_key() -> Result<PublicKey32, LeaderboardSigningError> {
        let response = request_browser_operation("public_key", None).await?;
        let result: BrowserPublicKeyResult = decode_browser_result("public_key", &response)?;
        if result.kind != "public_key" {
            return Err(LeaderboardSigningError::Identity(
                "public_key response has the wrong kind".to_owned(),
            ));
        }
        result.public_key.parse().map_err(|error| {
            LeaderboardSigningError::Identity(format!("invalid public key: {error}"))
        })
    }

    async fn sign_submission(
        submission: SubmissionV2,
    ) -> Result<SignedSubmissionV2, LeaderboardSigningError> {
        submission.validate().map_err(invalid_claim)?;
        const OPERATION: &str = "sign_submission";
        let response =
            request_browser_operation(OPERATION, Some(encode_payload(&submission)?)).await?;
        let result: BrowserParticipantSignatureResult =
            decode_browser_result(OPERATION, &response)?;
        if result.kind != "participant_signature" {
            return Err(LeaderboardSigningError::Identity(format!(
                "{OPERATION} response has the wrong kind"
            )));
        }
        let signed: BrowserSubmissionSignature =
            decode_browser_result(OPERATION, &result.participant_signature_json)?;
        assemble_signed_submission(submission, signed.public_key, signed.signature)
    }

    async fn sign_submission_owner_status(
        challenge: SubmissionOwnerStatusChallengeV1,
    ) -> Result<SubmissionOwnerStatusEnvelopeV1, LeaderboardSigningError> {
        const OPERATION: &str = "sign_submission_owner_status";
        let response =
            request_browser_operation(OPERATION, Some(encode_payload(&challenge)?)).await?;
        let result: BrowserSignedDocumentResult = decode_browser_result(OPERATION, &response)?;
        if result.kind != "signed_document" {
            return Err(LeaderboardSigningError::Identity(format!(
                "{OPERATION} response has the wrong kind"
            )));
        }
        let signed: SubmissionOwnerStatusEnvelopeV1 =
            decode_browser_result(OPERATION, &result.document_json)?;
        signed.validate().map_err(invalid_claim)?;
        if signed.challenge != challenge {
            return Err(LeaderboardSigningError::InvalidClaim(
                "signer changed the owner-status challenge".to_owned(),
            ));
        }
        Ok(signed)
    }
}
