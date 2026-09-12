//! Bounded, frame-polled HTTP for the in-game leaderboard client.
//!
//! Native requests run on short-lived worker threads; browser requests run as
//! local fetch futures. The caller only polls a capacity-one completion
//! channel. Redirects are refused on both platforms, especially for the
//! signed multipart body containing the canonical replay and campaign.

use robin_run_protocol::{RANKED_CAMPAIGN_MEDIA_TYPE_V1, RANKED_REPLAY_MEDIA_TYPE_V1};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const DEFAULT_REQUEST_TIMEOUT_MS: u32 = 15_000;
pub const MAX_JSON_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_REPLAY_RESPONSE_BYTES: usize =
    robin_replay_format::DEFAULT_REPLAY_ADMISSION_LIMITS.max_input_bytes;
const DEFAULT_MAX_IN_FLIGHT: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HttpTransportError {
    #[error("request timed out")]
    Timeout,
    #[error("request failed: {0}")]
    Request(String),
    #[error("response body exceeds its {limit}-byte limit")]
    ResponseTooLarge { limit: usize },
    #[error("leaderboard HTTP concurrency limit is reached")]
    TooManyInFlight,
    #[error("HTTP worker closed without a result")]
    WorkerClosed,
    #[error("starting campaign is mandatory")]
    MissingStartingCampaign,
    #[error("replay body is mandatory")]
    MissingReplay,
}

#[derive(Debug)]
pub struct HttpTask {
    receiver: async_channel::Receiver<Result<HttpResponse, HttpTransportError>>,
}

impl HttpTask {
    /// Attach a typed response decoder without another one-purpose task type.
    /// Decoding remains lazy and nonblocking at the consumer's polling boundary.
    pub fn map<T, E>(
        self,
        mut decode: impl FnMut(Result<HttpResponse, HttpTransportError>) -> Result<T, E>,
    ) -> impl FnMut() -> Option<Result<T, E>> {
        move || self.try_take().map(&mut decode)
    }

    /// Non-blocking completion check intended to run once per graphical frame.
    pub fn try_take(&self) -> Option<Result<HttpResponse, HttpTransportError>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => Some(Err(HttpTransportError::WorkerClosed)),
        }
    }

    /// Await completion during pre-frame mission admission. This consumes the
    /// one-shot task, retains the same bounded transport, and never blocks a
    /// native executor thread or the browser event loop.
    pub async fn take(self) -> Result<HttpResponse, HttpTransportError> {
        self.receiver
            .recv()
            .await
            .map_err(|_| HttpTransportError::WorkerClosed)?
    }
}

#[derive(Debug, Clone)]
pub enum HttpRequestBody {
    Empty,
    Json(Arc<[u8]>),
    /// The only upload form. Roles and media types are deliberately not
    /// caller-selectable.
    SubmissionMultipart {
        submission_json: Arc<[u8]>,
        replay_bytes: Arc<[u8]>,
        starting_campaign_bytes: Arc<[u8]>,
    },
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: reqwest::Method,
    pub url: String,
    pub body: HttpRequestBody,
    pub no_store: bool,
    pub max_response_bytes: usize,
}

impl HttpRequest {
    pub fn get_json(url: String) -> Self {
        Self {
            method: reqwest::Method::GET,
            url,
            body: HttpRequestBody::Empty,
            no_store: false,
            max_response_bytes: MAX_JSON_RESPONSE_BYTES,
        }
    }

    #[cfg(test)]
    pub fn get_replay(url: String, expected_bytes: u64) -> Result<Self, HttpTransportError> {
        let expected_bytes = usize::try_from(expected_bytes).map_err(|error| {
            request_error(format!("replay length is not representable: {error}"))
        })?;
        if expected_bytes == 0 {
            return Err(HttpTransportError::MissingReplay);
        }
        if expected_bytes > MAX_REPLAY_RESPONSE_BYTES {
            return Err(HttpTransportError::ResponseTooLarge {
                limit: MAX_REPLAY_RESPONSE_BYTES,
            });
        }
        Ok(Self {
            method: reqwest::Method::GET,
            url,
            body: HttpRequestBody::Empty,
            no_store: true,
            max_response_bytes: expected_bytes,
        })
    }

    pub fn json(method: reqwest::Method, url: String, json: Vec<u8>) -> Self {
        Self {
            method,
            url,
            body: HttpRequestBody::Json(json.into()),
            no_store: true,
            max_response_bytes: MAX_JSON_RESPONSE_BYTES,
        }
    }

    pub fn submission_multipart(
        url: String,
        submission_json: Arc<[u8]>,
        replay_bytes: Arc<[u8]>,
        starting_campaign_bytes: Arc<[u8]>,
    ) -> Result<Self, HttpTransportError> {
        if replay_bytes.is_empty() {
            return Err(HttpTransportError::MissingReplay);
        }
        if starting_campaign_bytes.is_empty() {
            return Err(HttpTransportError::MissingStartingCampaign);
        }
        Ok(Self {
            method: reqwest::Method::POST,
            url,
            body: HttpRequestBody::SubmissionMultipart {
                submission_json,
                replay_bytes,
                starting_campaign_bytes,
            },
            no_store: true,
            max_response_bytes: MAX_JSON_RESPONSE_BYTES,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LeaderboardHttpClient {
    in_flight: Arc<AtomicUsize>,
    max_in_flight: usize,
    timeout_ms: u32,
}

impl Default for LeaderboardHttpClient {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_IN_FLIGHT, DEFAULT_REQUEST_TIMEOUT_MS)
            .expect("default leaderboard HTTP policy is valid")
    }
}

impl LeaderboardHttpClient {
    pub fn new(max_in_flight: usize, timeout_ms: u32) -> Result<Self, HttpTransportError> {
        if max_in_flight == 0 || timeout_ms == 0 {
            return Err(request_error(
                "HTTP concurrency and timeout must both be positive",
            ));
        }
        Ok(Self {
            in_flight: Arc::new(AtomicUsize::new(0)),
            max_in_flight,
            timeout_ms,
        })
    }

    pub fn spawn(&self, request: HttpRequest) -> Result<HttpTask, HttpTransportError> {
        if request.max_response_bytes == 0 || request.max_response_bytes > MAX_REPLAY_RESPONSE_BYTES
        {
            return Err(request_error("invalid response-body limit"));
        }
        let permit = InFlightPermit::acquire(Arc::clone(&self.in_flight), self.max_in_flight)?;
        let (sender, receiver) = async_channel::bounded(1);
        spawn_platform(request, self.timeout_ms, sender, permit)?;
        Ok(HttpTask { receiver })
    }
}

#[derive(Debug)]
struct InFlightPermit {
    counter: Arc<AtomicUsize>,
}

impl InFlightPermit {
    fn acquire(counter: Arc<AtomicUsize>, maximum: usize) -> Result<Self, HttpTransportError> {
        counter
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < maximum).then_some(current + 1)
            })
            .map_err(|_| HttpTransportError::TooManyInFlight)?;
        Ok(Self { counter })
    }
}

impl Drop for InFlightPermit {
    fn drop(&mut self) {
        let previous = self.counter.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "leaderboard HTTP permit underflow");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_platform(
    request: HttpRequest,
    timeout_ms: u32,
    sender: async_channel::Sender<Result<HttpResponse, HttpTransportError>>,
    permit: InFlightPermit,
) -> Result<(), HttpTransportError> {
    std::thread::Builder::new()
        .name("leaderboard-http".to_owned())
        .spawn(move || {
            let _permit = permit;
            let result = execute_native(request, timeout_ms);
            let _ = sender.send_blocking(result);
        })
        .map(|_| ())
        .map_err(|error| request_error(format!("failed to spawn HTTP worker: {error}")))
}

#[cfg(not(target_arch = "wasm32"))]
fn execute_native(
    request: HttpRequest,
    timeout_ms: u32,
) -> Result<HttpResponse, HttpTransportError> {
    // rustls deliberately has no implicit process-wide provider. An already
    // installed provider (for example iroh's) is valid and remains in force.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_millis(u64::from(timeout_ms)))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(request_error)?;
    let response_limit = request.max_response_bytes;
    let mut builder = client.request(request.method, request.url);
    if request.no_store {
        builder = builder.header(reqwest::header::CACHE_CONTROL, "no-store");
    }
    let builder = attach_native_body(builder, request.body)?;
    let response = builder.send().map_err(classify_native_error)?;
    read_native_response(response, response_limit)
}

#[cfg(not(target_arch = "wasm32"))]
fn attach_native_body(
    builder: reqwest::blocking::RequestBuilder,
    body: HttpRequestBody,
) -> Result<reqwest::blocking::RequestBuilder, HttpTransportError> {
    Ok(match body {
        HttpRequestBody::Empty => builder,
        HttpRequestBody::Json(json) => builder
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(json.to_vec()),
        HttpRequestBody::SubmissionMultipart {
            submission_json,
            replay_bytes,
            starting_campaign_bytes,
        } => {
            let submission = reqwest::blocking::multipart::Part::bytes(submission_json.to_vec())
                .file_name("submission.json")
                .mime_str("application/json")
                .map_err(request_error)?;
            let replay = reqwest::blocking::multipart::Part::bytes(replay_bytes.to_vec())
                .file_name("replay.rhrec")
                .mime_str(RANKED_REPLAY_MEDIA_TYPE_V1)
                .map_err(request_error)?;
            let starting_campaign =
                reqwest::blocking::multipart::Part::bytes(starting_campaign_bytes.to_vec())
                    .file_name("starting-campaign.bin")
                    .mime_str(RANKED_CAMPAIGN_MEDIA_TYPE_V1)
                    .map_err(request_error)?;
            builder.multipart(
                reqwest::blocking::multipart::Form::new()
                    .part("submission", submission)
                    .part("replay", replay)
                    .part("starting_campaign", starting_campaign),
            )
        }
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn read_native_response(
    mut response: reqwest::blocking::Response,
    limit: usize,
) -> Result<HttpResponse, HttpTransportError> {
    use std::io::Read as _;

    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(HttpTransportError::ResponseTooLarge { limit });
    }
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(limit as u64) as usize,
    );
    response
        .by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut body)
        .map_err(request_error)?;
    if body.len() > limit {
        return Err(HttpTransportError::ResponseTooLarge { limit });
    }
    Ok(HttpResponse {
        status,
        content_type,
        body,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn classify_native_error(error: reqwest::Error) -> HttpTransportError {
    if error.is_timeout() {
        HttpTransportError::Timeout
    } else {
        request_error(error)
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn_platform(
    request: HttpRequest,
    timeout_ms: u32,
    sender: async_channel::Sender<Result<HttpResponse, HttpTransportError>>,
    permit: InFlightPermit,
) -> Result<(), HttpTransportError> {
    wasm_bindgen_futures::spawn_local(async move {
        let _permit = permit;
        let result = execute_browser(request, timeout_ms).await;
        let _ = sender.send(result).await;
    });
    Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn execute_browser(
    request: HttpRequest,
    timeout_ms: u32,
) -> Result<HttpResponse, HttpTransportError> {
    let response_limit = request.max_response_bytes;
    let controller = web_sys::AbortController::new().map_err(browser_js_error)?;
    let request = build_browser_request(request, &controller.signal())?;
    let mut deadline = BrowserDeadline::new(controller, timeout_ms);
    let window = web_sys::window().ok_or_else(|| request_error("browser window is unavailable"))?;
    let fetch = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request));
    let response_value = deadline.wait(fetch).await?.map_err(browser_js_error)?;
    let response: web_sys::Response = wasm_bindgen::JsCast::dyn_into(response_value)
        .map_err(|_| request_error("fetch returned a non-response value"))?;
    let content_length = response
        .headers()
        .get("content-length")
        .map_err(browser_js_error)?
        .map(|raw| {
            raw.parse::<u64>()
                .map_err(|_| request_error("response Content-Length is not an integer"))
        })
        .transpose()?;
    if content_length.is_some_and(|length| length > response_limit as u64) {
        deadline.abort();
        return Err(HttpTransportError::ResponseTooLarge {
            limit: response_limit,
        });
    }
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .map_err(browser_js_error)?;
    let body = collect_browser_body(
        response.body(),
        content_length,
        response_limit,
        &mut deadline,
    )
    .await?;
    Ok(HttpResponse {
        status,
        content_type,
        body,
    })
}

#[cfg(target_arch = "wasm32")]
struct BrowserDeadline {
    controller: web_sys::AbortController,
    timeout: std::pin::Pin<Box<gloo_timers::future::TimeoutFuture>>,
}

#[cfg(target_arch = "wasm32")]
impl BrowserDeadline {
    fn new(controller: web_sys::AbortController, timeout_ms: u32) -> Self {
        Self {
            controller,
            timeout: Box::pin(gloo_timers::future::TimeoutFuture::new(timeout_ms)),
        }
    }

    async fn wait<F: std::future::Future>(
        &mut self,
        future: F,
    ) -> Result<F::Output, HttpTransportError> {
        let future = Box::pin(future);
        match futures::future::select(future, self.timeout.as_mut()).await {
            futures::future::Either::Left((result, _)) => Ok(result),
            futures::future::Either::Right(((), _)) => {
                self.abort();
                Err(HttpTransportError::Timeout)
            }
        }
    }

    fn abort(&self) {
        self.controller.abort();
    }
}

#[cfg(target_arch = "wasm32")]
async fn collect_browser_body(
    raw_body: Option<web_sys::ReadableStream>,
    content_length: Option<u64>,
    limit: usize,
    deadline: &mut BrowserDeadline,
) -> Result<Vec<u8>, HttpTransportError> {
    use futures::StreamExt as _;

    let mut body =
        Vec::with_capacity(content_length.unwrap_or_default().min(limit as u64) as usize);
    if let Some(raw_body) = raw_body {
        let mut chunks = wasm_streams::ReadableStream::from_raw(raw_body).into_stream();
        while let Some(chunk) = deadline.wait(chunks.next()).await? {
            let chunk = chunk.map_err(browser_js_error)?;
            let chunk = js_sys::Uint8Array::new(&chunk);
            let chunk_len = chunk.length() as usize;
            if body.len().saturating_add(chunk_len) > limit {
                deadline.abort();
                return Err(HttpTransportError::ResponseTooLarge { limit });
            }
            let start = body.len();
            body.resize(start + chunk_len, 0);
            chunk.copy_to(&mut body[start..]);
        }
    }
    Ok(body)
}

#[cfg(target_arch = "wasm32")]
fn build_browser_request(
    request: HttpRequest,
    signal: &web_sys::AbortSignal,
) -> Result<web_sys::Request, HttpTransportError> {
    let init = web_sys::RequestInit::new();
    init.set_method(request.method.as_str());
    init.set_redirect(web_sys::RequestRedirect::Error);
    init.set_credentials(web_sys::RequestCredentials::Omit);
    init.set_cache(if request.no_store {
        web_sys::RequestCache::NoStore
    } else {
        web_sys::RequestCache::Default
    });
    init.set_mode(web_sys::RequestMode::Cors);
    init.set_signal(Some(signal));
    let headers = web_sys::Headers::new().map_err(browser_js_error)?;
    match request.body {
        HttpRequestBody::Empty => {}
        HttpRequestBody::Json(json) => {
            headers
                .set("Content-Type", "application/json")
                .map_err(browser_js_error)?;
            let bytes = js_sys::Uint8Array::from(json.as_ref());
            init.set_body(&bytes.into());
        }
        HttpRequestBody::SubmissionMultipart {
            submission_json,
            replay_bytes,
            starting_campaign_bytes,
        } => {
            let form = web_sys::FormData::new().map_err(browser_js_error)?;
            let submission = browser_blob(&submission_json, "application/json")?;
            form.append_with_blob_and_filename("submission", &submission, "submission.json")
                .map_err(browser_js_error)?;
            let replay = browser_blob(&replay_bytes, RANKED_REPLAY_MEDIA_TYPE_V1)?;
            form.append_with_blob_and_filename("replay", &replay, "replay.rhrec")
                .map_err(browser_js_error)?;
            let starting_campaign =
                browser_blob(&starting_campaign_bytes, RANKED_CAMPAIGN_MEDIA_TYPE_V1)?;
            form.append_with_blob_and_filename(
                "starting_campaign",
                &starting_campaign,
                "starting-campaign.bin",
            )
            .map_err(browser_js_error)?;
            // Fetch must author the multipart boundary.
            init.set_body(&form.into());
        }
    }
    init.set_headers(&headers.into());
    web_sys::Request::new_with_str_and_init(&request.url, &init).map_err(browser_js_error)
}

#[cfg(target_arch = "wasm32")]
fn browser_blob(bytes: &[u8], media_type: &str) -> Result<web_sys::Blob, HttpTransportError> {
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(media_type);
    web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(browser_js_error)
}

#[cfg(target_arch = "wasm32")]
fn browser_js_error(error: wasm_bindgen::JsValue) -> HttpTransportError {
    request_error(
        error
            .as_string()
            .unwrap_or_else(|| format!("browser fetch failed: {error:?}")),
    )
}

fn request_error(error: impl std::fmt::Display) -> HttpTransportError {
    HttpTransportError::Request(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    fn wait_for(task: &HttpTask) -> Result<HttpResponse, HttpTransportError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(result) = task.try_take() {
                return result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "leaderboard HTTP test task did not complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn request_builder_requires_both_exact_artifacts() {
        assert!(matches!(
            HttpRequest::submission_multipart(
                "http://127.0.0.1:9/api/v1/submissions".to_owned(),
                Arc::from(&b"{}"[..]),
                Arc::from(&b""[..]),
                Arc::from(&b"campaign"[..]),
            ),
            Err(HttpTransportError::MissingReplay)
        ));
        assert!(matches!(
            HttpRequest::submission_multipart(
                "http://127.0.0.1:9/api/v1/submissions".to_owned(),
                Arc::from(&b"{}"[..]),
                Arc::from(&b"replay"[..]),
                Arc::from(&b""[..]),
            ),
            Err(HttpTransportError::MissingStartingCampaign)
        ));
    }

    #[test]
    fn replay_response_limit_is_exact_and_globally_bounded() {
        let request = HttpRequest::get_replay("/api/v1/runs/id/replay".to_owned(), 123).unwrap();
        assert_eq!(request.max_response_bytes, 123);
        assert!(matches!(
            HttpRequest::get_replay("/api/v1/runs/id/replay".to_owned(), 0),
            Err(HttpTransportError::MissingReplay)
        ));
        assert!(matches!(
            HttpRequest::get_replay(
                "/api/v1/runs/id/replay".to_owned(),
                MAX_REPLAY_RESPONSE_BYTES as u64 + 1
            ),
            Err(HttpTransportError::ResponseTooLarge { .. })
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_network_wait_never_blocks_and_concurrency_is_bounded() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let server_thread = std::thread::spawn(move || {
            let request = server.recv().unwrap();
            release_rx.recv().unwrap();
            request
                .respond(tiny_http::Response::from_string("ok").with_status_code(200))
                .unwrap();
        });

        let client = LeaderboardHttpClient::new(1, 1_000).unwrap();
        let task = client
            .spawn(HttpRequest::get_json(format!("http://{address}/held")))
            .unwrap();
        assert!(task.try_take().is_none());
        assert!(matches!(
            client.spawn(HttpRequest::get_json(format!("http://{address}/second"))),
            Err(HttpTransportError::TooManyInFlight)
        ));
        release_tx.send(()).unwrap();
        assert_eq!(wait_for(&task).unwrap().body, b"ok");
        server_thread.join().unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn multipart_transport_preserves_order_media_types_and_exact_bytes() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let exact_replay: Arc<[u8]> = Arc::from(&b"header\n\0canonical\r\nreplay\xffbytes"[..]);
        let exact_campaign: Arc<[u8]> = Arc::from(&b"\0full campaign\xffbytes"[..]);
        let expected_replay = exact_replay.clone();
        let expected_campaign = exact_campaign.clone();
        let server_thread = std::thread::spawn(move || {
            let mut request = server.recv().unwrap();
            let mut body = Vec::new();
            request.as_reader().read_to_end(&mut body).unwrap();
            let text = String::from_utf8_lossy(&body);
            let submission_at = text.find("name=\"submission\"").unwrap();
            let replay_at = text.find("name=\"replay\"").unwrap();
            let campaign_at = text.find("name=\"starting_campaign\"").unwrap();
            assert!(submission_at < replay_at && replay_at < campaign_at);
            assert!(text.contains(RANKED_REPLAY_MEDIA_TYPE_V1));
            assert!(text.contains(RANKED_CAMPAIGN_MEDIA_TYPE_V1));
            assert!(
                body.windows(expected_replay.len())
                    .any(|window| window == expected_replay.as_ref())
            );
            assert!(
                body.windows(expected_campaign.len())
                    .any(|window| window == expected_campaign.as_ref())
            );
            request
                .respond(
                    tiny_http::Response::from_string("{}")
                        .with_status_code(202)
                        .with_header(
                            tiny_http::Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"application/json"[..],
                            )
                            .unwrap(),
                        ),
                )
                .unwrap();
        });

        let task = LeaderboardHttpClient::default()
            .spawn(
                HttpRequest::submission_multipart(
                    format!("http://{address}/api/v1/submissions"),
                    Arc::from(&b"{\"schema_version\":1}"[..]),
                    exact_replay,
                    exact_campaign,
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(wait_for(&task).unwrap().status, 202);
        server_thread.join().unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn redirects_never_forward_replay_bytes() {
        let redirect_server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let redirect_address = redirect_server.server_addr().to_ip().unwrap();
        let sink_server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let sink_address = sink_server.server_addr().to_ip().unwrap();
        let redirect_thread = std::thread::spawn(move || {
            let request = redirect_server.recv().unwrap();
            let location = tiny_http::Header::from_bytes(
                &b"Location"[..],
                format!("http://{sink_address}/stolen").as_bytes(),
            )
            .unwrap();
            request
                .respond(tiny_http::Response::empty(307).with_header(location))
                .unwrap();
        });

        let task = LeaderboardHttpClient::default()
            .spawn(
                HttpRequest::submission_multipart(
                    format!("http://{redirect_address}/api/v1/submissions"),
                    Arc::from(&b"{}"[..]),
                    Arc::from(&b"canonical replay"[..]),
                    Arc::from(&b"starting campaign"[..]),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(wait_for(&task).unwrap().status, 307);
        redirect_thread.join().unwrap();
        assert!(
            sink_server
                .recv_timeout(std::time::Duration::from_millis(200))
                .unwrap()
                .is_none(),
            "redirect policy leaked the replay to the redirect target"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn timeout_and_streaming_body_limit_fail_closed() {
        let timeout_server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let timeout_address = timeout_server.server_addr().to_ip().unwrap();
        let (timeout_ready_tx, timeout_ready_rx) = std::sync::mpsc::sync_channel(0);
        let timeout_thread = std::thread::spawn(move || {
            timeout_ready_tx.send(()).unwrap();
            // Under scheduler load the client's 20 ms deadline can expire
            // before connecting. That is still the expected timeout result;
            // the fixture must not then wait forever for a request/join.
            if let Some(request) = timeout_server
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
            {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let _ = request.respond(tiny_http::Response::from_string("late"));
            }
        });
        timeout_ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("timeout fixture did not become ready");
        let task = LeaderboardHttpClient::new(1, 20)
            .unwrap()
            .spawn(HttpRequest::get_json(format!(
                "http://{timeout_address}/slow"
            )))
            .unwrap();
        assert_eq!(wait_for(&task), Err(HttpTransportError::Timeout));
        timeout_thread.join().unwrap();

        let body_server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let body_address = body_server.server_addr().to_ip().unwrap();
        let (body_ready_tx, body_ready_rx) = std::sync::mpsc::sync_channel(0);
        let body_thread = std::thread::spawn(move || {
            body_ready_tx.send(()).unwrap();
            body_server
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .expect("body-limit fixture received no request")
                .respond(tiny_http::Response::from_data(vec![0; 17]))
                .unwrap();
        });
        body_ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("body-limit fixture did not become ready");
        let mut request = HttpRequest::get_json(format!("http://{body_address}/large"));
        request.max_response_bytes = 16;
        let task = LeaderboardHttpClient::default().spawn(request).unwrap();
        assert_eq!(
            wait_for(&task),
            Err(HttpTransportError::ResponseTooLarge { limit: 16 })
        );
        body_thread.join().unwrap();
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn every_browser_request_refuses_redirects_and_credentials() {
        let controller = web_sys::AbortController::new().unwrap();
        let requests = [
            HttpRequest::get_json("/api/v1/leaderboard-metadata".to_owned()),
            HttpRequest::json(
                reqwest::Method::POST,
                "/api/v1/deletion-requests".to_owned(),
                br#"{"signature":"typed"}"#.to_vec(),
            ),
            HttpRequest::submission_multipart(
                "/api/v1/submissions".to_owned(),
                Arc::from(&b"{}"[..]),
                Arc::from(&b"canonical replay"[..]),
                Arc::from(&b"starting campaign"[..]),
            )
            .unwrap(),
        ];
        for request in requests {
            let request = build_browser_request(request, &controller.signal()).unwrap();
            assert_eq!(request.redirect(), web_sys::RequestRedirect::Error);
            assert_eq!(request.credentials(), web_sys::RequestCredentials::Omit);
        }
    }
}
