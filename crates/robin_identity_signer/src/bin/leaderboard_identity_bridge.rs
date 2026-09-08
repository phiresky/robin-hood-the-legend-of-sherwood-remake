//! No-start wasm-bindgen bridge for the isolated identity-signer document.

#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodAuthorizeLeaderboardIdentityParent)]
pub fn authorize_parent(parent_origin: String) -> Result<(), wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_authorize_parent(parent_origin)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodLeaderboardIdentityStatus)]
pub async fn status(parent_origin: String) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_identity_status(parent_origin).await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodLeaderboardPublicKey)]
pub async fn public_key(parent_origin: String) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_public_key(parent_origin).await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodSignUsernameUpdate)]
pub async fn sign_username_update(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_username_update(parent_origin, json).await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodSignCompetitionRunGrantRequest)]
pub async fn sign_competition_run_grant_request(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_competition_run_grant_request(parent_origin, json)
        .await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodSignFreshRunPreflightRequest)]
pub async fn sign_fresh_run_preflight_request(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_fresh_run_preflight_request(parent_origin, json)
        .await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(
    js_name = robinhoodSignCampaignContinuationPreflightAsHost
)]
pub async fn sign_campaign_continuation_preflight_as_host(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_campaign_continuation_preflight_as_host(
        parent_origin,
        json,
    )
    .await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(
    js_name = robinhoodSignCampaignContinuationPreflightAsController
)]
pub async fn sign_campaign_continuation_preflight_as_controller(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_campaign_continuation_preflight_as_controller(
        parent_origin,
        json,
    )
    .await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodSignSubmissionClaim)]
pub async fn sign_submission_claim(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_submission_claim(parent_origin, json).await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(
    js_name = robinhoodSignMultiplayerLeaderboardRequest
)]
pub async fn sign_multiplayer_leaderboard_request(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_multiplayer_leaderboard_request(
        parent_origin,
        json,
    )
    .await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodSignNamedSeatJoin)]
pub async fn sign_named_seat_join(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_named_seat_join(parent_origin, json).await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodSignReplaySessionGenesis)]
pub async fn sign_replay_session_genesis(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_replay_session_genesis(parent_origin, json).await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodSignCampaignContinuation)]
pub async fn sign_campaign_continuation(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_campaign_continuation(parent_origin, json).await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodSignSubmissionOwnerStatus)]
pub async fn sign_submission_owner_status(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_submission_owner_status(parent_origin, json).await
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = robinhoodSignDeletionRequest)]
pub async fn sign_deletion_request(
    parent_origin: String,
    json: String,
) -> Result<String, wasm_bindgen::JsValue> {
    robin_identity_signer::browser::browser_sign_deletion_request(parent_origin, json).await
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
