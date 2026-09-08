use axum::Json;
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("signature authentication failed")]
    Unauthorized,
    #[error("resource not found")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("this exact upload is already in progress; retry after {retry_after_ms} ms")]
    UploadInProgress { retry_after_ms: u64 },
    #[error("{0}")]
    PayloadTooLarge(String),
    #[error("submission queue is temporarily full")]
    QueueFull,
    #[error("service temporarily unavailable")]
    Unavailable,
    #[error("internal server error")]
    Internal,
}

#[derive(Serialize)]
struct ErrorBody {
    schema_version: u32,
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message, retry_after_ms) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, "bad_request", message, None),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "signature_authentication_failed",
                self.to_string(),
                None,
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", self.to_string(), None),
            Self::Conflict(message) => (StatusCode::CONFLICT, "conflict", message, None),
            Self::UploadInProgress { retry_after_ms } => (
                StatusCode::CONFLICT,
                "upload_in_progress",
                format!(
                    "this exact upload is already in progress; retry after {retry_after_ms} ms"
                ),
                Some(retry_after_ms),
            ),
            Self::PayloadTooLarge(message) => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                message,
                None,
            ),
            Self::QueueFull => (
                StatusCode::TOO_MANY_REQUESTS,
                "submission_queue_full",
                self.to_string(),
                None,
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                self.to_string(),
                None,
            ),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_server_error",
                self.to_string(),
                None,
            ),
        };
        let mut response = (
            status,
            Json(ErrorBody {
                schema_version: crate::model::API_SCHEMA_VERSION,
                error: ErrorDetail { code, message },
            }),
        )
            .into_response();
        if let Some(retry_after_ms) = retry_after_ms {
            let retry_after_seconds = retry_after_ms.div_ceil(1_000).max(1);
            let value: HeaderValue = retry_after_seconds
                .to_string()
                .parse()
                .expect("a decimal u64 is always a valid Retry-After header");
            response.headers_mut().insert(RETRY_AFTER, value);
        }
        response
    }
}

impl From<crate::db::DbError> for ApiError {
    fn from(error: crate::db::DbError) -> Self {
        match error {
            crate::db::DbError::NotFound => Self::NotFound,
            crate::db::DbError::InvalidChallenge => {
                Self::Conflict("upload challenge is expired, consumed, or stale".to_owned())
            }
            crate::db::DbError::QueueFull => Self::QueueFull,
            crate::db::DbError::AdmissionUnavailable => Self::Unavailable,
            crate::db::DbError::SubmissionConflict => Self::Conflict(
                "submission challenge was already used for different immutable content".to_owned(),
            ),
            // Database invariants are downstream of HTTP validation and may
            // contain verifier-derived or operator-private detail. They are
            // never suitable client diagnostics.
            invariant @ crate::db::DbError::ResultInvariant(_) => {
                tracing::error!(
                    error_code = invariant.safe_log_code(),
                    "database request failed an internal invariant"
                );
                Self::Internal
            }
            other => {
                tracing::error!(
                    error_code = other.safe_log_code(),
                    "database request failed"
                );
                Self::Internal
            }
        }
    }
}

impl From<crate::replay_store::StoreError> for ApiError {
    fn from(error: crate::replay_store::StoreError) -> Self {
        match error {
            crate::replay_store::StoreError::Empty
            | crate::replay_store::StoreError::LengthMismatch { .. }
            | crate::replay_store::StoreError::DigestMismatch
            | crate::replay_store::StoreError::Upload(_) => Self::BadRequest(error.to_string()),
            crate::replay_store::StoreError::TooLarge { .. } => {
                Self::PayloadTooLarge(error.to_string())
            }
            crate::replay_store::StoreError::Io(_) => {
                tracing::error!(error_code = error.safe_log_code(), "replay storage failed");
                Self::Internal
            }
        }
    }
}

impl From<crate::campaign_store::CampaignStoreError> for ApiError {
    fn from(error: crate::campaign_store::CampaignStoreError) -> Self {
        match error {
            crate::campaign_store::CampaignStoreError::Empty
            | crate::campaign_store::CampaignStoreError::LengthMismatch { .. }
            | crate::campaign_store::CampaignStoreError::DigestMismatch
            | crate::campaign_store::CampaignStoreError::Upload(_) => {
                Self::BadRequest(error.to_string())
            }
            crate::campaign_store::CampaignStoreError::TooLarge { .. } => {
                Self::PayloadTooLarge(error.to_string())
            }
            crate::campaign_store::CampaignStoreError::Io(_) => {
                tracing::error!(
                    error_code = error.safe_log_code(),
                    "campaign storage failed"
                );
                Self::Internal
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt as _;

    #[tokio::test]
    async fn database_invariant_detail_is_private() {
        let sentinel = "/srv/private/fullgame/content.bundle";
        let response = ApiError::from(crate::db::DbError::ResultInvariant(sentinel.to_owned()))
            .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(!body.contains(sentinel), "private detail leaked: {body}");
        assert!(body.contains("internal server error"));
    }

    #[test]
    fn active_exact_upload_has_retryable_stable_response() {
        let response = ApiError::UploadInProgress {
            retry_after_ms: 2_001,
        }
        .into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(response.headers().get(RETRY_AFTER).unwrap(), "3");
    }
}
