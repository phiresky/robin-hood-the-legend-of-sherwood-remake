//! Native automation routing, independent of the listener lifecycle.

use super::request_decode::{self, RequestKind};
use super::*;
use hyper::Method;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
enum Route {
    Info,
    Natives,
    EngineDump,
    Decompile,
    Screenshot,
    Rpc(RequestKind),
}

/// Route recognition is also the transport's body-acquisition policy. Unknown
/// paths and unsupported methods never acquire a potentially large body.
fn classify(method: &Method, path: &str) -> Option<Route> {
    Some(match (method, path) {
        (&Method::GET, "/") | (&Method::GET, "/info") => Route::Info,
        (&Method::GET, "/natives") => Route::Natives,
        (&Method::GET, "/engine-dump") => Route::EngineDump,
        (&Method::GET, "/script/decompile") => Route::Decompile,
        (&Method::GET, "/screenshot") => Route::Screenshot,
        (&Method::GET, "/state" | "/host-debug" | "/level-assets" | "/script" | "/get-replay")
        | (
            &Method::POST,
            "/native" | "/batch" | "/console" | "/command" | "/step-forward" | "/step-back"
            | "/go-to-frame" | "/set-paused" | "/load-replay",
        ) => Route::Rpc(RequestKind::from_method(&path[1..]).expect("classified RPC method")),
        _ => return None,
    })
}

fn replay_body_limit() -> usize {
    const JSON_OVERHEAD_BYTES: usize = 1024;
    crate::replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS
        .max_input_bytes
        .checked_add(JSON_OVERHEAD_BYTES)
        .expect("replay JSON transport limit fits usize")
}

pub(super) fn body_limit(method: &Method, path: &str) -> Option<usize> {
    let route = classify(method, path)?;
    Some(if *method == Method::GET {
        0
    } else {
        match route {
            Route::Rpc(RequestKind::LoadReplay) => replay_body_limit(),
            Route::Rpc(RequestKind::Batch) => 1024 * 1024,
            _ => 64 * 1024,
        }
    })
}

pub(super) async fn dispatch(
    req: NativeRequest,
    queue: &Queue,
    listen_port: u16,
) -> (u16, ReplyBody) {
    if let Some(reason) = browser_rejection_reason(
        req.header("Origin"),
        req.header("Sec-Fetch-Site"),
        req.header("Host"),
        listen_port,
    ) {
        tracing::warn!("script HTTP server: rejected request: {reason}");
        return (403, RpcError::invalid_request(reason).wire_body().into());
    }
    let (path, query) = req.url().split_once('?').unwrap_or((req.url(), ""));
    let payload = match classify(req.method(), path) {
        Some(Route::Info) => return (200, info_json().into()),
        Some(Route::Natives) => return (200, list_natives_json().into()),
        Some(Route::EngineDump) => Ok(HttpPayload::EngineDump),
        Some(Route::Decompile) => {
            crate::rpc_query::decompile_class(query).map(|class| HttpPayload::Decompile { class })
        }
        Some(Route::Screenshot) => crate::rpc_query::screenshot(query).map(HttpPayload::Screenshot),
        Some(Route::Rpc(kind)) => {
            if matches!(kind, RequestKind::LoadReplay)
                && let Err(error) = validate_replay_headers(&req)
            {
                return (400, error.wire_body().into());
            }
            request_decode::decode_json(kind, req.body_bytes())
        }
        None => {
            return (
                404,
                RpcError::unavailable_capability("not found")
                    .wire_body()
                    .into(),
            );
        }
    };
    match payload {
        Ok(payload) => relay(queue, payload).await,
        Err(error) => (400, error.wire_body().into()),
    }
}

/// Reject unsupported replay framing before acquiring the request body.
/// Returns the maximum accepted body size for the transport collector.
pub(super) fn validate_replay_headers(req: &NativeRequest) -> Result<usize, RpcError> {
    let limit = replay_body_limit();
    if req.header("Transfer-Encoding").is_some() {
        return Err(RpcError::invalid_request(
            "load-replay does not accept Transfer-Encoding",
        ));
    }
    if req.header("Content-Encoding").is_some() {
        return Err(RpcError::invalid_request(
            "load-replay does not accept Content-Encoding",
        ));
    }
    let content_type = req.header("Content-Type").ok_or_else(|| {
        RpcError::invalid_request("load-replay requires Content-Type: application/json")
    })?;
    let media_type = content_type.split(';').next().unwrap_or_default().trim();
    if !media_type.eq_ignore_ascii_case("application/json") {
        return Err(RpcError::invalid_request(
            "load-replay requires Content-Type: application/json",
        ));
    }
    let declared = req.body_length().ok_or_else(|| {
        RpcError::invalid_request("load-replay requires a bounded Content-Length")
    })?;
    if declared > limit {
        return Err(RpcError::capacity(format!(
            "load-replay JSON body observed {declared} bytes, limit is {limit}"
        )));
    }
    Ok(limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_body_policy_preserves_native_aliases_and_rejects_unknown_routes() {
        for path in [
            "/",
            "/info",
            "/natives",
            "/state",
            "/host-debug",
            "/engine-dump",
            "/level-assets",
            "/script",
            "/script/decompile",
            "/screenshot",
            "/get-replay",
        ] {
            assert_eq!(body_limit(&Method::GET, path), Some(0), "{path}");
            assert_eq!(body_limit(&Method::POST, path), None, "{path}");
        }
        for path in [
            "/native",
            "/console",
            "/command",
            "/step-forward",
            "/step-back",
            "/go-to-frame",
            "/set-paused",
        ] {
            assert_eq!(body_limit(&Method::POST, path), Some(64 * 1024), "{path}");
            assert_eq!(body_limit(&Method::GET, path), None, "{path}");
        }
        assert_eq!(body_limit(&Method::POST, "/batch"), Some(1024 * 1024));
        assert_eq!(
            body_limit(&Method::POST, "/load-replay"),
            Some(replay_body_limit())
        );
        assert_eq!(body_limit(&Method::GET, "/decompile"), None);
        assert_eq!(body_limit(&Method::POST, "/unknown"), None);
        assert_eq!(body_limit(&Method::PUT, "/native"), None);
    }
}
