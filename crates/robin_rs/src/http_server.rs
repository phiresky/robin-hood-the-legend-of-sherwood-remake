//! Local script-RPC endpoint exposing the script VM, console, player
//! command pipeline, engine dump, decompiler, and per-frame
//! screenshot capture to external tools (debug shells, test harnesses,
//! AI drivers).
//!
//! Two transports share the same request/reply enums + per-tick drain:
//!
//! - **Native:** an application-owned Hyper listener on `127.0.0.1:<port>` (see
//!   [`HttpTransport::start`]).  Endpoints are:
//!
//!   | Method | Path                | Body / Query                                 | Response                                               |
//!   |--------|---------------------|----------------------------------------------|--------------------------------------------------------|
//!   | GET    | `/`                 | —                                            | endpoint listing                                       |
//!   | GET    | `/natives`          | —                                            | `{natives: [{index, name, return_type, params}]}`     |
//!   | GET    | `/engine-dump`      | —                                            | full serialized engine for ad-hoc debug                  |
//!   | GET    | `/level-assets`     | —                                            | level-scoped static assets for ad-hoc debug             |
//!   | GET    | `/script`           | —                                            | mission-script class & function listing                |
//!   | GET    | `/script/decompile` | `?class=<name>` (optional)                   | `{source: "..."}` — pseudocode for one or all classes  |
//!   | POST   | `/native`           | `{op, args, this?}`                          | `{return}` or `{error}`                                |
//!   | POST   | `/batch`            | `{calls: [{op, args, this?}]}`               | `{results: [...]}`                                     |
//!   | POST   | `/console`          | `{command: "..."}`                           | `{kind, message?}`                                     |
//!   | POST   | `/command`          | externally-tagged `PlayerCommand` JSON       | `{ok: true}` or `{error}`                              |
//!   | GET    | `/screenshot`       | `?frame=&full_map=&w=&h=&hide_ui=&…`          | `image/png` at or after the requested frame            |
//!
//! - **Wasm:** no loopback socket inside the browser.  Instead, the
//!   exported `rh_rpc({ method, params })` async function returns a JS
//!   Promise. The request lands on the same queue as the native
//!   transport and is drained on the game tick; JSON replies arrive as
//!   parsed JS values, and binary replies arrive as
//!   `{ contentType, data: Uint8Array }`.
//!
//! ### Threading (native)
//!
//! A dedicated listener thread owns a Tokio runtime and at most eight HTTP
//! connection tasks. Shutdown cancels and joins every connection task.
//! Each request is decoded into a [`HttpRequest`] and pushed onto a
//! shared FIFO with a single-owner asynchronous reply channel. The game
//! mission owner drains its queue once per tick and admits eligible requests
//! before execution. The listener awaits replies without polling and serialises them to JSON (or raw
//! image/png bytes for `/screenshot`).
//!
//! Requests requiring an engine fail immediately between missions. A busy
//! active mission can defer execution until its next RPC boundary, bounded
//! by a 60 s reply deadline on the
//! listener side.  Clients that want to fail fast instead of waiting
//! out a blocked main loop should pass a shorter HTTP timeout
//! themselves (e.g. `curl --max-time 2`).
//!
//! ### Screenshot pipeline
//!
//! `/screenshot` is special because it needs a rendered frame, not the
//! post-tick engine state.  The game loop:
//!
//! 1. [`SessionIngress::drain`] moves screenshot requests from the request
//!    queue into its mission-owned pending list.  **No mutation** of the
//!    live `Engine`, `DevState`, or any host state happens here.
//! 2. Before the live frame is rendered, the main loop calls
//!    [`SessionIngress::take_pending_screenshots`] and renders one throwaway frame
//!    per request into the offscreen target.  Each uses its own
//!    cloned `DevState` with flags applied via
//!    [`apply_screenshot_flags`] — the live `dev` is untouched.
//! 3. After each throwaway render the loop reads pixels back
//!    (`Renderer::begin_capture_frame_rgba`), hands completed pixels to
//!    [`PendingScreenshot::respond`] to reply with `image/png`. Submission
//!    consumes the queued commands; completion owns its independent readback.
//! 4. Finally the live frame is rendered and presented as normal.
//!
//! No authentication. Bind is `127.0.0.1` only. Pass `--http-server 0`
//! to disable the server entirely.

use crate::http_server::diagnostics::{
    decompile_script, engine_dump_json, frame_console_response_to_json, info_json,
    level_assets_json, list_natives_json, snapshot_host_debug, snapshot_script, snapshot_state,
};
pub use crate::http_server::screenshot::apply_screenshot_flags;
use crate::http_server::screenshot::{can_capture_presented_ui, encode_png};
use robin_engine::element as engine_element;
use robin_engine::engine as engine_api;
use robin_engine::player_command::{DialogResult, FrameCommands, ModalKind, PlayerCommand};
use robin_engine::replay_rankability::InputTaintKind;
#[cfg(all(test, feature = "script-rpc", not(target_arch = "wasm32")))]
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
use std::thread;
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
use std::time::Duration;

use robin_engine::engine::{Engine, LevelAssets};

/// Default port. Reasonably uncommon and easy to remember; change with
/// `--http-server <port>` or set 0 to disable.
pub const DEFAULT_PORT: u16 = 17640;

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HttpModalDismissal {
    pub kind: ModalKind,
    pub result: DialogResult,
}

/// Modal behavior attached to an HTTP step. Automation keeps the historical
/// auto-dismiss default, while deterministic drivers can set
/// `auto_dismiss=false` and supply exact typed outcomes.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct StepModalPolicy {
    #[serde(default = "default_true")]
    pub auto_dismiss: bool,
    pub dismissals: Vec<HttpModalDismissal>,
    /// Required for timeline movement in a live multiplayer session. Only the
    /// host accepts it and reconnects every peer from the resulting snapshot.
    pub synchronized_multiplayer: bool,
}

impl Default for StepModalPolicy {
    fn default() -> Self {
        Self {
            auto_dismiss: true,
            dismissals: Vec::new(),
            synchronized_multiplayer: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct StepRequest {
    pub n: u32,
    #[serde(flatten)]
    pub modal_policy: StepModalPolicy,
}

impl Default for StepRequest {
    fn default() -> Self {
        Self {
            n: 1,
            modal_policy: StepModalPolicy::default(),
        }
    }
}

/// One pending request waiting for the game tick.
pub struct HttpRequest {
    pub payload: HttpPayload,
    pub response_tx: Responder,
}

impl HttpRequest {
    fn admit_unless_deferred(&self) -> bool {
        if matches!(
            self.payload,
            HttpPayload::Screenshot(_)
                | HttpPayload::StepForward { .. }
                | HttpPayload::StepBack { .. }
                | HttpPayload::GoToFrame { .. }
                | HttpPayload::SetPaused { .. }
        ) {
            self.response_tx.eligible()
        } else {
            self.response_tx.admit()
        }
    }
}

/// Per-request payload — the transport layer parses each endpoint
/// down to one of these.  Distinct variants (rather than a generic
/// `serde_json::Value` body) keep the dispatch typed: classification
/// selects the authority needed by each handler.
pub enum HttpPayload {
    /// `POST /native` / `robin.call("native", …)` — single native invocation.
    Native {
        name: String,
        args: Vec<i32>,
        this: Option<i32>,
    },
    /// `POST /batch` — N natives in a row, all on the same tick.
    Batch(Vec<NativeCall>),
    /// `POST /console` — debug-console cheat / introspection.
    Console(String),
    /// `POST /command` — apply a PlayerCommand to the engine.
    Command(PlayerCommand),
    /// `GET /state` / `robin.call("state")` — compact frame/replay status.
    State,
    /// `GET /host-debug` / `robin.call("host-debug")` — host/UI-only state.
    HostDebug,
    /// `GET /engine-dump` — full serialized engine for ad-hoc debug.
    EngineDump,
    /// `GET /level-assets` — level-scoped static assets for ad-hoc debug.
    LevelAssets,
    /// `GET /script` — class/function listing for the mission script.
    Script,
    /// `GET /script/decompile?class=<name>` — pseudocode dump.
    Decompile { class: Option<String> },
    /// `GET /screenshot` — PNG capture of the next rendered frame.
    Screenshot(ScreenshotRequest),
    /// `POST /step-forward` — run `n` engine ticks synchronously.
    StepForward { request: StepRequest },
    /// `POST /step-back` — rewind `n` frames synchronously.
    StepBack { request: StepRequest },
    /// `POST /go-to-frame` — absolute simulation-frame seek in live play,
    /// or dense recording-ordinal seek during replay.
    GoToFrame {
        target: u32,
        modal_policy: StepModalPolicy,
    },
    /// `POST /set-paused` / `robin.call("set-paused", {paused})` —
    /// toggle the single-player mission loop's manual pause flag. Live
    /// multiplayer rejects local pause changes.
    SetPaused { paused: bool },
    /// `GET /get-replay` — snapshot the current recorder's byte stream and
    /// export the one canonical compact-bitcode replay. Served from a bounded
    /// in-memory spool populated by the recorder's tee-writer, so native and
    /// wasm use the same source without reading the filesystem.
    GetReplay,
    /// `POST /load-replay` — stash replay bytes + a `paused` flag into
    /// the replay service's pending launch slot that mission startup consumes
    /// on the next mission start.  The caller is responsible for
    /// triggering a mission restart (e.g. by sending a console command
    /// or by resetting the Game op) so the slot is actually picked up.
    LoadReplay { data: String, paused: bool },
}

/// Internal routing types are deliberately separate from the transport schema.
/// A query handler cannot receive a mutation or a deferred operation.
enum QueryRequest {
    State,
    EngineDump,
    LevelAssets,
    Script,
    Decompile { class: Option<String> },
}

enum CommandRequest {
    Native {
        name: String,
        args: Vec<i32>,
        this: Option<i32>,
    },
    Batch(Vec<NativeCall>),
    Console(String),
    Player(PlayerCommand),
}

enum DeferredRequest {
    Step(StepKind),
    Screenshot(ScreenshotRequest),
}

enum ProcessRequest {
    ExportReplay,
    LoadReplay { data: String, paused: bool },
}

enum RoutedRequest {
    Query(QueryRequest),
    HostDebug,
    Command(CommandRequest),
    Deferred(DeferredRequest),
    Process(ProcessRequest),
}

impl HttpPayload {
    fn classify(self) -> RoutedRequest {
        match self {
            Self::State => RoutedRequest::Query(QueryRequest::State),
            Self::HostDebug => RoutedRequest::HostDebug,
            Self::EngineDump => RoutedRequest::Query(QueryRequest::EngineDump),
            Self::LevelAssets => RoutedRequest::Query(QueryRequest::LevelAssets),
            Self::Script => RoutedRequest::Query(QueryRequest::Script),
            Self::Decompile { class } => RoutedRequest::Query(QueryRequest::Decompile { class }),
            Self::Native { name, args, this } => {
                RoutedRequest::Command(CommandRequest::Native { name, args, this })
            }
            Self::Batch(calls) => RoutedRequest::Command(CommandRequest::Batch(calls)),
            Self::Console(command) => RoutedRequest::Command(CommandRequest::Console(command)),
            Self::Command(command) => RoutedRequest::Command(CommandRequest::Player(command)),
            Self::Screenshot(request) => {
                RoutedRequest::Deferred(DeferredRequest::Screenshot(request))
            }
            Self::StepForward { request } => {
                RoutedRequest::Deferred(DeferredRequest::Step(StepKind::Forward {
                    n: request.n,
                    modal_policy: request.modal_policy,
                }))
            }
            Self::StepBack { request } => {
                RoutedRequest::Deferred(DeferredRequest::Step(StepKind::Back {
                    n: request.n,
                    modal_policy: request.modal_policy,
                }))
            }
            Self::GoToFrame {
                target,
                modal_policy,
            } => RoutedRequest::Deferred(DeferredRequest::Step(StepKind::GoToFrame {
                target,
                modal_policy,
            })),
            Self::SetPaused { paused } => {
                RoutedRequest::Deferred(DeferredRequest::Step(StepKind::SetPaused { paused }))
            }
            Self::GetReplay => RoutedRequest::Process(ProcessRequest::ExportReplay),
            Self::LoadReplay { data, paused } => {
                RoutedRequest::Process(ProcessRequest::LoadReplay { data, paused })
            }
        }
    }
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct ScreenshotRequest {
    /// Earliest absolute simulation frame at which to capture. `None`
    /// captures the next rendered frame.
    pub frame: Option<u32>,
    /// Output width bound. Used with `height` as an aspect-preserving maximum.
    pub width: Option<u16>,
    /// Output height bound. Used with `width` as an aspect-preserving maximum.
    pub height: Option<u16>,
    /// Omit all screen-space HUD drawing and crop the bottom panel area.
    pub hide_ui: bool,
    /// Capture the complete level at 1:1 map scale instead of the current
    /// viewport.
    pub full_map: bool,
    /// Debug-overlay overrides merged into the frame's `DevState` for
    /// this one render only.  Each `Some(x)` forces the corresponding
    /// `DebugFlags` field to `x`; `None` leaves it at the live value.
    pub flags: ScreenshotFlags,
}

/// Debug-overlay overrides for a single screenshot.  None of these
/// mutate the live `DevState`; they're merged into a `Cow<DevState>`
/// that exists only for the duration of one `render_frame` call.
#[derive(Clone, Default, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct ScreenshotFlags {
    pub view_cones: Option<bool>,
    pub pc_sight: Option<bool>,
    pub motion_graph: Option<bool>,
    pub surface: Option<bool>,
    pub all_obstacles: Option<bool>,
    pub elevation: Option<bool>,
    pub noise: Option<bool>,
    pub sound_source: Option<bool>,
    pub actor_info: Option<bool>,
    pub script_zones: Option<bool>,
    pub door: Option<bool>,
    pub projection_areas: Option<bool>,
    pub railroad: Option<bool>,
    pub probability: Option<bool>,
    pub company_number: Option<bool>,
    pub combat_energy: Option<bool>,
    pub light_zones: Option<bool>,
    pub animation_lines: Option<bool>,
    pub seek_points: Option<bool>,
    pub fps: Option<bool>,
    pub sprite_masks: Option<bool>,
    /// Rust-only dev overlay — draws each entity's numeric ID below its
    /// feet.  Useful for correlating `/state` entries with what is
    /// visible on screen.
    pub entity_ids: Option<bool>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct NativeCall {
    pub op: String,
    #[serde(default)]
    pub args: Vec<i32>,
    /// Optional transient `ThisActor` receiver for the call.
    #[serde(default, rename = "this")]
    pub this: Option<i32>,
}

/// Body of a successful reply.
///
/// Most endpoints return JSON; `/screenshot` returns raw `image/png`
/// bytes.  Kept as an enum rather than always-JSON so the screenshot
/// path doesn't pay a base64 tax.
pub enum ReplyBody {
    Json(serde_json::Value),
    /// Encoding is owned by the replay service; the transport awaits only the
    /// result, under the same deadline/retirement guard as the original request.
    ReplayExport(crate::replay_service::ExportResult),
    Binary {
        content_type: &'static str,
        data: Vec<u8>,
    },
}

impl From<serde_json::Value> for ReplyBody {
    fn from(v: serde_json::Value) -> Self {
        ReplyBody::Json(v)
    }
}

/// Reply the game loop sends back to the transport.
///
/// `Ok(body)` becomes a 200 with the matching Content-Type; `Err`
/// becomes a 400 with `{"error": msg}` (always JSON).
pub type Reply = Result<ReplyBody, RpcError>;

pub mod diagnostics;
pub mod query;
pub mod screenshot;

mod dispatch;
mod error;
#[cfg(all(test, feature = "script-rpc", not(target_arch = "wasm32")))]
use dispatch::dispatch_query;
use dispatch::start_replay_export;
pub use error::{RpcError, RpcErrorKind};

async fn resolve_deferred_reply(reply: Reply) -> Reply {
    match reply? {
        ReplyBody::ReplayExport(result) => result
            .recv()
            .await
            .map_err(|error| {
                RpcError::internal(format!("replay export worker dropped its result: {error}"))
            })?
            .map(|content| ReplyBody::Json(serde_json::json!({ "content": content })))
            .map_err(|error| match error {
                crate::replay_service::ExportError::Capacity(message) => {
                    RpcError::capacity(message)
                }
                crate::replay_service::ExportError::Retired(message) => RpcError::retired(message),
                crate::replay_service::ExportError::Unavailable(message) => {
                    RpcError::unavailable_capability(message)
                }
                crate::replay_service::ExportError::Internal(message) => {
                    RpcError::internal(message)
                }
            }),
        body => Ok(body),
    }
}

mod request_lifetime;
pub use request_lifetime::Responder;

mod ingress;
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
mod native_routes;
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
mod native_transport;
mod request_decode;
use ingress::RequestRouter;
pub use ingress::SessionIngress;
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
use native_transport::NativeRequest;

type Queue = Arc<Mutex<RequestRouter>>;

struct HttpServer {
    replay_exports: crate::replay_service::ReplayExports,
    replay_launches: crate::replay_service::ReplayLaunches,
    queue: Queue,
    #[cfg(all(test, feature = "script-rpc", not(target_arch = "wasm32")))]
    bind_addr: std::net::SocketAddr,
    #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
    listener: Option<thread::JoinHandle<()>>,
    #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Application-owned transport. Diagnostics cannot recreate a listener.
/// Stopping retires the queue and cancels and joins every native connection,
/// including stalled bodies and responses. Dropping performs the same teardown.
#[derive(Default, serde::Serialize)]
pub struct HttpTransport {
    port: Option<u16>,
    #[serde(skip)]
    server: Option<HttpServer>,
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpTransport")
            .field("port", &self.port)
            .finish()
    }
}

robin_util::deny_deserialize!(HttpTransport, "live HTTP transport cannot be deserialized");

#[cfg(target_arch = "wasm32")]
thread_local! {
    // JS needs one current entry point, not ownership of an application service.
    static BROWSER_QUEUE: std::cell::RefCell<std::sync::Weak<Mutex<RequestRouter>>> = const { std::cell::RefCell::new(std::sync::Weak::new()) };
}

impl HttpTransport {
    pub fn is_started(&self) -> bool {
        self.port.is_some()
    }

    pub fn matches_replay(
        &self,
        exports: &crate::replay_service::ReplayExports,
        launches: &crate::replay_service::ReplayLaunches,
    ) -> bool {
        self.server.as_ref().is_none_or(|server| {
            server.replay_exports.same_service(exports)
                && server.replay_launches.same_service(launches)
        })
    }

    pub fn attach(&self) -> SessionIngress {
        SessionIngress::attach(self.server.as_ref())
    }

    pub fn drain_pre_engine(&self) {
        if let Some(server) = &self.server {
            drain_pre_engine(server);
        }
    }

    pub fn stop(&mut self) {
        self.port = None;
        if let Some(server) = self.server.take() {
            #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
            let mut server = server;
            #[cfg(target_arch = "wasm32")]
            BROWSER_QUEUE.with(|binding| {
                let mut binding = binding.borrow_mut();
                if binding.ptr_eq(&Arc::downgrade(&server.queue)) {
                    *binding = std::sync::Weak::new();
                }
            });
            #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
            if let Some(stop) = server.stop.take() {
                let _ = stop.send(());
            }
            server.queue.lock().expect("RPC router poisoned").retire();
            #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
            if let Some(listener) = server.listener.take() {
                if listener.join().is_err() {
                    tracing::error!("script HTTP listener panicked during shutdown");
                }
            }
        }
    }
}

impl Drop for HttpTransport {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(all(test, not(feature = "script-rpc"), not(target_arch = "wasm32")))]
mod disabled_transport_tests {
    use super::*;

    #[test]
    fn native_listener_requires_feature_but_disabled_ingress_remains_usable() {
        let replay = crate::replay_service::ReplayService::default();
        let mut transport = HttpTransport::default();
        assert!(
            transport
                .start(DEFAULT_PORT, replay.exports(), replay.launches())
                .is_err()
        );
        assert!(!transport.is_started());
        transport
            .start(0, replay.exports(), replay.launches())
            .unwrap();
        let _ingress = transport.attach();
        transport.stop();
        assert!(!transport.is_started());
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod browser_transport_tests {
    use super::*;

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn stop_rejects_an_already_deferred_browser_promise_without_another_tick() {
        use futures::FutureExt as _;
        let replay = Arc::new(crate::replay_service::ReplayService::default());
        let mut transport = HttpTransport::default();
        transport
            .start(0, replay.exports(), replay.launches())
            .unwrap();
        let mut ingress = transport.attach();
        // Match the browser's plain JS object, not serde-wasm-bindgen's
        // default Map representation of serde_json::Value objects.
        let value =
            js_sys::JSON::parse(r#"{"method":"set-paused","params":{"paused":true}}"#).unwrap();
        let mut promise = Box::pin(wasm_rpc::rh_rpc(value));
        if let Some(reply) = promise.as_mut().now_or_never() {
            // wasm panic aborts without running Drop. Release the bridge before
            // reporting a bad fixture so one failure cannot contaminate tests.
            transport.stop();
            panic!("deferred RPC completed before mission dispatch: {reply:?}");
        }
        let request = ingress.take_requests().pop().expect("queued request");
        ingress.defer_request(
            DeferredRequest::Step(StepKind::SetPaused { paused: true }),
            request.response_tx,
            true,
        );
        transport.stop();
        assert_eq!(
            promise.await.unwrap_err().as_string().as_deref(),
            Some("HTTP transport stopped")
        );
        assert!(ingress.take_pending_steps().is_empty());
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn early_requests_survive_owner_transfer_and_stop_allows_rebinding() {
        let replay = Arc::new(crate::replay_service::ReplayService::default());
        let mut early = HttpTransport::default();
        early.start(0, replay.exports(), replay.launches()).unwrap();
        let queue = BROWSER_QUEUE
            .with(|binding| binding.borrow().upgrade())
            .unwrap();
        let (response_tx, _rx) = Responder::channel();
        queue.lock().unwrap().push_back(HttpRequest {
            payload: HttpPayload::LoadReplay {
                data: "early replay".into(),
                paused: true,
            },
            response_tx,
        });
        let mut application = early;
        // Native port options are irrelevant to the browser bridge binding.
        application
            .start(DEFAULT_PORT, replay.exports(), replay.launches())
            .unwrap();
        let mut mission = application.attach();
        assert_eq!(mission.take_requests().len(), 1);
        let mut replacement = HttpTransport::default();
        assert!(
            replacement
                .start(0, replay.exports(), replay.launches())
                .is_err()
        );
        application.stop();
        // The old mission and a caller still retain the old queue, but neither
        // can prevent a new application from taking over the JS entry point.
        replacement
            .start(0, replay.exports(), replay.launches())
            .unwrap();
        let current = BROWSER_QUEUE
            .with(|binding| binding.borrow().upgrade())
            .unwrap();
        assert!(!Arc::ptr_eq(&queue, &current));
    }
}

#[cfg(all(test, feature = "script-rpc", not(target_arch = "wasm32")))]
mod transport_lifecycle_tests {
    use super::*;
    use std::io::Write;

    fn running() -> (HttpTransport, Arc<crate::replay_service::ReplayService>) {
        let replay = Arc::new(crate::replay_service::ReplayService::default());
        let server =
            start(0, replay.exports(), replay.launches()).expect("ephemeral HTTP listener");
        let port = server.bind_addr.port();
        (
            HttpTransport {
                port: Some(port),
                server: Some(server),
            },
            replay,
        )
    }

    #[test]
    fn socket_disconnect_cancels_queued_and_deferred_requests() {
        for deferred in [false, true] {
            let (mut transport, _) = running();
            let port = transport.port.unwrap();
            let mut ingress = transport.attach();
            let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            let body = r#"{"paused":true}"#;
            write!(client, "POST /set-paused HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let request = loop {
                if let Some(request) = ingress.take_requests().pop() {
                    break request;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "socket request was not queued"
                );
                thread::yield_now();
            };
            let cancelled = request.response_tx.cancellation_observer();
            let queued = if deferred {
                ingress.defer_request(
                    DeferredRequest::Step(StepKind::SetPaused { paused: true }),
                    request.response_tx,
                    true,
                );
                None
            } else {
                Some(request)
            };
            client.shutdown(std::net::Shutdown::Both).unwrap();
            drop(client);
            // Observe transport cancellation itself, without driving a mission
            // tick or asking admission to notice a disconnected fixture.
            while !cancelled() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "Hyper retained the disconnected caller's reply future"
                );
                thread::yield_now();
            }
            if let Some(request) = queued {
                assert!(!request.response_tx.admit());
            }
            assert!(ingress.take_pending_steps().is_empty());
            transport.stop();
        }
    }

    #[test]
    fn stop_prevents_admission_after_inbox_extraction() {
        let (mut transport, _) = running();
        let queue = transport.server.as_ref().unwrap().queue.clone();
        let mut ingress = transport.attach();
        let (response_tx, _reply) = Responder::channel();
        queue.lock().unwrap().push_back(HttpRequest {
            payload: HttpPayload::Console("cheat".into()),
            response_tx: response_tx.with_router(&queue),
        });
        let request = ingress.take_requests().pop().unwrap();
        transport.stop();
        assert!(!request.admit_unless_deferred());
    }

    #[test]
    fn repeated_binding_checks_port_and_replay_authority_and_stop_releases_port() {
        let (mut transport, replay) = running();
        let port = transport.port.unwrap();
        transport
            .start(port, replay.exports(), replay.launches())
            .unwrap();
        assert!(
            transport
                .start(0, replay.exports(), replay.launches())
                .is_err()
        );
        let other = Arc::new(crate::replay_service::ReplayService::default());
        assert!(
            transport
                .start(port, other.exports(), replay.launches())
                .is_err()
        );
        assert!(
            transport
                .start(port, replay.exports(), other.launches())
                .is_err()
        );
        transport.stop();
        assert!(!transport.is_started());
        transport
            .start(port, other.exports(), other.launches())
            .expect("rebind with new authority");
        assert!(transport.matches_replay(&other.exports(), &other.launches()));
    }

    #[test]
    fn stop_cancels_a_reply_already_deferred_by_the_mission() {
        let (mut transport, _) = running();
        let queue = transport.server.as_ref().unwrap().queue.clone();
        let mut ingress = transport.attach();
        let worker = thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(relay(&queue, HttpPayload::SetPaused { paused: true }))
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(request) = ingress.take_requests().pop() {
                // Keep the responder alive in the mission's deferred queue.
                ingress.defer_request(
                    DeferredRequest::Step(StepKind::SetPaused { paused: true }),
                    request.response_tx,
                    true,
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "request did not reach ingress"
            );
            thread::yield_now();
        }
        transport.stop();
        assert_eq!(worker.join().unwrap().0, 400);
    }

    #[test]
    fn repeated_start_reports_a_listener_that_has_exited() {
        let (mut transport, replay) = running();
        let port = transport.port.unwrap();
        transport
            .server
            .as_mut()
            .unwrap()
            .stop
            .take()
            .unwrap()
            .send(())
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !transport
            .server
            .as_ref()
            .unwrap()
            .listener
            .as_ref()
            .unwrap()
            .is_finished()
        {
            assert!(
                std::time::Instant::now() < deadline,
                "listener did not exit"
            );
            thread::yield_now();
        }
        assert!(
            transport
                .start(port, replay.exports(), replay.launches())
                .unwrap_err()
                .contains("exited")
        );
        transport.stop();
        transport
            .start(port, replay.exports(), replay.launches())
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stop_rebinds_after_a_completed_http_connection() {
        use std::io::Read;
        let (mut transport, replay) = running();
        let port = transport.port.unwrap();
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(
            client,
            "GET /info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        transport.stop();
        transport
            .start(port, replay.exports(), replay.launches())
            .expect("rebind despite TIME_WAIT");
    }

    #[test]
    fn shutdown_does_not_wait_forever_for_an_incomplete_request_body() {
        let (mut transport, replay) = running();
        let port = transport.port.unwrap();
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(client, "POST /console HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 100\r\nContent-Type: application/json\r\n\r\n{{").unwrap();
        // Allow the connection task to begin acquiring the incomplete body.
        thread::sleep(Duration::from_millis(50));
        let before = std::time::Instant::now();
        transport.stop();
        assert!(before.elapsed() < Duration::from_secs(2));
        transport
            .start(port, replay.exports(), replay.launches())
            .expect("listener released after partial body");
    }

    #[test]
    fn shutdown_cancels_a_continuously_trickled_body() {
        let (mut transport, _) = running();
        let port = transport.port.unwrap();
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write!(
            client,
            "POST /console HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 1000000\r\n\r\n{{"
        )
        .unwrap();
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer_finished = finished.clone();
        let writer = thread::spawn(move || {
            while !writer_finished.load(std::sync::atomic::Ordering::Acquire) {
                if client.write_all(b" ").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        thread::sleep(Duration::from_millis(50));
        let before = std::time::Instant::now();
        transport.stop();
        let elapsed = before.elapsed();
        finished.store(true, std::sync::atomic::Ordering::Release);
        writer.join().unwrap();
        assert!(elapsed < Duration::from_secs(2));
    }

    #[test]
    fn native_transport_preserves_security_and_early_replay_header_rejection() {
        use std::io::Read;
        let (transport, _) = running();
        let port = transport.port.unwrap();
        let exchange = |request: String| {
            let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client.write_all(request.as_bytes()).unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            response
        };
        assert!(
            exchange(format!(
                "GET /info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            ))
            .starts_with("HTTP/1.1 200")
        );
        assert!(exchange(format!("GET /info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: https://example.com\r\nConnection: close\r\n\r\n")).starts_with("HTTP/1.1 403"));
        assert!(
            exchange(
                "GET /info HTTP/1.1\r\nHost: attacker.example\r\nConnection: close\r\n\r\n".into()
            )
            .starts_with("HTTP/1.1 403")
        );
        // No chunk/body follows: validation must reject from headers alone.
        let response = exchange(format!(
            "POST /load-replay HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
        ));
        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(response.contains("Transfer-Encoding"));
    }

    #[test]
    fn native_query_validation_happens_before_mission_admission() {
        use std::io::Read;
        let (transport, _) = running();
        let port = transport.port.unwrap();
        let mut ingress = transport.attach();
        for path in [
            "/screenshot?frame=bad",
            "/screenshot?view_cones=maybe",
            "/screenshot?frame=1&frame=2",
            "/script/decompile?class=%FF",
        ] {
            let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(
                client,
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 400"), "{response}");
            assert!(response.contains("\"error\""));
            assert!(
                ingress.take_requests().is_empty(),
                "invalid query reached the mission"
            );
        }
        for path in [
            "/screenshot?view_cones&frame=12",
            "/script/decompile?class=Guard%20A%2BB",
        ] {
            let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(
                client,
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let request = loop {
                if let Some(request) = ingress.take_requests().pop() {
                    break request;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "valid query was not queued"
                );
                thread::yield_now();
            };
            match &request.payload {
                HttpPayload::Screenshot(value) => {
                    assert_eq!(value.frame, Some(12));
                    assert_eq!(value.flags.view_cones, Some(true));
                }
                HttpPayload::Decompile { class } => assert_eq!(class.as_deref(), Some("Guard A+B")),
                _ => panic!("unexpected query payload"),
            }
            assert!(request.response_tx.admit());
            request
                .response_tx
                .send(Ok(serde_json::json!({"ok": true}).into()));
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        }
    }
}
fn ranked_input_taint(payload: &HttpPayload) -> Option<InputTaintKind> {
    match payload {
        HttpPayload::Native { .. } | HttpPayload::Batch(_) => {
            Some(InputTaintKind::HttpStateMutation)
        }
        HttpPayload::Console(_) => Some(InputTaintKind::ConsoleCommand),
        HttpPayload::Command(_) => Some(InputTaintKind::HttpPlayerCommand),
        HttpPayload::StepForward { .. }
        | HttpPayload::StepBack { .. }
        | HttpPayload::GoToFrame { .. }
        | HttpPayload::SetPaused { .. } => Some(InputTaintKind::HttpSimulationStep),
        HttpPayload::LoadReplay { .. } => Some(InputTaintKind::ReplayPlayback),
        HttpPayload::State
        | HttpPayload::HostDebug
        | HttpPayload::EngineDump
        | HttpPayload::LevelAssets
        | HttpPayload::Script
        | HttpPayload::Decompile { .. }
        | HttpPayload::Screenshot(_)
        | HttpPayload::GetReplay => None,
    }
}

/// Start this application's script-RPC listener. Repeated starts must match
/// both the binding and replay capabilities; stop explicitly before rebinding.
///
/// Native: binds a loopback HTTP listener on `port` (0 disables).
/// Wasm: ignores `port`; just installs the empty queue so `rh_rpc`
/// has somewhere to push.
impl HttpTransport {
    pub fn start(
        &mut self,
        port: u16,
        replay_exports: crate::replay_service::ReplayExports,
        replay_launches: crate::replay_service::ReplayLaunches,
    ) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        let port = {
            let _ = port;
            0
        };
        if let Some(bound_port) = self.port {
            #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
            if self
                .server
                .as_ref()
                .and_then(|server| server.listener.as_ref())
                .is_some_and(thread::JoinHandle::is_finished)
            {
                return Err("HTTP listener has exited; stop it before restarting".into());
            }
            return if bound_port == port && self.matches_replay(&replay_exports, &replay_launches) {
                Ok(())
            } else {
                Err(
                    "HTTP transport already initialized with a different port or replay authority"
                        .into(),
                )
            };
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = port;
            let queue = Arc::new(Mutex::new(RequestRouter::default()));
            BROWSER_QUEUE.with(|binding| {
                let mut binding = binding.borrow_mut();
                if binding.upgrade().is_some() {
                    return Err("another application owns the browser RPC bridge".to_owned());
                }
                *binding = Arc::downgrade(&queue);
                Ok(())
            })?;
            self.server = Some(HttpServer {
                replay_exports,
                replay_launches,
                queue,
            });
            self.port = Some(port);
            tracing::info!("script RPC: wasm bridge ready (rh_rpc)");
            Ok(())
        }
        #[cfg(all(not(feature = "script-rpc"), not(target_arch = "wasm32")))]
        {
            let _ = (replay_exports, replay_launches);
            if port != 0 {
                return Err("native HTTP transport requires the script-rpc feature".into());
            }
            self.port = Some(0);
            Ok(())
        }
        #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
        {
            if port == 0 {
                self.port = Some(port);
                tracing::info!("script HTTP server: disabled (--http-server 0)");
                return Ok(());
            }
            let server = start(port, replay_exports, replay_launches)?;
            self.server = Some(server);
            self.port = Some(port);
            Ok(())
        }
    }
}

#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
fn start(
    port: u16,
    replay_exports: crate::replay_service::ReplayExports,
    replay_launches: crate::replay_service::ReplayLaunches,
) -> Result<HttpServer, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("HTTP runtime: {e}"))?;
    let listener = {
        let _entered = runtime.enter();
        let socket = tokio::net::TcpSocket::new_v4().map_err(|e| format!("HTTP socket: {e}"))?;
        // Reuse a stopped listener's address after accepted connections enter
        // TIME_WAIT. Do not enable Windows SO_REUSEADDR's port-sharing semantics.
        #[cfg(unix)]
        socket
            .set_reuseaddr(true)
            .map_err(|e| format!("HTTP socket reuse: {e}"))?;
        socket.bind(std::net::SocketAddr::from(([127, 0, 0, 1], port))).map_err(|e| {
            format!("script HTTP server failed to bind 127.0.0.1:{port}: {e} (another robin instance? pass `--http-server 0` to disable, or `--http-server <port>` to pick a different port)")
        })?;
        socket
            .listen(128)
            .map_err(|e| format!("HTTP listen: {e}"))?
    };
    let bind_addr = listener
        .local_addr()
        .map_err(|e| format!("HTTP listener address: {e}"))?;
    tracing::info!("script HTTP server listening on http://{bind_addr}");

    let queue: Queue = Arc::new(Mutex::new(RequestRouter::default()));
    let queue_for_thread = queue.clone();
    let (stop, listener_stop) = tokio::sync::oneshot::channel();
    let listener = thread::Builder::new()
        .name("robin-http-server".into())
        .spawn(move || {
            runtime.block_on(native_transport::run(
                listener,
                queue_for_thread,
                listener_stop,
                bind_addr.port(),
            ))
        })
        .map_err(|e| format!("script HTTP server: failed to spawn listener thread: {e}"))?;
    Ok(HttpServer {
        queue,
        #[cfg(test)]
        bind_addr,
        replay_exports,
        replay_launches,
        listener: Some(listener),
        stop: Some(stop),
    })
}

/// Reject requests a browser could have issued cross-origin.
///
/// The POST endpoints are unauthenticated and reachable via
/// no-preflight `POST` from any web page, so a malicious page could
/// drive the running game (CSRF). Non-browser clients (curl, scripts)
/// never send `Origin` / `Sec-Fetch-Site` and send an exact local
/// `Host`, so they pass untouched. Returns a rejection reason, or
/// `None` when the request is acceptable.
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
fn browser_rejection_reason(
    origin: Option<&str>,
    sec_fetch_site: Option<&str>,
    host: Option<&str>,
    port: u16,
) -> Option<&'static str> {
    // Browsers attach `Origin` to every cross-origin (and most
    // same-origin) fetches; no legitimate client of this API is a web
    // page, so any `Origin` at all is rejected.
    if origin.is_some() {
        return Some("cross-origin requests are not allowed (Origin header present)");
    }
    match sec_fetch_site {
        None | Some("none") | Some("same-origin") => {}
        Some(_) => return Some("cross-site request blocked (Sec-Fetch-Site)"),
    }
    // Anti DNS-rebinding: the Host header must name this loopback server.
    let host_ok = host.is_some_and(|h| {
        let h = h.to_ascii_lowercase();
        h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}")
    });
    if !host_ok {
        return Some("request rejected: Host header does not match the local server");
    }
    None
}

/// Send a payload to the game loop and wait for the reply.  Caps the
/// wait at 60 s so a wedged game doesn't hang the client forever.
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
async fn relay(queue: &Queue, payload: HttpPayload) -> (u16, ReplyBody) {
    let (response_tx, rx) = Responder::channel();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let retirement = queue
        .lock()
        .expect("RPC router poisoned")
        .retirement_receiver();
    queue
        .lock()
        .expect("queue mutex poisoned")
        .push_back(HttpRequest {
            payload,
            response_tx: response_tx.with_router(queue).with_deadline(deadline),
        });
    let reply = tokio::select! {
        biased;
        _ = retirement.recv() => return (400, RpcError::retired("HTTP transport stopped").wire_body().into()),
        reply = async {
            match rx.recv().await {
                Ok(reply) => Ok(resolve_deferred_reply(reply).await),
                Err(error) => Err(error),
            }
        } => reply,
        _ = tokio::time::sleep_until(deadline.into()) => {
            rx.expire();
            return (504, RpcError::deadline("game loop did not process the request within 60s; already-admitted work may complete").wire_body().into());
        },
    };
    match reply {
        Ok(Ok(body)) => (200, body),
        Ok(Err(error)) => (400, error.wire_body().into()),
        Err(_) if rx.is_expired() => (
            504,
            RpcError::deadline("request expired before admission")
                .wire_body()
                .into(),
        ),
        Err(_) => (
            500,
            RpcError::internal("game loop dropped the response channel")
                .wire_body()
                .into(),
        ),
    }
}

/// Drain process requests while no mission is active.
fn drain_pre_engine(server: &HttpServer) {
    let pending = {
        let mut q = server.queue.lock().expect("queue mutex poisoned");
        q.take_idle()
    };
    for req in pending {
        if !req.response_tx.admit() {
            continue;
        }
        match req.payload {
            HttpPayload::GetReplay => start_replay_export(&server.replay_exports, req.response_tx),
            HttpPayload::LoadReplay { data, paused } => {
                let reply = decode_load_replay(&server.replay_launches, &data, paused);
                req.response_tx.send(reply);
            }
            _ => req.response_tx.send(Err(RpcError::unavailable_capability(
                "engine not ready — only `load-replay`, `get-replay`, and `info` work during --wait-for-command"
            ))),
        }
    }
}

/// Parse a production replay payload and admit it to the pending slot. Both
/// browser and native RPC accept exactly the canonical compact envelope.
fn decode_load_replay(
    launches: &crate::replay_service::ReplayLaunches,
    data: &str,
    paused: bool,
) -> Reply {
    // Compact admission is byte-canonical: whitespace is not discarded.
    // Local JSONL tooling likewise emits its header at byte zero.
    let trimmed = data;
    let replay = crate::replay_format::decode_compact_for_public_playback(trimmed)
        .map(|(_, replay)| replay)
        .map_err(|error| RpcError::replay_load("decode compact replay", error))?;
    let frame_count = replay.frame_count();
    let seed = replay.header().rng_seed;
    launches
        .admit_pending(crate::replay_service::PendingReplay {
            data: replay,
            paused,
        })
        .map_err(RpcError::capacity)?;
    Ok(ReplyBody::Json(serde_json::json!({
        "ok": true,
        "frames": frame_count,
        "rng_seed": seed,
        "paused": paused,
        "note": "pending — takes effect on next mission init (restart mission to apply)",
    })))
}

/// Per-frame replay-playback status surfaced to the script-RPC
/// `state` endpoint so JS timeline UIs can render a playhead without
/// polling a dedicated endpoint.  `None` when no replay is playing
/// (live gameplay). Updated on the owning mission's manual-step boundary.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct ReplayStatus {
    pub frame: u32,
    pub total: u32,
    pub paused: bool,
}

// ──────────────────────────────────────────────────────────────────
// Screenshot pipeline
// ──────────────────────────────────────────────────────────────────

/// A screenshot request waiting for the next rendered frame.
///
/// The caller (the main loop) is expected to:
/// 1. Clone the live `DevState` and feed the per-request
///    [`ScreenshotFlags`] through [`apply_screenshot_flags`].
/// 2. Render a throwaway frame with that dev clone into the offscreen
///    target.
/// 3. Submit readback (`Renderer::begin_capture_frame_rgba`) and retain the
///    future and this responder in the mission's bounded capture queue.
/// 4. Consume this struct via [`PendingScreenshot::respond`], handing
///    over the pixels so the request replies with `image/png`.
/// Submission clears recorded commands; completion never borrows the live
/// renderer. Ending the mission retires outstanding replies and readbacks.
pub struct PendingScreenshot {
    response_tx: Responder,
    request: ScreenshotRequest,
}

impl PendingScreenshot {
    /// Full screenshot options shared by viewport and full-map captures.
    pub fn request(&self) -> &ScreenshotRequest {
        &self.request
    }

    /// Encode the captured RGBA frame as PNG (applying the request's
    /// optional crop + resize) and send the reply to the HTTP client.
    /// Consumes `self` — callers get one shot.
    pub fn respond(self, src_w: u32, src_h: u32, rgba: &[u8]) {
        let reply = encode_png(src_w, src_h, rgba, &self.request);
        self.response_tx.send(reply);
    }

    /// Reply with an error string instead of a PNG (e.g. when pixel
    /// readback failed).  Consumes `self`.
    pub fn respond_err(self, error: RpcError) {
        self.response_tx.send(Err(error));
    }
}

// ──────────────────────────────────────────────────────────────────
// Step-forward / step-back pipeline
// ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepKind {
    /// Run `n` ticks forward from the current frame.
    Forward {
        n: u32,
        modal_policy: StepModalPolicy,
    },
    /// Rewind `n` frames from the current frame.
    Back {
        n: u32,
        modal_policy: StepModalPolicy,
    },
    /// Absolute simulation-frame seek in live play; dense recording-ordinal
    /// seek during replay, so reloads and stationary records remain addressable.
    GoToFrame {
        target: u32,
        modal_policy: StepModalPolicy,
    },
    /// Toggle the single-player mission loop's manual pause flag. Queued with
    /// scrubbing so pause/play and seek requests apply in caller order; live
    /// multiplayer rejects this instead of desynchronizing one peer.
    SetPaused { paused: bool },
}

/// A step-forward / step-back request waiting for the main loop to
/// drive the engine.  The main loop is expected to
/// [`SessionIngress::take_pending_steps`] once per frame and, for each request, either:
///
/// - run `n` full frame-equivalent ticks (`Forward`), or
/// - rewind `n` frames through the rewind buffer (`Back`),
///
/// then reply via [`PendingStep::respond_ok`] /
/// [`PendingStep::respond_err`].  Refuse to run when the game has
/// modal state queued (dialog / briefing / scroll) — advancing the
/// sim while a modal is pending would skip past the modal.
pub struct PendingStep {
    response_tx: Responder,
    pub kind: StepKind,
}

impl PendingStep {
    pub fn respond_ok(self, body: serde_json::Value) {
        self.response_tx.send(Ok(ReplyBody::Json(body)));
    }

    pub fn respond_err(self, error: RpcError) {
        self.response_tx.send(Err(error));
    }
}

#[cfg(all(test, feature = "script-rpc", not(target_arch = "wasm32")))]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn shared_dispatch_preserves_admission_commands_queries_and_capability_differences() {
        for graphical in [false, true] {
            let mut assets = LevelAssets::new();
            let mut engine = Engine::new_for_test(1024.0, 768.0, Default::default(), &mut assets)
                .expect("RPC fixture engine");
            let mut host = crate::host::Host::scratch(1024.0, 768.0);
            let mut ingress = SessionIngress::detached_for_test();
            // Cancellation must occur before taint accounting or mutation.
            drop(ingress.enqueue_for_test(HttpPayload::Console("UNBLIP".into())));
            let state = ingress.enqueue_for_test(HttpPayload::State);
            let command = ingress.enqueue_for_test(HttpPayload::Command(PlayerCommand::CrouchDown));
            let debug = ingress.enqueue_for_test(HttpPayload::HostDebug);
            let screenshot =
                ingress.enqueue_for_test(HttpPayload::Screenshot(ScreenshotRequest::default()));
            let step = ingress.enqueue_for_test(HttpPayload::StepForward {
                request: StepRequest::default(),
            });
            let process = ingress.enqueue_for_test(HttpPayload::GetReplay);
            let mut selected = None;
            let commands = if graphical {
                let mut commands = FrameCommands::new();
                let external = ingress.drain(
                    &mut engine,
                    &mut host.frontend,
                    robin_engine::player_command::PlayerId::HOST,
                    None,
                    &assets,
                    &mut commands,
                );
                assert!(external.is_empty());
                commands
            } else {
                ingress.drain_headless(&mut engine, &assets, &mut selected)
            };
            assert_eq!(commands.commands.len(), 1);
            assert_eq!(
                commands.commands[0].player_id,
                robin_engine::player_command::PlayerId::HOST
            );
            assert!(matches!(
                commands.commands[0].command,
                PlayerCommand::CrouchDown
            ));
            assert!(
                matches!(state.try_recv().unwrap(), Ok(ReplyBody::Json(value)) if value["frame"] == engine.frame_counter())
            );
            assert!(
                matches!(command.try_recv().unwrap(), Ok(ReplyBody::Json(value)) if value == serde_json::json!({"ok": true}))
            );
            let debug_reply = debug.try_recv().unwrap();
            if graphical {
                assert!(matches!(debug_reply, Ok(ReplyBody::Json(_))));
                assert!(
                    screenshot.try_recv().is_err(),
                    "graphical capture remains deferred"
                );
                assert_eq!(
                    ingress
                        .take_pending_screenshots(engine.frame_counter())
                        .len(),
                    1
                );
            } else {
                assert!(
                    matches!(debug_reply, Err(error) if error.kind == RpcErrorKind::UnavailableCapability && error.message == "host-debug is unavailable in a headless runner")
                );
                assert!(
                    matches!(screenshot.try_recv().unwrap(), Err(error) if error.kind == RpcErrorKind::UnavailableCapability)
                );
            }
            assert!(step.try_recv().is_err());
            assert_eq!(ingress.take_pending_steps().len(), 1);
            assert!(
                matches!(process.try_recv().unwrap(), Err(error) if error.kind == RpcErrorKind::UnavailableCapability)
            );
            assert_eq!(
                ingress.take_pending_replay_taints(),
                BTreeSet::from([
                    InputTaintKind::HttpPlayerCommand,
                    InputTaintKind::HttpSimulationStep,
                ])
            );
        }
    }

    #[test]
    fn headless_diagnostic_policy_omits_rng_without_mutating_live_engine() {
        for graphical in [false, true] {
            let mut assets = LevelAssets::new();
            let mut engine =
                Engine::new_for_test(1024.0, 768.0, Default::default(), &mut assets).unwrap();
            engine = Engine::new(engine_api::EngineArgs {
                campaign: engine.campaign().clone(),
                level: engine_api::LevelLoadArgs {
                    assets: &mut assets,
                    level_directory: "",
                    progress: &mut |_| {},
                    loaded: robin_engine::level_data::LoadedLevel::empty_for_test(),
                    bg_pixel_dims: (0.0, 0.0),
                },
                ground_mark_sprite: None,
                titbit_row_frame_counts: Vec::new(),
                rng_seed: 0,
                original_rng_replay: Some(vec![11, 22]),
                sim_config: engine_api::SimConfig {
                    script_enabled: false,
                    ..Default::default()
                },
            })
            .expect("original RNG diagnostic fixture");
            let rng_cursor = engine.original_rng_replay_cursor();
            assert!(rng_cursor.is_some());
            let mut ingress = SessionIngress::detached_for_test();
            let dump = ingress.enqueue_for_test(HttpPayload::EngineDump);
            if graphical {
                let mut host = crate::host::Host::scratch(1024.0, 768.0);
                ingress.drain(
                    &mut engine,
                    &mut host.frontend,
                    robin_engine::player_command::PlayerId::HOST,
                    None,
                    &assets,
                    &mut FrameCommands::new(),
                );
                assert!(
                    matches!(dump.try_recv().unwrap(), Err(error) if error.kind == RpcErrorKind::Internal && error.message.starts_with("engine serialize:"))
                );
            } else {
                ingress.drain_headless(&mut engine, &assets, &mut None);
                assert!(matches!(dump.try_recv().unwrap(), Ok(ReplyBody::Json(_))));
            }
            assert_eq!(engine.original_rng_replay_cursor(), rng_cursor);
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn query_dispatch_has_only_read_authority() {
        // Function-pointer coercion is a compile-time capability proof: adding
        // mutable state, frontend input, or command sinks breaks this contract.
        let _: fn(QueryRequest, Option<ReplayStatus>, &Engine, &LevelAssets) -> Reply =
            dispatch_query;
        let _: fn(
            &Engine,
            &crate::host::HostFrontend,
            robin_engine::player_command::PlayerId,
            &LevelAssets,
        ) -> serde_json::Value = snapshot_host_debug;
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn transport_classification_separates_authority_without_losing_arguments() {
        for payload in [
            HttpPayload::State,
            HttpPayload::EngineDump,
            HttpPayload::LevelAssets,
            HttpPayload::Script,
        ] {
            assert!(matches!(payload.classify(), RoutedRequest::Query(_)));
        }
        assert!(matches!(
            HttpPayload::HostDebug.classify(),
            RoutedRequest::HostDebug
        ));
        assert!(
            matches!(HttpPayload::Decompile { class: Some("Mission".into()) }.classify(), RoutedRequest::Query(QueryRequest::Decompile { class: Some(class) }) if class == "Mission")
        );
        assert!(
            matches!(HttpPayload::Native { name: "test".into(), args: vec![1, -2], this: Some(3) }.classify(), RoutedRequest::Command(CommandRequest::Native { name, args, this: Some(3) }) if name == "test" && args == [1, -2])
        );
        assert!(
            matches!(HttpPayload::Console("UNBLIP".into()).classify(), RoutedRequest::Command(CommandRequest::Console(command)) if command == "UNBLIP")
        );
        assert!(matches!(
            HttpPayload::Command(PlayerCommand::CrouchDown).classify(),
            RoutedRequest::Command(CommandRequest::Player(PlayerCommand::CrouchDown))
        ));
        assert!(
            matches!(HttpPayload::Batch(vec![]).classify(), RoutedRequest::Command(CommandRequest::Batch(calls)) if calls.is_empty())
        );
        assert!(matches!(
            HttpPayload::GetReplay.classify(),
            RoutedRequest::Process(ProcessRequest::ExportReplay)
        ));
        assert!(
            matches!(HttpPayload::LoadReplay { data: "encoded".into(), paused: true }.classify(), RoutedRequest::Process(ProcessRequest::LoadReplay { data, paused: true }) if data == "encoded")
        );
    }

    #[test]
    fn step_request_defaults_to_one_tick_and_auto_dismiss() {
        let request: StepRequest = serde_json::from_value(serde_json::json!({}))
            .expect("empty step request uses documented defaults");
        assert_eq!(request, StepRequest::default());
        assert_eq!(request.n, 1);
        assert!(request.modal_policy.auto_dismiss);
        assert!(!request.modal_policy.synchronized_multiplayer);
    }

    #[test]
    fn step_request_requires_explicit_multiplayer_synchronization() {
        let request: StepRequest = serde_json::from_value(serde_json::json!({
            "n": 2,
            "synchronized_multiplayer": true,
        }))
        .expect("explicit multiplayer step policy");
        assert!(request.modal_policy.synchronized_multiplayer);
    }

    #[test]
    fn step_request_decodes_typed_modal_outcomes() {
        let dismissal = HttpModalDismissal {
            kind: ModalKind::Dialog { dialog_id: 17 },
            result: DialogResult::Aborted,
        };
        let request: StepRequest = serde_json::from_value(serde_json::json!({
            "n": 4,
            "auto_dismiss": false,
            "dismissals": [serde_json::to_value(&dismissal).expect("dismissal JSON")],
        }))
        .expect("typed step request");
        assert_eq!(request.n, 4);
        assert!(!request.modal_policy.auto_dismiss);
        assert_eq!(request.modal_policy.dismissals, vec![dismissal]);
    }

    #[test]
    fn ranked_input_taints_cover_every_mutating_automation_lane() {
        let cases = vec![
            (
                HttpPayload::Command(PlayerCommand::CrouchDown),
                InputTaintKind::HttpPlayerCommand,
            ),
            (
                HttpPayload::StepForward {
                    request: StepRequest::default(),
                },
                InputTaintKind::HttpSimulationStep,
            ),
            (
                HttpPayload::StepBack {
                    request: StepRequest::default(),
                },
                InputTaintKind::HttpSimulationStep,
            ),
            (
                HttpPayload::GoToFrame {
                    target: 20,
                    modal_policy: StepModalPolicy::default(),
                },
                InputTaintKind::HttpSimulationStep,
            ),
            (
                HttpPayload::SetPaused { paused: true },
                InputTaintKind::HttpSimulationStep,
            ),
            (
                HttpPayload::Native {
                    name: "SetMoney".into(),
                    args: vec![100],
                    this: None,
                },
                InputTaintKind::HttpStateMutation,
            ),
            (
                HttpPayload::Batch(vec![NativeCall {
                    op: "SetMoney".into(),
                    args: vec![100],
                    this: None,
                }]),
                InputTaintKind::HttpStateMutation,
            ),
            (
                HttpPayload::Console("UNBLIP".into()),
                InputTaintKind::ConsoleCommand,
            ),
            (
                HttpPayload::LoadReplay {
                    data: "rhrec-fixture".into(),
                    paused: false,
                },
                InputTaintKind::ReplayPlayback,
            ),
        ];
        let mut ingress = SessionIngress::detached_for_test();
        for (payload, expected) in &cases {
            assert_eq!(ranked_input_taint(payload), Some(*expected));
            ingress.observe_ranked_input_taint(payload);
        }
        assert_eq!(
            ingress.take_pending_replay_taints(),
            BTreeSet::from([
                InputTaintKind::HttpPlayerCommand,
                InputTaintKind::HttpSimulationStep,
                InputTaintKind::HttpStateMutation,
                InputTaintKind::ConsoleCommand,
                InputTaintKind::ReplayPlayback,
            ]),
            "the exact ingress hook must retain every observed mutating lane"
        );
        assert_eq!(ranked_input_taint(&HttpPayload::State), None);
        assert_eq!(ranked_input_taint(&HttpPayload::GetReplay), None);
    }

    #[test]
    fn browser_guard_rejects_any_origin_header() {
        assert!(
            browser_rejection_reason(
                Some("https://evil.example"),
                None,
                Some("127.0.0.1:17640"),
                17640
            )
            .is_some()
        );
        // Even a "local-looking" Origin is rejected: no browser client exists.
        assert!(
            browser_rejection_reason(
                Some("http://127.0.0.1:17640"),
                None,
                Some("127.0.0.1:17640"),
                17640
            )
            .is_some()
        );
    }

    #[test]
    fn browser_guard_rejects_cross_site_fetch_metadata() {
        assert!(
            browser_rejection_reason(None, Some("cross-site"), Some("127.0.0.1:17640"), 17640)
                .is_some()
        );
        assert!(
            browser_rejection_reason(None, Some("same-site"), Some("127.0.0.1:17640"), 17640)
                .is_some()
        );
        assert!(
            browser_rejection_reason(None, Some("none"), Some("127.0.0.1:17640"), 17640).is_none()
        );
        assert!(
            browser_rejection_reason(None, Some("same-origin"), Some("127.0.0.1:17640"), 17640)
                .is_none()
        );
    }

    #[test]
    fn browser_guard_rejects_foreign_or_missing_host() {
        assert!(
            browser_rejection_reason(None, None, Some("attacker.example:17640"), 17640).is_some()
        );
        assert!(browser_rejection_reason(None, None, Some("127.0.0.1:9999"), 17640).is_some());
        assert!(browser_rejection_reason(None, None, None, 17640).is_some());
        // The curl default workflow keeps working.
        assert!(browser_rejection_reason(None, None, Some("127.0.0.1:17640"), 17640).is_none());
        assert!(browser_rejection_reason(None, None, Some("localhost:17640"), 17640).is_none());
        assert!(browser_rejection_reason(None, None, Some("LOCALHOST:17640"), 17640).is_none());
    }

    #[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
    #[test]
    fn screenshot_query_parses_frame_and_full_map() {
        let req =
            crate::http_server::query::screenshot("frame=10&full_map=1&hide_ui=true&entity_ids=0")
                .unwrap();
        assert_eq!(req.frame, Some(10));
        assert!(req.full_map);
        assert!(req.hide_ui);
        assert_eq!(req.flags.entity_ids, Some(false));
    }
}

// ──────────────────────────────────────────────────────────────────
// Wasm JS bridge
// ──────────────────────────────────────────────────────────────────
//
// Browser has no loopback socket, so we expose the same request/reply
// pipeline as a JS-callable `rh_rpc({ method, params }) -> Promise`.
// Requests land on the current application's weakly bound queue,
// drain on the game tick, and resolve the Promise through an internal
// one-shot channel.

#[cfg(target_arch = "wasm32")]
pub mod wasm_rpc {
    use super::{BROWSER_QUEUE, HttpPayload, HttpRequest, Reply, ReplyBody, Responder};
    use wasm_bindgen::JsValue;

    fn reply_to_js(reply: Reply) -> Result<JsValue, JsValue> {
        match reply {
            Ok(ReplyBody::ReplayExport(_)) => {
                panic!("deferred replay export must be resolved before JavaScript encoding")
            }
            Ok(ReplyBody::Json(value)) => {
                use serde::Serialize;

                let serializer = serde_wasm_bindgen::Serializer::json_compatible();
                value
                    .serialize(&serializer)
                    .map_err(|e| JsValue::from_str(&format!("encode reply: {e}")))
            }
            Ok(ReplyBody::Binary { content_type, data }) => {
                let array = js_sys::Uint8Array::from(data.as_slice());
                let out = js_sys::Object::new();
                js_sys::Reflect::set(
                    &out,
                    &JsValue::from_str("contentType"),
                    &JsValue::from_str(content_type),
                )
                .map_err(|e| JsValue::from_str(&format!("set contentType: {e:?}")))?;
                js_sys::Reflect::set(&out, &JsValue::from_str("data"), &array)
                    .map_err(|e| JsValue::from_str(&format!("set data: {e:?}")))?;
                Ok(out.into())
            }
            Err(message) => Err(JsValue::from_str(&message.to_string())),
        }
    }

    /// JS → Rust entry point.  Accepts `{ method, params }` and returns
    /// a Promise resolved once the game loop drains the request on a
    /// frame boundary.
    #[wasm_bindgen::prelude::wasm_bindgen]
    pub async fn rh_rpc(request: JsValue) -> Result<JsValue, JsValue> {
        #[derive(serde::Deserialize)]
        struct Req {
            method: String,
            #[serde(default)]
            params: serde_json::Value,
        }
        let req: Req = serde_wasm_bindgen::from_value(request).map_err(|e| {
            JsValue::from_str(
                &super::RpcError::invalid_request(format!("bad request: {e}")).to_string(),
            )
        })?;
        // Pure-introspection methods don't need a live engine — resolve
        // inline without touching the tick queue.
        match req.method.as_str() {
            "info" => {
                return reply_to_js(Ok(ReplyBody::Json(super::info_json())));
            }
            "natives" => {
                return reply_to_js(Ok(ReplyBody::Json(super::list_natives_json())));
            }
            _ => {}
        }
        let payload = decode_request(&req.method, req.params)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let queue = BROWSER_QUEUE
            .with(|binding| binding.borrow().upgrade())
            .ok_or_else(|| {
                JsValue::from_str(
                    &super::RpcError::unavailable_capability("RPC bridge not initialized")
                        .to_string(),
                )
            })?;
        let retirement = queue
            .lock()
            .expect("queue mutex poisoned")
            .retirement_receiver();
        let (response_tx, rx) = Responder::channel();
        queue
            .lock()
            .expect("queue mutex poisoned")
            .push_back(HttpRequest {
                payload,
                response_tx: response_tx.with_router(&queue),
            });
        use futures::FutureExt as _;
        let reply = futures::select_biased! {
            _ = retirement.recv().fuse() => return reply_to_js(Err(super::RpcError::retired("HTTP transport stopped"))),
            reply = async {
                let reply = rx.recv().await.map_err(|e| JsValue::from_str(&super::RpcError::internal(format!("RPC response dropped: {e}")).to_string()))?;
                Ok::<_, JsValue>(super::resolve_deferred_reply(reply).await)
            }.fuse() => reply?,
        };
        reply_to_js(reply)
    }

    fn decode_request(
        method: &str,
        params: serde_json::Value,
    ) -> Result<HttpPayload, super::RpcError> {
        super::request_decode::decode_browser(method, params)
    }
}
