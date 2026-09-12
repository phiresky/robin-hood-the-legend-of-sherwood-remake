//! Typed access to the verified leaderboard API.
//!
//! Wire documents remain owned by `robin_run_protocol`. This layer owns route
//! construction, bounded transport selection, response validation, and the
//! invariant that the one replay byte string is uploaded, authenticated,
//! decoded, retained, and downloaded unchanged.

use crate::leaderboard_http::{
    HttpRequest, HttpResponse, HttpTask, HttpTransportError, LeaderboardHttpClient,
};
use crate::leaderboard_preferences::{
    LeaderboardApiBaseUrl, LeaderboardPreferences, LeaderboardPreferencesError,
};
use robin_run_protocol::{
    AbuseReportAcceptedV1, AbuseReportV1, CampaignContentManifestV1,
    CampaignContinuationPreflightGrantV1, CampaignContinuationPreflightRequestV1,
    CampaignSessionDetailV1, ContentManifestV1, DeletionChallengeRequestV1, DeletionChallengeV1,
    DeletionReceiptV1, DeletionRequestEnvelopeV1, Digest32, FreshRunPreflightGrantV1,
    FreshRunPreflightRequestV1, LeaderboardMetadataV1, LeaderboardPageV1, LeaderboardQueryV1,
    OpaqueId, PlayerProfileV1, PublicKey32, PublishedRulesetV1, RANKED_CAMPAIGN_MEDIA_TYPE_V1,
    RANKED_REPLAY_MEDIA_TYPE_V1, ReplayArtifactV1, RulesConfigIdentityV1, SignedSubmissionV1,
    SubmissionAcceptedV1, SubmissionOfferRequestV1, SubmissionOfferV1,
    SubmissionOwnerStatusChallengeRequestV1, SubmissionOwnerStatusChallengeV1,
    SubmissionOwnerStatusEnvelopeV1, SubmissionOwnerStatusResponseV1, UsernameChallengeRequestV1,
    UsernameChallengeV1, UsernameUpdateEnvelopeV1, Validate, VersionedBuildManifest,
};
use serde::de::DeserializeOwned;
use std::sync::Arc;

pub const DEFAULT_BOARD_PAGE_LIMIT: u16 = 50;
pub const MAX_BOARD_PAGE_LIMIT: u16 = 100;

#[derive(Debug, thiserror::Error)]
pub enum LeaderboardServiceError {
    #[error(transparent)]
    Endpoint(#[from] LeaderboardPreferencesError),
    #[error(transparent)]
    Transport(#[from] HttpTransportError),
    #[error("leaderboard service returned HTTP {status}")]
    HttpStatus { status: u16 },
    #[error("leaderboard response is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("leaderboard response failed protocol validation: {0}")]
    InvalidProtocol(String),
    #[error("leaderboard response does not match the requested board")]
    BoardFilterMismatch,
    #[error("leaderboard response does not echo the exact requested page cursor")]
    BoardCursorMismatch,
    #[error("signed artifacts do not match the exact bytes selected for upload")]
    ArtifactMismatch,
    #[error("leaderboard artifact response has no Content-Type header")]
    MissingContentType,
    #[error("starting campaign is mandatory for every ranked run")]
    MissingStartingCampaign,
    #[error("replay is not the canonical current compact-bitcode artifact: {0}")]
    InvalidCompactReplay(String),
    #[error("leaderboard success response is not JSON (Content-Type was {found:?})")]
    UnexpectedContentType { found: Option<String> },
    #[error("failed to encode leaderboard request: {0}")]
    RequestEncoding(String),
}

#[derive(Debug, Clone)]
pub struct LeaderboardApi {
    base_url: LeaderboardApiBaseUrl,
    http: LeaderboardHttpClient,
}

impl LeaderboardApi {
    pub fn from_preferences(
        preferences: &LeaderboardPreferences,
    ) -> Result<Self, LeaderboardServiceError> {
        Ok(Self::new(preferences.effective_api_base_url()?))
    }

    pub fn new(base_url: LeaderboardApiBaseUrl) -> Self {
        Self {
            base_url,
            http: LeaderboardHttpClient::default(),
        }
    }

    pub fn with_http_client(base_url: LeaderboardApiBaseUrl, http: LeaderboardHttpClient) -> Self {
        Self { base_url, http }
    }

    pub fn metadata(&self) -> Result<HttpTask, LeaderboardServiceError> {
        self.spawn(HttpRequest::get_json(self.route("leaderboard-metadata")?))
    }

    pub fn content_manifest(&self, digest: Digest32) -> Result<HttpTask, LeaderboardServiceError> {
        self.immutable_document("content-manifests", digest)
    }

    pub fn campaign_content_manifest(
        &self,
        digest: Digest32,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.immutable_document("campaign-content-manifests", digest)
    }

    pub fn rules_config(&self, digest: Digest32) -> Result<HttpTask, LeaderboardServiceError> {
        self.immutable_document("rules-configs", digest)
    }

    pub fn published_ruleset(&self, digest: Digest32) -> Result<HttpTask, LeaderboardServiceError> {
        self.immutable_document("published-rulesets", digest)
    }

    pub fn build_manifest(&self, digest: Digest32) -> Result<HttpTask, LeaderboardServiceError> {
        self.immutable_document("builds", digest)
    }

    pub fn board(&self, query: &LeaderboardQueryV1) -> Result<HttpTask, LeaderboardServiceError> {
        query.validate().map_err(invalid_protocol)?;
        let query = serde_urlencoded::to_string(query)
            .map_err(|error| LeaderboardServiceError::RequestEncoding(error.to_string()))?;
        self.spawn(HttpRequest::get_json(format!(
            "{}?{query}",
            self.route("leaderboards")?
        )))
    }

    pub fn run_detail(&self, run_id: &OpaqueId) -> Result<HttpTask, LeaderboardServiceError> {
        self.spawn(HttpRequest::get_json(
            self.route(&format!("runs/{}", path_segment(run_id.as_str())))?,
        ))
    }

    pub fn run_replay(
        &self,
        run_id: &OpaqueId,
        expected: &ReplayArtifactV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        expected.validate().map_err(invalid_protocol)?;
        self.spawn(HttpRequest::get_replay(
            self.route(&format!("runs/{}/replay", path_segment(run_id.as_str())))?,
            expected.artifact.byte_length,
        )?)
    }

    pub fn campaign_session_detail(
        &self,
        aggregate_run_id: &OpaqueId,
        ordinal: u32,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.spawn(HttpRequest::get_json(self.route(&format!(
            "runs/{}/sessions/{ordinal}",
            path_segment(aggregate_run_id.as_str())
        ))?))
    }

    pub fn campaign_session_replay(
        &self,
        aggregate_run_id: &OpaqueId,
        ordinal: u32,
        expected: &ReplayArtifactV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        expected.validate().map_err(invalid_protocol)?;
        self.spawn(HttpRequest::get_replay(
            self.route(&format!(
                "runs/{}/sessions/{ordinal}/replay",
                path_segment(aggregate_run_id.as_str())
            ))?,
            expected.artifact.byte_length,
        )?)
    }

    pub fn submission_offer(
        &self,
        request: &SubmissionOfferRequestV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.post_json("submission-offers", request)
    }

    pub fn fresh_run_preflight_grant(
        &self,
        request: &FreshRunPreflightRequestV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.post_json("fresh-run-preflight-grants", request)
    }

    pub fn campaign_continuation_preflight_grant(
        &self,
        request: &CampaignContinuationPreflightRequestV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.post_json("campaign-continuation-preflight-grants", request)
    }

    pub fn submit(
        &self,
        submission: &SignedSubmissionV1,
        exact_replay_bytes: Arc<[u8]>,
        exact_starting_campaign_bytes: Arc<[u8]>,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        submission.validate().map_err(invalid_protocol)?;
        if exact_starting_campaign_bytes.is_empty() {
            return Err(LeaderboardServiceError::MissingStartingCampaign);
        }
        validate_canonical_replay_bytes(&exact_replay_bytes)?;
        let artifacts = &submission.submission.artifacts;
        if artifacts.replay.artifact.media_type != RANKED_REPLAY_MEDIA_TYPE_V1
            || u64::try_from(exact_replay_bytes.len()).ok()
                != Some(artifacts.replay.artifact.byte_length)
            || Digest32::digest_bytes(&exact_replay_bytes) != artifacts.replay.artifact.sha256
            || artifacts.starting_campaign.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1
            || u64::try_from(exact_starting_campaign_bytes.len()).ok()
                != Some(artifacts.starting_campaign.byte_length)
            || Digest32::digest_bytes(&exact_starting_campaign_bytes)
                != artifacts.starting_campaign.sha256
        {
            return Err(LeaderboardServiceError::ArtifactMismatch);
        }
        let submission_json: Arc<[u8]> = serde_json::to_vec(submission)
            .map_err(|error| LeaderboardServiceError::RequestEncoding(error.to_string()))?
            .into();
        self.spawn(HttpRequest::submission_multipart(
            self.route("submissions")?,
            submission_json,
            exact_replay_bytes,
            exact_starting_campaign_bytes,
        )?)
    }

    pub fn submission_owner_status_challenge(
        &self,
        request: &SubmissionOwnerStatusChallengeRequestV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.post_json("submission-owner-status-challenges", request)
    }

    pub fn submission_owner_status(
        &self,
        envelope: &SubmissionOwnerStatusEnvelopeV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        envelope.validate().map_err(invalid_protocol)?;
        self.post_json(
            &format!(
                "submissions/{}/private-status",
                path_segment(envelope.challenge.submission_id.as_str())
            ),
            envelope,
        )
    }

    pub fn player_profile(
        &self,
        public_key: PublicKey32,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.spawn(HttpRequest::get_json(
            self.route(&format!("players/{public_key}"))?,
        ))
    }

    pub fn username_challenge(
        &self,
        request: &UsernameChallengeRequestV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.post_json("username-challenges", request)
    }

    pub fn update_username(
        &self,
        request: &UsernameUpdateEnvelopeV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        request.validate().map_err(invalid_protocol)?;
        self.put_json(&format!("players/{}/username", request.public_key), request)
    }

    pub fn deletion_challenge(
        &self,
        request: &DeletionChallengeRequestV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.post_json("deletion-challenges", request)
    }

    pub fn delete_owned_run(
        &self,
        request: &DeletionRequestEnvelopeV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.post_json("deletion-requests", request)
    }

    pub fn report_abuse(
        &self,
        request: &AbuseReportV1,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.post_json("reports", request)
    }

    fn post_json<T: serde::Serialize + Validate>(
        &self,
        route: &str,
        request: &T,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.request_json(reqwest::Method::POST, route, request)
    }

    fn put_json<T: serde::Serialize + Validate>(
        &self,
        route: &str,
        request: &T,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.request_json(reqwest::Method::PUT, route, request)
    }

    fn immutable_document(
        &self,
        collection: &str,
        digest: Digest32,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        if digest.is_zero() {
            return Err(LeaderboardServiceError::InvalidProtocol(
                "immutable document route digest is zero".to_owned(),
            ));
        }
        self.spawn(HttpRequest::get_json(
            self.route(&format!("{collection}/{digest}"))?,
        ))
    }

    fn request_json<T: serde::Serialize + Validate>(
        &self,
        method: reqwest::Method,
        route: &str,
        request: &T,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        request.validate().map_err(invalid_protocol)?;
        let json = serde_json::to_vec(request)
            .map_err(|error| LeaderboardServiceError::RequestEncoding(error.to_string()))?;
        self.spawn(HttpRequest::json(method, self.route(route)?, json))
    }

    fn route(&self, suffix: &str) -> Result<String, LeaderboardServiceError> {
        Ok(self.base_url.route(suffix)?)
    }

    fn spawn(&self, request: HttpRequest) -> Result<HttpTask, LeaderboardServiceError> {
        Ok(self.http.spawn(request)?)
    }
}

fn path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

pub fn decode_metadata(
    result: Result<HttpResponse, HttpTransportError>,
) -> Result<LeaderboardMetadataV1, LeaderboardServiceError> {
    decode_validated_json(result)
}

pub fn decode_content_manifest(
    result: Result<HttpResponse, HttpTransportError>,
    expected: Digest32,
) -> Result<ContentManifestV1, LeaderboardServiceError> {
    decode_addressed_json(result, expected, "content manifest")
}

pub fn decode_campaign_content_manifest(
    result: Result<HttpResponse, HttpTransportError>,
    expected: Digest32,
) -> Result<CampaignContentManifestV1, LeaderboardServiceError> {
    decode_addressed_json(result, expected, "campaign content manifest")
}

pub fn decode_rules_config(
    result: Result<HttpResponse, HttpTransportError>,
    expected: Digest32,
) -> Result<RulesConfigIdentityV1, LeaderboardServiceError> {
    decode_addressed_json(result, expected, "rules config")
}

pub fn decode_published_ruleset(
    result: Result<HttpResponse, HttpTransportError>,
    expected: Digest32,
) -> Result<PublishedRulesetV1, LeaderboardServiceError> {
    let published: PublishedRulesetV1 = decode_validated_json(result)?;
    if published.ruleset_manifest_sha256 != expected
        || canonical_digest(&published.manifest)? != expected
    {
        return Err(address_mismatch("published ruleset"));
    }
    Ok(published)
}

pub fn decode_build_manifest(
    result: Result<HttpResponse, HttpTransportError>,
    expected: Digest32,
) -> Result<VersionedBuildManifest, LeaderboardServiceError> {
    decode_addressed_json(result, expected, "build manifest")
}

fn decode_addressed_json<T>(
    result: Result<HttpResponse, HttpTransportError>,
    expected: Digest32,
    label: &'static str,
) -> Result<T, LeaderboardServiceError>
where
    T: DeserializeOwned + serde::Serialize + Validate,
{
    let document: T = decode_validated_json(result)?;
    if canonical_digest(&document)? != expected {
        return Err(address_mismatch(label));
    }
    Ok(document)
}

fn canonical_digest(
    document: &(impl serde::Serialize + ?Sized),
) -> Result<Digest32, LeaderboardServiceError> {
    robin_run_protocol::canonical_json_bytes(document)
        .map(|bytes| Digest32::digest_bytes(&bytes))
        .map_err(|error| LeaderboardServiceError::InvalidProtocol(error.to_string()))
}

fn address_mismatch(label: &str) -> LeaderboardServiceError {
    LeaderboardServiceError::InvalidProtocol(format!(
        "{label} canonical digest does not match its immutable route"
    ))
}

pub fn decode_board(
    result: Result<HttpResponse, HttpTransportError>,
    requested_query: &LeaderboardQueryV1,
) -> Result<LeaderboardPageV1, LeaderboardServiceError> {
    let page: LeaderboardPageV1 = decode_validated_json(result)?;
    let requested_filter = requested_query.filter().map_err(invalid_protocol)?;
    if page.filter != requested_filter {
        return Err(LeaderboardServiceError::BoardFilterMismatch);
    }
    if requested_query.cursor.as_deref()
        != page
            .previous_cursor
            .as_ref()
            .map(|cursor| cursor.opaque_token.as_str())
    {
        return Err(LeaderboardServiceError::BoardCursorMismatch);
    }
    Ok(page)
}

pub fn decode_run_detail(
    result: Result<HttpResponse, HttpTransportError>,
    expected_run_id: &OpaqueId,
) -> Result<robin_run_protocol::RunDetailV1, LeaderboardServiceError> {
    let detail: robin_run_protocol::RunDetailV1 = decode_validated_json(result)?;
    if &detail.run_id != expected_run_id {
        return Err(LeaderboardServiceError::InvalidProtocol(
            "run detail does not match its route id".to_owned(),
        ));
    }
    Ok(detail)
}

pub fn decode_campaign_session_detail(
    result: Result<HttpResponse, HttpTransportError>,
    expected_aggregate_run_id: &OpaqueId,
    expected_ordinal: u32,
) -> Result<CampaignSessionDetailV1, LeaderboardServiceError> {
    let detail: CampaignSessionDetailV1 = decode_validated_json(result)?;
    if &detail.aggregate_run_id != expected_aggregate_run_id || detail.ordinal != expected_ordinal {
        return Err(LeaderboardServiceError::InvalidProtocol(
            "campaign session detail does not match its route".to_owned(),
        ));
    }
    Ok(detail)
}

#[derive(Clone)]
pub struct CanonicalReplayDownload {
    pub bytes: Arc<[u8]>,
    pub engine_hash: String,
}

impl std::fmt::Debug for CanonicalReplayDownload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanonicalReplayDownload")
            .field("byte_length", &self.bytes.len())
            .field("engine_hash", &self.engine_hash)
            .finish()
    }
}

pub fn decode_replay_download(
    result: Result<HttpResponse, HttpTransportError>,
    expected: &ReplayArtifactV1,
) -> Result<CanonicalReplayDownload, LeaderboardServiceError> {
    expected.validate().map_err(invalid_protocol)?;
    let response = result?;
    if !(200..300).contains(&response.status) {
        return Err(LeaderboardServiceError::HttpStatus {
            status: response.status,
        });
    }
    let media_type = response
        .content_type
        .as_deref()
        .map(base_media_type)
        .ok_or(LeaderboardServiceError::MissingContentType)?;
    if !media_type.eq_ignore_ascii_case(RANKED_REPLAY_MEDIA_TYPE_V1)
        || expected.artifact.media_type != RANKED_REPLAY_MEDIA_TYPE_V1
        || u64::try_from(response.body.len()).ok() != Some(expected.artifact.byte_length)
        || Digest32::digest_bytes(&response.body) != expected.artifact.sha256
    {
        return Err(LeaderboardServiceError::ArtifactMismatch);
    }
    let engine_hash = validate_canonical_replay_bytes(&response.body)?;
    Ok(CanonicalReplayDownload {
        bytes: response.body.into(),
        engine_hash,
    })
}

pub fn decode_offer(
    result: Result<HttpResponse, HttpTransportError>,
) -> Result<SubmissionOfferV1, LeaderboardServiceError> {
    decode_validated_json(result)
}

pub fn decode_fresh_run_preflight_grant(
    result: Result<HttpResponse, HttpTransportError>,
    request: &FreshRunPreflightRequestV1,
) -> Result<FreshRunPreflightGrantV1, LeaderboardServiceError> {
    let grant: FreshRunPreflightGrantV1 = decode_validated_json(result)?;
    grant.validate_request(request).map_err(invalid_protocol)?;
    Ok(grant)
}

pub fn decode_campaign_continuation_preflight_grant(
    result: Result<HttpResponse, HttpTransportError>,
    request: &CampaignContinuationPreflightRequestV1,
) -> Result<CampaignContinuationPreflightGrantV1, LeaderboardServiceError> {
    let grant: CampaignContinuationPreflightGrantV1 = decode_validated_json(result)?;
    grant.validate_request(request).map_err(invalid_protocol)?;
    Ok(grant)
}

pub fn decode_submission_accepted(
    result: Result<HttpResponse, HttpTransportError>,
) -> Result<SubmissionAcceptedV1, LeaderboardServiceError> {
    decode_validated_json_with_status(result, 202)
}

pub fn decode_submission_owner_status_challenge(
    result: Result<HttpResponse, HttpTransportError>,
    request: &SubmissionOwnerStatusChallengeRequestV1,
) -> Result<SubmissionOwnerStatusChallengeV1, LeaderboardServiceError> {
    let challenge: SubmissionOwnerStatusChallengeV1 = decode_validated_json(result)?;
    if challenge.controller_public_key != request.controller_public_key
        || challenge.submission_id != request.submission_id
    {
        return Err(LeaderboardServiceError::InvalidProtocol(
            "owner-status challenge does not match the request".to_owned(),
        ));
    }
    Ok(challenge)
}

pub fn decode_submission_owner_status(
    result: Result<HttpResponse, HttpTransportError>,
    envelope: &SubmissionOwnerStatusEnvelopeV1,
) -> Result<SubmissionOwnerStatusResponseV1, LeaderboardServiceError> {
    let response: SubmissionOwnerStatusResponseV1 = decode_validated_json(result)?;
    response
        .validate_against_envelope(envelope)
        .map_err(invalid_protocol)?;
    Ok(response)
}

pub fn decode_username_challenge(
    result: Result<HttpResponse, HttpTransportError>,
) -> Result<UsernameChallengeV1, LeaderboardServiceError> {
    decode_validated_json(result)
}

pub fn decode_player_profile(
    result: Result<HttpResponse, HttpTransportError>,
) -> Result<PlayerProfileV1, LeaderboardServiceError> {
    decode_validated_json(result)
}

pub fn decode_deletion_challenge(
    result: Result<HttpResponse, HttpTransportError>,
) -> Result<DeletionChallengeV1, LeaderboardServiceError> {
    decode_validated_json(result)
}

pub fn decode_deletion_receipt(
    result: Result<HttpResponse, HttpTransportError>,
) -> Result<DeletionReceiptV1, LeaderboardServiceError> {
    decode_validated_json(result)
}

pub fn decode_abuse_report_accepted(
    result: Result<HttpResponse, HttpTransportError>,
) -> Result<AbuseReportAcceptedV1, LeaderboardServiceError> {
    decode_validated_json_with_status(result, 202)
}

fn validate_canonical_replay_bytes(bytes: &[u8]) -> Result<String, LeaderboardServiceError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| LeaderboardServiceError::InvalidCompactReplay("not UTF-8".to_owned()))?;
    let limits = robin_replay_format::ReplayAdmissionLimits {
        max_input_bytes: bytes.len(),
        ..Default::default()
    };
    let (engine_hash, replay) = robin_replay_format::decode_compact_bounded(text, &limits)
        .map_err(|error| LeaderboardServiceError::InvalidCompactReplay(error.to_string()))?;
    robin_replay_format::validate_engine_hash(&engine_hash)
        .map_err(|error| LeaderboardServiceError::InvalidCompactReplay(error.to_string()))?;
    let canonical = robin_replay_format::encode_compact(&replay, &engine_hash)
        .map_err(|error| LeaderboardServiceError::InvalidCompactReplay(error.to_string()))?;
    if canonical.as_bytes() != bytes {
        return Err(LeaderboardServiceError::InvalidCompactReplay(
            "bytes do not equal their canonical re-encoding".to_owned(),
        ));
    }
    Ok(engine_hash)
}

fn decode_validated_json<T>(
    result: Result<HttpResponse, HttpTransportError>,
) -> Result<T, LeaderboardServiceError>
where
    T: DeserializeOwned + Validate,
{
    let response = result?;
    if !(200..300).contains(&response.status) {
        return Err(LeaderboardServiceError::HttpStatus {
            status: response.status,
        });
    }
    decode_validated_success_body(response)
}

fn decode_validated_json_with_status<T>(
    result: Result<HttpResponse, HttpTransportError>,
    expected_status: u16,
) -> Result<T, LeaderboardServiceError>
where
    T: DeserializeOwned + Validate,
{
    let response = result?;
    if response.status != expected_status {
        return Err(LeaderboardServiceError::HttpStatus {
            status: response.status,
        });
    }
    decode_validated_success_body(response)
}

fn decode_validated_success_body<T>(response: HttpResponse) -> Result<T, LeaderboardServiceError>
where
    T: DeserializeOwned + Validate,
{
    if !response
        .content_type
        .as_deref()
        .is_some_and(is_json_content_type)
    {
        return Err(LeaderboardServiceError::UnexpectedContentType {
            found: response.content_type,
        });
    }
    let document: T = serde_json::from_slice(&response.body)
        .map_err(|error| LeaderboardServiceError::InvalidJson(error.to_string()))?;
    document.validate().map_err(invalid_protocol)?;
    Ok(document)
}

fn is_json_content_type(content_type: &str) -> bool {
    let media_type = base_media_type(content_type).to_ascii_lowercase();
    media_type == "application/json" || media_type.ends_with("+json")
}

fn base_media_type(content_type: &str) -> &str {
    content_type.split(';').next().unwrap_or_default().trim()
}

fn invalid_protocol(error: impl std::fmt::Display) -> LeaderboardServiceError {
    LeaderboardServiceError::InvalidProtocol(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::replay::{ReplayData, ReplayFile, ReplayHeader};
    use std::collections::BTreeMap;

    fn response(status: u16, media_type: &str, body: Vec<u8>) -> HttpResponse {
        HttpResponse {
            status,
            content_type: Some(media_type.to_owned()),
            body,
        }
    }

    fn compact_fixture() -> Vec<u8> {
        let replay = ReplayData::try_from(ReplayFile {
            header: ReplayHeader {
                mission_id: "m01s01".to_owned(),
                mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    "m01s01", "m01s01", "m01s01",
                )
                .expect("valid built-in leaderboard-service test descriptor"),
                rng_seed: 7,
                sim_config: robin_engine::engine::SimConfig::default(),
                spellforge_package: None,
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 0,
                rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
                campaign: bitcode::encode(&robin_engine::campaign::Campaign::default()),
            },
            frames: BTreeMap::new(),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        });
        let replay = replay.expect("valid service replay fixture");
        robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
            .unwrap()
            .into_bytes()
    }

    #[test]
    fn replay_download_requires_mime_length_digest_and_canonical_decode() {
        let bytes = compact_fixture();
        let expected = ReplayArtifactV1 {
            artifact: robin_run_protocol::ArtifactRefV1 {
                sha256: Digest32::digest_bytes(&bytes),
                byte_length: bytes.len() as u64,
                media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
            },
            replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        };
        let decoded = decode_replay_download(
            Ok(response(200, RANKED_REPLAY_MEDIA_TYPE_V1, bytes.clone())),
            &expected,
        )
        .unwrap();
        assert_eq!(decoded.bytes.as_ref(), bytes);

        assert!(matches!(
            decode_replay_download(
                Ok(response(200, "application/json", compact_fixture())),
                &expected
            ),
            Err(LeaderboardServiceError::ArtifactMismatch)
        ));
        let mut corrupt = compact_fixture();
        corrupt.push(b' ');
        let corrupt_expected = ReplayArtifactV1 {
            artifact: robin_run_protocol::ArtifactRefV1 {
                sha256: Digest32::digest_bytes(&corrupt),
                byte_length: corrupt.len() as u64,
                media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
            },
            replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        };
        assert!(matches!(
            decode_replay_download(
                Ok(response(200, RANKED_REPLAY_MEDIA_TYPE_V1, corrupt)),
                &corrupt_expected
            ),
            Err(LeaderboardServiceError::InvalidCompactReplay(_))
        ));
    }

    #[test]
    fn successful_json_requires_json_content_type() {
        let response = response(200, "text/html", b"{}".to_vec());
        assert!(matches!(
            decode_metadata(Ok(response)),
            Err(LeaderboardServiceError::UnexpectedContentType { .. })
        ));
    }

    #[test]
    fn route_ids_are_path_encoded() {
        assert_eq!(path_segment("safe-id"), "safe-id");
        assert_eq!(path_segment("bad/id"), "bad%2Fid");
    }
}
