//! Cancellation owns every connection, including incomplete HTTP bodies.
use super::*;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};

type Response = hyper::Response<Full<Bytes>>;

/// Acquisition deadlines are independent of the mission's deferred RPC deadline.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ResourcePolicy {
    headers: std::time::Duration,
    body: std::time::Duration,
    connection: std::time::Duration,
}

impl Default for ResourcePolicy {
    fn default() -> Self {
        Self {
            headers: std::time::Duration::from_secs(10),
            body: std::time::Duration::from_secs(30),
            connection: std::time::Duration::from_secs(120),
        }
    }
}

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
    body: Bytes,
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
    pub(super) fn body_bytes(&self) -> &[u8] {
        &self.body
    }
}

pub(super) async fn run(
    listener: tokio::net::TcpListener,
    queue: Queue,
    stop: tokio::sync::oneshot::Receiver<()>,
    port: u16,
) {
    run_with_policy(listener, queue, stop, port, ResourcePolicy::default()).await;
}

async fn run_with_policy(
    listener: tokio::net::TcpListener,
    queue: Queue,
    mut stop: tokio::sync::oneshot::Receiver<()>,
    port: u16,
    policy: ResourcePolicy,
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
                        async move { Ok::<_, std::convert::Infallible>(respond(request, &queue, port, policy).await) }
                    });
                    let mut builder = hyper::server::conn::http1::Builder::new();
                    // Hyper restarts this deadline on every read_head, including
                    // idle keep-alive connections awaiting their next request.
                    builder.timer(TokioTimer::new())
                        .header_read_timeout(policy.headers);
                    match tokio::time::timeout(policy.connection,
                        builder.serve_connection(TokioIo::new(stream), service)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => tracing::debug!(%error, "HTTP connection ended"),
                        Err(_) => tracing::warn!("HTTP connection deadline exceeded"),
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

async fn respond(
    request: hyper::Request<Incoming>,
    queue: &Queue,
    port: u16,
    policy: ResourcePolicy,
) -> Response {
    match acquire(request, port, policy).await {
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
    policy: ResourcePolicy,
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
    // Resolve the route before polling Incoming: unknown endpoints must not
    // acquire replay-sized bodies (or wait for a client to finish one).
    let limit = super::native_routes::body_limit(&parts.method, parts.uri.path())
        .ok_or_else(|| (404, "not found".to_owned()))?;
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
        body: Bytes::new(),
    };
    if request.method == hyper::Method::POST
        && request.url.split('?').next() == Some("/load-replay")
    {
        super::native_routes::validate_replay_headers(&request).map_err(|error| (400, error))?;
    }
    let body = tokio::time::timeout(policy.body, Limited::new(body, limit).collect())
        .await
        .map_err(|_| (408, "HTTP body acquisition deadline exceeded".to_owned()))?
        .map_err(|error| (400, format!("HTTP body acquisition: {error}")))?
        .to_bytes();
    request.body = body;
    Ok(request)
}

fn response(status: u16, body: ReplyBody) -> Response {
    let (content_type, data) = match body {
        ReplyBody::ReplayExport(_) => {
            panic!("deferred replay export must be resolved before HTTP encoding")
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn policy() -> ResourcePolicy {
        ResourcePolicy {
            headers: Duration::from_millis(100),
            body: Duration::from_millis(100),
            connection: Duration::from_secs(2),
        }
    }

    async fn server() -> (
        u16,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let queue = Arc::new(Mutex::new(RequestRouter::default()));
        let (stop, stopped) = tokio::sync::oneshot::channel();
        (
            port,
            stop,
            tokio::spawn(run_with_policy(listener, queue, stopped, port, policy())),
        )
    }

    async fn exchange(port: u16, request: &str) -> String {
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            client.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).await.unwrap();
            response
        })
        .await
        .expect("bounded HTTP exchange")
    }

    #[tokio::test]
    async fn eight_stalled_headers_release_slots_for_a_healthy_request() {
        let (port, stop, task) = server().await;
        let mut stalled = Vec::new();
        for _ in 0..8 {
            let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            client
                .write_all(b"GET /info HTTP/1.1\r\nHost:")
                .await
                .unwrap();
            stalled.push(client);
        }
        let response = exchange(
            port,
            &format!("GET /info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        for mut client in stalled {
            let mut bytes = Vec::new();
            // Header timeout may either send 408 or close/reset the socket.
            let _ = tokio::time::timeout(Duration::from_secs(1), client.read_to_end(&mut bytes))
                .await
                .expect("stalled headers must be closed");
        }
        stop.send(()).unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn incomplete_body_returns_timeout_and_unknown_or_oversized_routes_reject_early() {
        let (port, stop, task) = server().await;
        let response = exchange(port, &format!("POST /console HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: 20\r\n\r\n{{")).await;
        assert!(response.starts_with("HTTP/1.1 408"), "{response}");
        for (path, expected) in [("/console", 400), ("/unknown", 404)] {
            let response = exchange(port, &format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: 67108864\r\n\r\n")).await;
            assert!(
                response.starts_with(&format!("HTTP/1.1 {expected}")),
                "{response}"
            );
            assert!(!response.contains("deadline exceeded"));
        }
        stop.send(()).unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn keep_alive_is_supported_but_idle_connections_expire() {
        let (port, stop, task) = server().await;
        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        client
            .write_all(format!("GET /info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), client.read_to_end(&mut response))
            .await
            .expect("idle keep-alive must expire")
            .unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(!response.to_ascii_lowercase().contains("connection: close"));
        stop.send(()).unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn replay_acquisition_preserves_bytes_above_the_small_command_limit() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = serde_json::json!({"data": "a".repeat(70_000), "paused": true}).to_string();
        let expected = body.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let service = service_fn(move |request| {
                let expected = expected.clone();
                async move {
                    let acquired = acquire(request, port, policy()).await.unwrap();
                    assert_eq!(acquired.body_bytes(), expected.as_bytes());
                    Ok::<_, std::convert::Infallible>(response(
                        200,
                        serde_json::json!({"acquired": true}).into(),
                    ))
                }
            });
            hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
                .unwrap();
        });
        let reply = exchange(port, &format!("POST /load-replay HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len())).await;
        assert!(reply.starts_with("HTTP/1.1 200"));
        server.await.unwrap();
    }
}
