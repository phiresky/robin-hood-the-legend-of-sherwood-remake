//! Native automation routing, independent of the listener lifecycle.

use super::*;
use hyper::Method;

pub(super) async fn dispatch(
    mut req: NativeRequest,
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
        return (403, serde_json::json!({"error": reason}).into());
    }

    let path_full = req.url().to_string();
    let (path, query) = match path_full.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (path_full, String::new()),
    };
    let method = req.method().clone();

    match (&method, path.as_str()) {
        (Method::GET, "/") | (Method::GET, "/info") => (200, info_json().into()),
        (Method::GET, "/natives") => (200, list_natives_json().into()),
        (Method::GET, "/state") => relay(queue, HttpPayload::State).await,
        (Method::GET, "/host-debug") => relay(queue, HttpPayload::HostDebug).await,
        (Method::GET, "/engine-dump") => relay(queue, HttpPayload::EngineDump).await,
        (Method::GET, "/level-assets") => relay(queue, HttpPayload::LevelAssets).await,
        (Method::GET, "/script") => relay(queue, HttpPayload::Script).await,
        (Method::GET, "/script/decompile") => {
            let class = query_param(&query, "class").map(str::to_string);
            relay(queue, HttpPayload::Decompile { class }).await
        }
        (Method::GET, "/screenshot") => {
            relay(
                queue,
                HttpPayload::Screenshot(parse_screenshot_query(&query)),
            )
            .await
        }
        (Method::POST, "/native") => match read_json::<NativeCall>(&mut req) {
            Ok(c) => {
                relay(
                    queue,
                    HttpPayload::Native {
                        name: c.op,
                        args: c.args,
                        this: c.this,
                    },
                )
                .await
            }
            Err(e) => (400, serde_json::json!({"error": e}).into()),
        },
        (Method::POST, "/batch") => {
            #[derive(serde::Serialize, serde::Deserialize)]
            struct BatchBody {
                calls: Vec<NativeCall>,
            }
            match read_json::<BatchBody>(&mut req) {
                Ok(b) => relay(queue, HttpPayload::Batch(b.calls)).await,
                Err(e) => (400, serde_json::json!({"error": e}).into()),
            }
        }
        (Method::POST, "/console") => {
            #[derive(serde::Serialize, serde::Deserialize)]
            struct ConsoleBody {
                command: String,
            }
            match read_json::<ConsoleBody>(&mut req) {
                Ok(c) => relay(queue, HttpPayload::Console(c.command)).await,
                Err(e) => (400, serde_json::json!({"error": e}).into()),
            }
        }
        (Method::POST, "/command") => match read_json::<PlayerCommand>(&mut req) {
            Ok(c) => relay(queue, HttpPayload::Command(c)).await,
            Err(e) => (400, serde_json::json!({"error": e}).into()),
        },
        (Method::POST, "/step-forward") => match parse_step_body(&mut req) {
            Ok(request) => relay(queue, HttpPayload::StepForward { request }).await,
            Err(e) => (400, serde_json::json!({"error": e}).into()),
        },
        (Method::POST, "/step-back") => match parse_step_body(&mut req) {
            Ok(request) => relay(queue, HttpPayload::StepBack { request }).await,
            Err(e) => (400, serde_json::json!({"error": e}).into()),
        },
        (Method::POST, "/go-to-frame") => {
            #[derive(serde::Serialize, serde::Deserialize)]
            struct GoToBody {
                frame: u32,
                #[serde(flatten)]
                modal_policy: StepModalPolicy,
            }
            match read_json::<GoToBody>(&mut req) {
                Ok(b) => {
                    relay(
                        queue,
                        HttpPayload::GoToFrame {
                            target: b.frame,
                            modal_policy: b.modal_policy,
                        },
                    )
                    .await
                }
                Err(e) => (400, serde_json::json!({"error": e}).into()),
            }
        }
        (Method::POST, "/set-paused") => {
            #[derive(serde::Serialize, serde::Deserialize)]
            struct SetPausedBody {
                paused: bool,
            }
            match read_json::<SetPausedBody>(&mut req) {
                Ok(b) => relay(queue, HttpPayload::SetPaused { paused: b.paused }).await,
                Err(e) => (400, serde_json::json!({"error": e}).into()),
            }
        }
        (Method::GET, "/get-replay") => relay(queue, HttpPayload::GetReplay).await,
        (Method::POST, "/load-replay") => {
            #[derive(serde::Serialize, serde::Deserialize)]
            struct LoadReplayBody {
                data: String,
                #[serde(default)]
                paused: bool,
            }
            match read_replay_json::<LoadReplayBody>(&mut req) {
                Ok(b) => match crate::replay_format::preflight_compact_transport(
                    &b.data,
                    &crate::replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS,
                ) {
                    Ok(_) => {
                        relay(
                            queue,
                            HttpPayload::LoadReplay {
                                data: b.data,
                                paused: b.paused,
                            },
                        )
                        .await
                    }
                    Err(error) => (
                        400,
                        serde_json::json!({"error": format!("invalid compact replay: {error}")})
                            .into(),
                    ),
                },
                Err(e) => (400, serde_json::json!({"error": e}).into()),
            }
        }
        _ => (404, serde_json::json!({"error": "not found"}).into()),
    }
}

/// Empty step bodies mean one tick; explicit counts must be positive.
fn parse_step_body(req: &mut NativeRequest) -> Result<StepRequest, String> {
    let mut body = String::new();
    std::io::Read::read_to_string(req.as_reader(), &mut body)
        .map_err(|e| format!("body read: {e}"))?;
    if body.trim().is_empty() {
        return Ok(StepRequest::default());
    }
    let body: StepRequest = serde_json::from_str(&body).map_err(|e| format!("bad json: {e}"))?;
    if body.n == 0 {
        return Err("n must be >= 1".into());
    }
    Ok(body)
}

fn read_json<T: serde::de::DeserializeOwned>(req: &mut NativeRequest) -> Result<T, String> {
    let mut body = String::new();
    std::io::Read::read_to_string(req.as_reader(), &mut body)
        .map_err(|e| format!("body read: {e}"))?;
    serde_json::from_str(&body).map_err(|e| format!("bad json: {e}"))
}

/// Reject unsupported replay framing before acquiring the request body.
/// Returns the maximum accepted body size for the transport collector.
pub(super) fn validate_replay_headers(req: &NativeRequest) -> Result<usize, String> {
    const JSON_OVERHEAD_BYTES: usize = 1024;
    let limit = crate::replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS
        .max_input_bytes
        .checked_add(JSON_OVERHEAD_BYTES)
        .expect("replay JSON transport limit fits usize");
    if req.header("Transfer-Encoding").is_some() {
        return Err("load-replay does not accept Transfer-Encoding".into());
    }
    if req.header("Content-Encoding").is_some() {
        return Err("load-replay does not accept Content-Encoding".into());
    }
    let content_type = req
        .header("Content-Type")
        .ok_or_else(|| "load-replay requires Content-Type: application/json".to_string())?;
    let media_type = content_type.split(';').next().unwrap_or_default().trim();
    if !media_type.eq_ignore_ascii_case("application/json") {
        return Err("load-replay requires Content-Type: application/json".into());
    }
    let declared = req
        .body_length()
        .ok_or_else(|| "load-replay requires a bounded Content-Length".to_string())?;
    if declared > limit {
        return Err(format!(
            "load-replay JSON body observed {declared} bytes, limit is {limit}"
        ));
    }
    Ok(limit)
}

/// Validate the bounded replay transport before UTF-8/JSON/String allocation.
fn read_replay_json<T: serde::de::DeserializeOwned>(req: &mut NativeRequest) -> Result<T, String> {
    use std::io::Read as _;

    let limit = validate_replay_headers(req)?;
    let declared = req
        .body_length()
        .expect("validated replay headers contain Content-Length");
    let mut body = Vec::with_capacity(declared.min(limit));
    std::io::Read::take(req.as_reader(), (limit + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|error| format!("body read: {error}"))?;
    if body.len() > limit {
        return Err(format!(
            "load-replay JSON body observed at least {} bytes, limit is {limit}",
            body.len()
        ));
    }
    serde_json::from_slice(&body).map_err(|error| format!("bad json: {error}"))
}
