//! Browser durable identity: every operation is delegated to the isolated
//! signer origin, and each response is size-bounded, decoded and re-verified
//! locally before it is returned.

use super::{
    GameIdentitySigner, LeaderboardSigningError, canonical, invalid_claim, verify_typed_signature,
};
use crate::leaderboard_ranked_session::{OfficialRankedSessionSetupV1, RankedSessionHost};
use robin_run_protocol::{
    CampaignContinuationAuthorizationClaimV1, CampaignContinuationAuthorizationV1,
    CampaignContinuationPreflightRequestClaimV1, FreshRunPreflightRequestClaimV1,
    FreshRunPreflightRequestV1, LeaderboardCoSignRequestV1, ParticipantSignatureV1, PublicKey32,
    ReplaySessionGenesisClaimV1, ReplaySessionGenesisV1, SubmissionEnvelopeV1, SubmissionOfferV1,
    SubmissionOwnerStatusChallengeV1, SubmissionOwnerStatusEnvelopeV1, Validate,
};
use robin_run_protocol::DomainSignedClaim as _;
#[cfg(feature = "multiplayer")]
use robin_run_protocol::{NamedSeatJoinAttestationV1, NamedSeatJoinClaimV1};

const MAX_RANKED_DOCUMENT_BYTES: usize = 128 * 1024;

const DEFAULT_SIGNER_ORIGIN: &str = "https://identity.robinhood.phiresky.xyz";
const DEPLOYMENT_SIGNER_ORIGIN: Option<&str> = option_env!("ROBINHOOD_IDENTITY_SIGNER_ORIGIN");

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserCampaignContinuationSigningInput {
    offer: SubmissionOfferV1,
    claim: CampaignContinuationAuthorizationClaimV1,
}

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
    decode_json(json, MAX_RANKED_DOCUMENT_BYTES).map_err(|error| {
        LeaderboardSigningError::Identity(format!("{operation} signer response: {error}"))
    })
}

async fn browser_public_key() -> Result<PublicKey32, LeaderboardSigningError> {
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

async fn sign_campaign_continuation_preflight(
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

async fn sign_replay_session_genesis(
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
    if signed.claim.host_public_key != browser_public_key().await? {
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

/// Decode a signer `participant_signature` response and require that it is a
/// non-zero signature by this browser's durable identity.
async fn own_participant_signature(
    operation: &str,
    payload: String,
) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
    let response = request_browser_operation(operation, Some(payload)).await?;
    let result: BrowserParticipantSignatureResult = decode_browser_result(operation, &response)?;
    if result.kind != "participant_signature" {
        return Err(LeaderboardSigningError::Identity(format!(
            "{operation} response has the wrong kind"
        )));
    }
    let participant: ParticipantSignatureV1 =
        decode_browser_result(operation, &result.participant_signature_json)?;
    if participant.public_key != browser_public_key().await? || participant.signature.is_zero() {
        return Err(LeaderboardSigningError::WrongIdentity);
    }
    Ok(participant)
}

pub struct BrowserSigner;

impl GameIdentitySigner for BrowserSigner {
    async fn public_key() -> Result<PublicKey32, LeaderboardSigningError> {
        browser_public_key().await
    }

    /// Create a browser-hosted official ranked session through the one closed
    /// genesis operation. Bootstrap code never sees raw key material and
    /// cannot replace the claim between construction, signing, and
    /// verification.
    async fn create_official_ranked_session(
        network_protocol_version: u32,
        setup: OfficialRankedSessionSetupV1,
    ) -> Result<RankedSessionHost, LeaderboardSigningError> {
        let claim = RankedSessionHost::prepare_official_genesis_claim(
            browser_public_key().await?,
            network_protocol_version,
            setup.clone(),
        )
        .map_err(invalid_claim)?;
        let genesis = sign_replay_session_genesis(&claim).await?;
        crate::leaderboard_ranked_session::validate_official_session_genesis(
            &genesis,
            *claim.host_public_key.as_bytes(),
            &setup,
        )
        .map_err(invalid_claim)?;
        RankedSessionHost::from_signed_genesis(genesis).map_err(invalid_claim)
    }

    async fn sign_submission_claim(
        envelope: &SubmissionEnvelopeV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
        let payload = serde_json::to_string(envelope)
            .map_err(|error| LeaderboardSigningError::InvalidJson(error.to_string()))?;
        own_participant_signature("sign_submission", payload).await
    }

    async fn sign_multiplayer_leaderboard_request(
        request: &LeaderboardCoSignRequestV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
        request.validate().map_err(invalid_claim)?;
        let payload = serde_json::to_string(request)
            .map_err(|error| LeaderboardSigningError::InvalidJson(error.to_string()))?;
        own_participant_signature("sign_multiplayer_leaderboard_request", payload).await
    }

    async fn sign_fresh_run_preflight_request(
        claim: FreshRunPreflightRequestClaimV1,
    ) -> Result<FreshRunPreflightRequestV1, LeaderboardSigningError> {
        let signed = browser_signed_document("sign_fresh_run_preflight_request", &claim).await?;
        let signed: FreshRunPreflightRequestV1 = signed;
        if signed.claim != claim {
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

    async fn sign_campaign_continuation_preflight_as_host(
        claim: &CampaignContinuationPreflightRequestClaimV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
        sign_campaign_continuation_preflight(
            "sign_campaign_continuation_preflight_as_host",
            claim,
            false,
        )
        .await
    }

    async fn sign_campaign_continuation_preflight_as_controller(
        claim: &CampaignContinuationPreflightRequestClaimV1,
    ) -> Result<ParticipantSignatureV1, LeaderboardSigningError> {
        sign_campaign_continuation_preflight(
            "sign_campaign_continuation_preflight_as_controller",
            claim,
            true,
        )
        .await
    }

    async fn sign_campaign_continuation(
        offer: &SubmissionOfferV1,
        claim: CampaignContinuationAuthorizationClaimV1,
    ) -> Result<CampaignContinuationAuthorizationV1, LeaderboardSigningError> {
        let input = BrowserCampaignContinuationSigningInput {
            offer: offer.clone(),
            claim: claim.clone(),
        };
        let signed = browser_signed_document("sign_campaign_continuation", &input).await?;
        let signed: CampaignContinuationAuthorizationV1 = signed;
        if signed.claim != claim {
            return Err(LeaderboardSigningError::InvalidClaim(
                "signer changed the campaign continuation claim".to_owned(),
            ));
        }
        Ok(signed)
    }

    async fn sign_submission_owner_status(
        challenge: SubmissionOwnerStatusChallengeV1,
    ) -> Result<SubmissionOwnerStatusEnvelopeV1, LeaderboardSigningError> {
        let signed = browser_signed_document("sign_submission_owner_status", &challenge).await?;
        let signed: SubmissionOwnerStatusEnvelopeV1 = signed;
        if signed.challenge != challenge {
            return Err(LeaderboardSigningError::InvalidClaim(
                "signer changed the owner-status challenge".to_owned(),
            ));
        }
        Ok(signed)
    }
}

// Native seats sign named-seat joins with their transport-bound key in
// `leaderboard_ranked_session::sign_named_seat_join`; only the browser relay
// client needs the isolated signer for this claim.
#[cfg(feature = "multiplayer")]
impl BrowserSigner {
    pub async fn sign_named_seat_join(
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
        if signed.claim.public_key != browser_public_key().await? {
            return Err(LeaderboardSigningError::WrongIdentity);
        }
        verify_typed_signature(
            signed.claim.public_key,
            &canonical(signed.claim.signing_bytes())?,
            signed.signature,
        )?;
        Ok(signed)
    }
}
