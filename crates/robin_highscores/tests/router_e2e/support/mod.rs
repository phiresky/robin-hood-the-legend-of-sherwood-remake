//! Shared router end-to-end rig and request builders.

pub(crate) use axum::Router;
pub(crate) use axum::body::Body;
pub(crate) use axum::extract::ConnectInfo;
pub(crate) use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
pub(crate) use axum::http::{Method, Request, StatusCode};
pub(crate) use ed25519_dalek::{Signer as _, SigningKey};
pub(crate) use http_body_util::BodyExt as _;
pub(crate) use robin_highscores::web::{AppState, RateLimiter, router};
pub(crate) use robin_highscores::{Database, ReplayStore, ServerConfig};
pub(crate) use robin_run_protocol::{
    AbuseReportAcceptedV1, AbuseReportCategoryV1, AbuseReportTargetV1, AbuseReportV1,
    ArtifactRefV1, BoardMetricV1, BoardMetricValueV2, DeletionReceiptV1, DeletionRequestV2,
    DeletionTargetV1, Digest32, LeaderboardMetadataV2, LeaderboardPageV2, OpaqueId,
    ParticipantPublicDisclosureV1, PlayerProfileV1, PlayerRunHistoryPageV2, PublicKey32,
    PublicSubmissionStateV1, PublicSubmissionStatusV1, RANKED_REPLAY_MEDIA_TYPE_V1,
    ReplayArtifactV1, RunDetailV2, SCHEMA_VERSION_V1, SCHEMA_VERSION_V2, SCHEMA_VERSION_V3,
    Signature64, SignatureAlgorithmV1, SignedDeletionRequestV2, SignedRequestClaim,
    SignedRequestV2, SignedSubmissionOwnerStatusRequestV2, SignedSubmissionV3,
    SignedUsernameUpdateV2, SubmissionAcceptedV1, SubmissionLifecycleV1,
    SubmissionOwnerStatusRequestV2, SubmissionOwnerStatusResponseV2, SubmissionV3,
    UsernameUpdateV2, Validate as _, VerifiedRunV2,
};
pub(crate) use serde::Serialize;
pub(crate) use serde::de::DeserializeOwned;
pub(crate) use std::net::{IpAddr, Ipv4Addr, SocketAddr};
pub(crate) use std::time::Duration;
pub(crate) use tower::ServiceExt as _;

pub(crate) const BOARD_ID: &str = "demo-standard-normal";
pub(crate) const SCORE_ONLY_BOARD_ID: &str = "demo-score-only";
pub(crate) const MISSION_ID: &str = "Dem_Lei_MP";

mod requests;
mod rig;

pub(crate) use requests::*;
pub(crate) use rig::*;
