//! Axum extractors shared by the public request handlers.

use crate::error::ApiError;
use axum::Json;
use axum::extract::{ConnectInfo, FromRequest, FromRequestParts, Request};
use axum::http::HeaderValue;
use axum::http::request::Parts;
use axum::response::{IntoResponse as _, Response};
use robin_run_protocol::Validate;
use serde::de::DeserializeOwned;
use std::net::{IpAddr, SocketAddr};

/// Transport peer plus the (first) `X-Forwarded-For` header of a request.
///
/// Resolution to the effective client address is deliberately deferred to
/// [`ClientIp::resolve`]: it needs the trusted-proxy configuration and its
/// errors must keep being reported *after* JSON body rejection, exactly like
/// the previous `ConnectInfo` + `HeaderMap` handler parameters. A missing
/// `ConnectInfo` extension is rejected with the same response as axum's own
/// `ConnectInfo` extractor.
pub(super) struct ClientIp {
    peer: SocketAddr,
    forwarded_for: Option<HeaderValue>,
}

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let ConnectInfo(peer) = ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| rejection.into_response())?;
        Ok(Self {
            peer,
            forwarded_for: parts.headers.get("x-forwarded-for").cloned(),
        })
    }
}

impl ClientIp {
    pub(super) fn resolve(&self, config: &crate::ServerConfig) -> Result<IpAddr, ApiError> {
        effective_client_ip(config, self.peer, self.forwarded_for.as_ref())
    }
}

pub(super) fn effective_client_ip(
    config: &crate::ServerConfig,
    peer: SocketAddr,
    forwarded_for: Option<&HeaderValue>,
) -> Result<IpAddr, ApiError> {
    let trusted = config
        .trusted_proxy_cidrs
        .iter()
        .filter_map(|network| network.parse::<ipnet::IpNet>().ok())
        .any(|network| network.contains(&peer.ip()));
    if !trusted {
        return Ok(peer.ip());
    }
    let forwarded = forwarded_for
        .ok_or_else(|| ApiError::BadRequest("trusted proxy omitted X-Forwarded-For".to_owned()))?
        .to_str()
        .map_err(|_| {
            ApiError::BadRequest("trusted proxy sent invalid X-Forwarded-For".to_owned())
        })?;
    if forwarded.contains(',') || forwarded.trim() != forwarded {
        return Err(ApiError::BadRequest(
            "trusted proxy must supply exactly one canonical X-Forwarded-For address".to_owned(),
        ));
    }
    forwarded
        .parse::<IpAddr>()
        .map_err(|_| ApiError::BadRequest("trusted proxy sent invalid X-Forwarded-For".to_owned()))
}

/// `Json<T>` followed immediately by `T::validate()`.
///
/// JSON rejections are returned as axum's own `JsonRejection` response
/// (unchanged from a plain `Json<T>` parameter); validation failures become
/// `ApiError::BadRequest` exactly like a handler-side `request.validate()?`.
/// Only use this where the handler previously validated as its first
/// statement, so the error precedence is preserved.
pub(super) struct ValidatedJson<T>(pub(super) T);

impl<T, S> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(request, state)
            .await
            .map_err(|rejection| rejection.into_response())?;
        value
            .validate()
            .map_err(|error| ApiError::from(error).into_response())?;
        Ok(Self(value))
    }
}
