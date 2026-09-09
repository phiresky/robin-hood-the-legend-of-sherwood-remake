//! Cancellation owns every connection, including incomplete HTTP bodies.
use super::*;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

type Response = hyper::Response<Full<Bytes>>;

/// A fully acquired request. The routing layer never performs network I/O.
#[derive(serde::Serialize)]
pub(super) struct NativeRequest {
    url: String,
    #[serde(skip)]
    method: hyper::Method,
    #[serde(skip)]
    headers: hyper::HeaderMap,
    declared_length: Option<usize>,
    #[serde(skip)]
    body: std::io::Cursor<Vec<u8>>,
}

impl<'de> serde::Deserialize<'de> for NativeRequest {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "HTTP request must be acquired from its transport",
        ))
    }
}

impl NativeRequest {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(name)
            .map(|value| value.to_str().expect("validated ASCII request header"))
    }
    pub(super) fn url(&self) -> &str {
        &self.url
    }
    pub(super) fn method(&self) -> &hyper::Method {
        &self.method
    }
    pub(super) fn body_length(&self) -> Option<usize> {
        self.declared_length
    }
    pub(super) fn as_reader(&mut self) -> &mut dyn std::io::Read {
        &mut self.body
    }
}

pub(super) async fn run(
    listener: tokio::net::TcpListener,
    queue: Queue,
    mut stop: tokio::sync::oneshot::Receiver<()>,
    port: u16,
) {
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            result = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = result { tracing::error!(%error, "HTTP connection task failed"); }
            }
            accepted = listener.accept(), if connections.len() < 8 => {
                let (stream, _) = match accepted {
                    Ok(connection) => connection,
                    Err(error) => { tracing::error!(%error, "HTTP accept failed"); break; }
                };
                let queue = queue.clone();
                connections.spawn(async move {
                    let service = service_fn(move |request| {
                        let queue = queue.clone();
                        async move { Ok::<_, std::convert::Infallible>(respond(request, &queue, port).await) }
                    });
                    if let Err(error) = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service).await {
                        tracing::debug!(%error, "HTTP connection ended");
                    }
                });
            }
        }
    }
    // Abort is cancellation, not graceful draining: a body that never ends or
    // a client that never reads its response must not keep the application alive.
    connections.abort_all();
    while let Some(result) = connections.join_next().await {
        if let Err(error) = result {
            if !error.is_cancelled() {
                tracing::error!(%error, "HTTP task failed during shutdown");
            }
        }
    }
    queue.lock().expect("RPC router poisoned").retire();
}

async fn respond(request: hyper::Request<Incoming>, queue: &Queue, port: u16) -> Response {
    match acquire(request, port).await {
        Ok(request) => {
            let (status, body) = super::native_routes::dispatch(request, queue, port).await;
            response(status, body)
        }
        Err((status, error)) => response(status, serde_json::json!({"error": error}).into()),
    }
}

async fn acquire(
    request: hyper::Request<Incoming>,
    port: u16,
) -> Result<NativeRequest, (u16, String)> {
    let (parts, body) = request.into_parts();
    for (name, value) in &parts.headers {
        value
            .to_str()
            .map_err(|_| (400, format!("invalid HTTP header: {name}")))?;
    }
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .map(|value| value.to_str().expect("validated header"))
    };
    if let Some(reason) = browser_rejection_reason(
        header("origin"),
        header("sec-fetch-site"),
        header("host"),
        port,
    ) {
        return Err((403, reason.to_owned()));
    }
    let limit = crate::replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS
        .max_input_bytes
        .checked_add(1024)
        .expect("HTTP replay envelope bound fits usize");
    let declared_length = header("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| (400, format!("invalid Content-Length: {error}")))?;
    if let Some(length) = declared_length {
        if length > limit {
            return Err((
                400,
                format!("HTTP body observed {length} bytes, limit is {limit}"),
            ));
        }
    }
    let mut request = NativeRequest {
        url: parts
            .uri
            .path_and_query()
            .map_or("/", |value| value.as_str())
            .to_owned(),
        method: parts.method,
        headers: parts.headers,
        declared_length,
        body: std::io::Cursor::new(Vec::new()),
    };
    if request.method == hyper::Method::POST
        && request.url.split('?').next() == Some("/load-replay")
    {
        super::native_routes::validate_replay_headers(&request).map_err(|error| (400, error))?;
    }
    let body = Limited::new(body, limit)
        .collect()
        .await
        .map_err(|error| (400, format!("HTTP body acquisition: {error}")))?
        .to_bytes();
    request.body = std::io::Cursor::new(body.to_vec());
    Ok(request)
}

fn response(status: u16, body: ReplyBody) -> Response {
    let (content_type, data) = match body {
        ReplyBody::Json(value) => (
            "application/json",
            serde_json::to_vec(&value).expect("RPC JSON values serialize"),
        ),
        ReplyBody::Binary { content_type, data } => (content_type, data),
    };
    hyper::Response::builder()
        .status(status)
        .header("content-type", content_type)
        .body(Full::new(Bytes::from(data)))
        .expect("valid RPC response status and content type")
}
