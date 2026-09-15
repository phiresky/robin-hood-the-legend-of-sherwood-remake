//! Typed access to the verified leaderboard API.
//!
//! Wire documents remain owned by `robin_run_protocol`. This layer owns route
//! construction, bounded transport selection, response validation, and the
//! invariant that the one replay byte string is uploaded, signed, verified,
//! retained, and downloaded unchanged.

use crate::leaderboard_http::{
    HttpRequest, HttpResponse, HttpTask, HttpTransportError, LeaderboardHttpClient,
};
use crate::leaderboard_preferences::{
    LeaderboardApiBaseUrl, LeaderboardPreferences, LeaderboardPreferencesError,
};
#[cfg(test)]
use robin_run_protocol::ReplayArtifactV1;
use robin_run_protocol::{
    Digest32, LeaderboardMetadataV2, LeaderboardPageV2, LeaderboardQueryV2,
    RANKED_REPLAY_MEDIA_TYPE_V1, SignedSubmissionOwnerStatusRequestV2, SignedSubmissionV3,
    SubmissionAcceptedV1, SubmissionOwnerStatusResponseV2, Validate,
};
use serde::de::DeserializeOwned;
use std::sync::Arc;

pub const DEFAULT_BOARD_PAGE_LIMIT: u16 = 50;

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
    #[error("signed replay artifact does not match the exact bytes selected for upload")]
    ArtifactMismatch,
    #[error("leaderboard artifact response has no Content-Type header")]
    #[cfg(test)]
    MissingContentType,
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

    pub fn metadata(&self) -> Result<HttpTask, LeaderboardServiceError> {
        self.spawn(HttpRequest::get_json(self.route("leaderboard-metadata")?))
    }

    pub fn board(&self, query: &LeaderboardQueryV2) -> Result<HttpTask, LeaderboardServiceError> {
        query.validate().map_err(invalid_protocol)?;
        let query = serde_urlencoded::to_string(query)
            .map_err(|error| LeaderboardServiceError::RequestEncoding(error.to_string()))?;
        self.spawn(HttpRequest::get_json(format!(
            "{}?{query}",
            self.route("leaderboards")?
        )))
    }

    /// Upload one signed submission together with the exact replay bytes its
    /// artifact reference names. The submission must be freshly signed: the
    /// server only accepts recent `signed_at_unix_ms` values.
    pub fn submit(
        &self,
        submission: &SignedSubmissionV3,
        exact_replay_bytes: Arc<[u8]>,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        submission.validate().map_err(invalid_protocol)?;
        validate_canonical_replay_bytes(&exact_replay_bytes)?;
        let artifact = &submission.request.replay.artifact;
        if artifact.media_type != RANKED_REPLAY_MEDIA_TYPE_V1
            || u64::try_from(exact_replay_bytes.len()).ok() != Some(artifact.byte_length)
            || Digest32::digest_bytes(&exact_replay_bytes) != artifact.sha256
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
        )?)
    }

    /// Read one submission's private lifecycle with a freshly signed request.
    pub fn submission_owner_status(
        &self,
        request: &SignedSubmissionOwnerStatusRequestV2,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        self.post_json(
            &format!(
                "submissions/{}/private-status",
                path_segment(request.request.submission_id.as_str())
            ),
            request,
        )
    }

    fn post_json<T: serde::Serialize + Validate>(
        &self,
        route: &str,
        request: &T,
    ) -> Result<HttpTask, LeaderboardServiceError> {
        request.validate().map_err(invalid_protocol)?;
        let json = serde_json::to_vec(request)
            .map_err(|error| LeaderboardServiceError::RequestEncoding(error.to_string()))?;
        self.spawn(HttpRequest::json(
            reqwest::Method::POST,
            self.route(route)?,
            json,
        ))
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
) -> Result<LeaderboardMetadataV2, LeaderboardServiceError> {
    decode_validated_json(result)
}

pub fn decode_board(
    result: Result<HttpResponse, HttpTransportError>,
    requested_query: &LeaderboardQueryV2,
) -> Result<LeaderboardPageV2, LeaderboardServiceError> {
    let page: LeaderboardPageV2 = decode_validated_json(result)?;
    if page.filter != requested_query.filter() {
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

#[cfg(test)]
#[derive(Clone)]
pub struct CanonicalReplayDownload {
    pub bytes: Arc<[u8]>,
    pub engine_hash: String,
}

#[cfg(test)]
impl std::fmt::Debug for CanonicalReplayDownload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanonicalReplayDownload")
            .field("byte_length", &self.bytes.len())
            .field("engine_hash", &self.engine_hash)
            .finish()
    }
}

// Preserve the exact artifact-boundary regression harness independently of
// the retired, unwired replay-download UI flow.
#[cfg(test)]
fn decode_replay_download(
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

pub fn decode_submission_accepted(
    result: Result<HttpResponse, HttpTransportError>,
) -> Result<SubmissionAcceptedV1, LeaderboardServiceError> {
    decode_validated_json_with_status(result, 202)
}

pub fn decode_submission_owner_status(
    result: Result<HttpResponse, HttpTransportError>,
    request: &SignedSubmissionOwnerStatusRequestV2,
) -> Result<SubmissionOwnerStatusResponseV2, LeaderboardServiceError> {
    let response: SubmissionOwnerStatusResponseV2 = decode_validated_json(result)?;
    response
        .validate_against_request(request)
        .map_err(invalid_protocol)?;
    Ok(response)
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
    use crate::leaderboard::test_fixtures::{MISSION_ID, single_frame_replay};

    fn response(status: u16, media_type: &str, body: Vec<u8>) -> HttpResponse {
        HttpResponse {
            status,
            content_type: Some(media_type.to_owned()),
            body,
        }
    }

    fn compact_fixture() -> Vec<u8> {
        let replay =
            single_frame_replay(bitcode::encode(&robin_engine::campaign::Campaign::default()));
        robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
            .unwrap()
            .into_bytes()
    }

    fn artifact(bytes: &[u8]) -> ReplayArtifactV1 {
        ReplayArtifactV1 {
            artifact: robin_run_protocol::ArtifactRefV1 {
                sha256: Digest32::digest_bytes(bytes),
                byte_length: bytes.len() as u64,
                media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
            },
            replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        }
    }

    #[test]
    fn replay_download_requires_mime_length_digest_and_canonical_decode() {
        let bytes = compact_fixture();
        let expected = artifact(&bytes);
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
        let corrupt_expected = artifact(&corrupt);
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
    fn board_pages_must_echo_the_exact_query_filter_and_cursor() {
        let query = LeaderboardQueryV2 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V2,
            board_id: robin_run_protocol::OpaqueId::new("demo-standard-normal").unwrap(),
            mission_id: MISSION_ID.to_owned(),
            metric: robin_run_protocol::BoardMetricV1::OriginalScore,
            max_concurrent_players: Some(1),
            player_public_key: None,
            limit: DEFAULT_BOARD_PAGE_LIMIT,
            cursor: None,
        };
        let page = LeaderboardPageV2 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V2,
            filter: query.filter(),
            accepted_sequence_watermark: 0,
            previous_cursor: None,
            entries: Vec::new(),
            next_cursor: None,
        };
        let json = |page: &LeaderboardPageV2| {
            Ok(response(
                200,
                "application/json",
                serde_json::to_vec(page).unwrap(),
            ))
        };
        assert_eq!(decode_board(json(&page), &query).unwrap(), page);
        let mut other = page.clone();
        other.filter.mission_id = "Demo_Lin".to_owned();
        assert!(matches!(
            decode_board(json(&other), &query),
            Err(LeaderboardServiceError::BoardFilterMismatch)
        ));
        let mut paged = query.clone();
        paged.cursor = Some("token".to_owned());
        assert!(matches!(
            decode_board(json(&page), &paged),
            Err(LeaderboardServiceError::BoardCursorMismatch)
        ));
    }

    #[test]
    fn route_ids_are_path_encoded() {
        assert_eq!(path_segment("safe-id"), "safe-id");
        assert_eq!(path_segment("bad/id"), "bad%2Fid");
    }
}
