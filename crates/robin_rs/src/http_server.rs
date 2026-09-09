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
//!    (`Renderer::capture_frame_rgba`), hands them to
//!    [`PendingScreenshot::respond`] to reply with `image/png`, and
//!    calls `Renderer::reset_render_target` to clear the target for
//!    the next render.
//! 4. Finally the live frame is rendered and presented as normal.
//!
//! No authentication. Bind is `127.0.0.1` only. Pass `--http-server 0`
//! to disable the server entirely.

use robin_assets::decompile as assets_decompile;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::element as engine_element;
use robin_engine::engine as engine_api;
use robin_engine::engine::PANNEL_HEIGHT;
use robin_engine::natives as engine_natives;
use robin_engine::player_command::{DialogResult, FrameCommands, ModalKind, PlayerCommand};
use robin_engine::position_interface as engine_position_interface;
use robin_engine::profiles as engine_profiles;
use robin_engine::replay_rankability::InputTaintKind;
use robin_engine::scb as engine_scb;
use robin_engine::weapons as engine_weapons;
use std::borrow::Cow;
use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

#[cfg(not(target_arch = "wasm32"))]
use std::thread;
#[cfg(not(target_arch = "wasm32"))]
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
    /// `POST /go-to-frame` — absolute seek to `target` frame.
    /// Internally decomposes into a forward or backward step
    /// depending on the current frame.  Replay scrubbing uses this.
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

mod error;
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
#[cfg(not(target_arch = "wasm32"))]
mod native_routes;
#[cfg(not(target_arch = "wasm32"))]
mod native_transport;
mod request_decode;
use ingress::RequestRouter;
pub use ingress::SessionIngress;
#[cfg(not(target_arch = "wasm32"))]
use native_transport::NativeRequest;

type Queue = Arc<Mutex<RequestRouter>>;

struct HttpServer {
    replay_exports: crate::replay_service::ReplayExports,
    replay_launches: crate::replay_service::ReplayLaunches,
    queue: Queue,
    #[cfg(all(test, not(target_arch = "wasm32")))]
    bind_addr: std::net::SocketAddr,
    #[cfg(not(target_arch = "wasm32"))]
    listener: Option<thread::JoinHandle<()>>,
    #[cfg(not(target_arch = "wasm32"))]
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

impl<'de> serde::Deserialize<'de> for HttpTransport {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "live HTTP transport cannot be deserialized",
        ))
    }
}

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
            #[cfg(not(target_arch = "wasm32"))]
            let mut server = server;
            #[cfg(target_arch = "wasm32")]
            BROWSER_QUEUE.with(|binding| {
                let mut binding = binding.borrow_mut();
                if binding.ptr_eq(&Arc::downgrade(&server.queue)) {
                    *binding = std::sync::Weak::new();
                }
            });
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(stop) = server.stop.take() {
                let _ = stop.send(());
            }
            server.queue.lock().expect("RPC router poisoned").retire();
            #[cfg(not(target_arch = "wasm32"))]
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

#[cfg(all(test, not(target_arch = "wasm32")))]
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
            #[cfg(not(target_arch = "wasm32"))]
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
        #[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
fn parse_screenshot_query(query: &str) -> ScreenshotRequest {
    ScreenshotRequest {
        frame: query_param(query, "frame").and_then(|s| s.parse().ok()),
        width: query_param(query, "w").and_then(|s| s.parse().ok()),
        height: query_param(query, "h").and_then(|s| s.parse().ok()),
        hide_ui: query_flag(query, "hide_ui").unwrap_or(false),
        full_map: query_flag(query, "full_map").unwrap_or(false),
        flags: ScreenshotFlags {
            view_cones: query_flag(query, "view_cones"),
            pc_sight: query_flag(query, "pc_sight"),
            motion_graph: query_flag(query, "motion_graph"),
            surface: query_flag(query, "surface"),
            all_obstacles: query_flag(query, "all_obstacles"),
            elevation: query_flag(query, "elevation"),
            noise: query_flag(query, "noise"),
            sound_source: query_flag(query, "sound_source"),
            actor_info: query_flag(query, "actor_info"),
            script_zones: query_flag(query, "script_zones"),
            door: query_flag(query, "door"),
            projection_areas: query_flag(query, "projection_areas"),
            railroad: query_flag(query, "railroad"),
            probability: query_flag(query, "probability"),
            company_number: query_flag(query, "company_number"),
            combat_energy: query_flag(query, "combat_energy"),
            light_zones: query_flag(query, "light_zones"),
            animation_lines: query_flag(query, "animation_lines"),
            seek_points: query_flag(query, "seek_points"),
            fps: query_flag(query, "fps"),
            sprite_masks: query_flag(query, "sprite_masks"),
            // Default-on for screenshots: if the caller doesn't mention
            // the flag, force it true so every `/screenshot` labels
            // entities.  Pass `entity_ids=0` to opt out.
            entity_ids: Some(query_flag(query, "entity_ids").unwrap_or(true)),
        },
    }
}

/// Send a payload to the game loop and wait for the reply.  Caps the
/// wait at 60 s so a wedged game doesn't hang the client forever.
#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    for kv in query.split('&') {
        if let Some((k, v)) = kv.split_once('=')
            && k == key
        {
            return Some(v);
        }
    }
    None
}

/// Parse a query param as an optional bool.  Accepts `1`/`0`,
/// `true`/`false`, `yes`/`no`, `on`/`off` (case-insensitive).  Absent
/// key → `None`; present but empty → `Some(true)` so bare
/// `?view_cones&pc_sight` works.
#[cfg(not(target_arch = "wasm32"))]
fn query_flag(query: &str, key: &str) -> Option<bool> {
    let v = query_param(query, key)?;
    if v.is_empty() {
        return Some(true);
    }
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn info_json() -> serde_json::Value {
    serde_json::json!({
        "name": "robin-hood-script-rpc",
        "endpoints": [
            {"method": "GET",  "path": "/natives",            "desc": "list every NativeFn (index, name, params, return type)"},
            {"method": "GET",  "path": "/engine-dump",        "desc": "full serialized engine for ad-hoc debug"},
            {"method": "GET",  "path": "/level-assets",       "desc": "level-scoped static assets for ad-hoc debug, including static fast-grid sectors plus runtime fast-grid flags"},
            {"method": "GET",  "path": "/host-debug",         "desc": "host/UI state for ad-hoc debug, including trajectory preview and mouse hover fields"},
            {"method": "GET",  "path": "/script",             "desc": "mission-script class & function listing"},
            {"method": "GET",  "path": "/script/decompile",   "desc": "decompile to TypeScript-like pseudocode (?class=Foo)"},
            {"method": "POST", "path": "/native",             "desc": "invoke one native: {op, args, this?}"},
            {"method": "POST", "path": "/batch",              "desc": "invoke many natives on one tick: {calls: [{op, args, this?}]}"},
            {"method": "POST", "path": "/console",            "desc": "run a debug-console command: {command: '...'}"},
            {"method": "POST", "path": "/command",            "desc": "apply a PlayerCommand (externally-tagged JSON enum)"},
            {"method": "GET",  "path": "/screenshot",         "desc": "PNG at the requested frame. Query: frame (absolute sim frame), full_map, w, h (aspect-preserving max bounds), hide_ui, view_cones, pc_sight, motion_graph, all_obstacles, elevation, noise, sound_source, actor_info, script_zones, door, projection_areas, railroad, probability, company_number, combat_energy, light_zones, animation_lines, seek_points, fps, sprite_masks, entity_ids (bool flags)"},
            {"method": "POST", "path": "/step-forward",       "desc": "Run N engine ticks with --start-paused. Body {n: N, auto_dismiss: bool, dismissals: [{kind, result}], synchronized_multiplayer: bool}; live multiplayer requires explicit synchronized_multiplayer=true on the host and reconnects peers from the result."},
            {"method": "POST", "path": "/step-back",          "desc": "Rewind N frames via the rewind buffer. Body {n: N, auto_dismiss, dismissals}; the modal policy matches step-forward. Fails if target frame is older than the oldest retained snapshot."},
            {"method": "POST", "path": "/go-to-frame",        "desc": "Seek to an absolute frame. Body {frame: N, auto_dismiss, dismissals}; forward seeks tick and backward seeks restore canonical timeline history."},
        ],
    })
}

fn list_natives_json() -> serde_json::Value {
    let mut entries = Vec::new();
    for i in 0u32..512 {
        if let Ok(n) = engine_natives::NativeFn::try_from(i) {
            let name: &'static str = n.into();
            let sig = engine_natives::native_signature_by_name(name);
            entries.push(serde_json::json!({
                "index": i,
                "name": name,
                "return_type": sig.map(|s| s.return_type),
                "params": sig.map(|s| {
                    s.params.iter().map(|p| serde_json::json!({"type": p.ty, "name": p.name})).collect::<Vec<_>>()
                }),
            }));
        }
    }
    serde_json::json!({"natives": entries})
}

// ──────────────────────────────────────────────────────────────────
// Per-tick dispatch
// ──────────────────────────────────────────────────────────────────

/// Drain pending requests through `engine`/`host`. Called once per
/// tick from the game-session frame loop.  No-op when the transport
/// isn't running.
/// Drain the RPC queue without an engine — for use during the
/// `--wait-for-command` idle phase, where replay import/export does not need
/// engine state. The router rejects mission requests while no session is active.
fn drain_pre_engine(server: &HttpServer) {
    let pending: Vec<HttpRequest> = {
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

impl SessionIngress {
    pub fn drain(
        &mut self,
        engine: &mut Engine,
        frontend: &mut crate::host::HostFrontend,
        local_seat: robin_engine::player_command::PlayerId,
        net: Option<&crate::multiplayer::NetChannels>,
        assets: &LevelAssets,
        post_commands: &mut FrameCommands,
    ) -> Vec<engine_api::ExternalAction> {
        let mut selected = frontend.selected_view_element();
        let mut external_actions = Vec::new();
        self.drain_with_capabilities(
            engine,
            assets,
            &mut selected,
            net,
            post_commands,
            &mut external_actions,
            DispatchCapabilities::Interactive {
                frontend,
                local_seat,
            },
        );
        external_actions
    }

    /// Drain requests for a headless tool that owns an [`Engine`] directly.
    ///
    /// This is the small counterpart to [`SessionIngress::drain`] used by deterministic
    /// replay/debug runners. Requests which need the renderer or live host UI are
    /// rejected, while engine inspection, script/native calls, player commands,
    /// and the pause/step queue remain available.
    pub fn drain_headless(
        &mut self,
        engine: &mut Engine,
        assets: &LevelAssets,
        selected_view_element: &mut Option<engine_element::EntityId>,
    ) -> FrameCommands {
        let mut commands = FrameCommands::new();
        let mut external_actions = Vec::new();
        self.drain_with_capabilities(
            engine,
            assets,
            selected_view_element,
            None,
            &mut commands,
            &mut external_actions,
            DispatchCapabilities::Headless,
        );
        commands
    }

    /// Admission, taint accounting and routing have one ordering for every
    /// runner. Only the named presentation/diagnostic capabilities differ.
    fn drain_with_capabilities(
        &mut self,
        engine: &mut Engine,
        assets: &LevelAssets,
        selected_view_element: &mut Option<engine_element::EntityId>,
        net: Option<&crate::multiplayer::NetChannels>,
        commands: &mut FrameCommands,
        external_actions: &mut Vec<engine_api::ExternalAction>,
        mut capabilities: DispatchCapabilities<'_>,
    ) {
        for req in self.take_requests() {
            if !req.admit_unless_deferred() {
                continue;
            }
            self.observe_ranked_input_taint(&req.payload);
            match req.payload.classify() {
                RoutedRequest::HostDebug => {
                    req.response_tx
                        .send(capabilities.host_debug(engine, assets));
                }
                RoutedRequest::Query(QueryRequest::EngineDump) => {
                    req.response_tx.send(capabilities.engine_dump(engine));
                }
                RoutedRequest::Query(query) => req.response_tx.send(dispatch_query(
                    query,
                    self.replay_status(),
                    engine,
                    assets,
                )),
                RoutedRequest::Deferred(request) => {
                    self.defer_request(request, req.response_tx, capabilities.has_presentation())
                }
                RoutedRequest::Process(request) => self.dispatch_process(request, req.response_tx),
                RoutedRequest::Command(command) => {
                    let reply = dispatch_command(
                        command,
                        engine,
                        assets,
                        selected_view_element,
                        net,
                        commands,
                        external_actions,
                    );
                    capabilities.publish_selection(*selected_view_element);
                    req.response_tx.send(reply);
                }
            }
        }
    }
}

/// Live capabilities are borrowed for one drain, never saved or reconstructed.
enum DispatchCapabilities<'a> {
    Interactive {
        frontend: &'a mut crate::host::HostFrontend,
        local_seat: robin_engine::player_command::PlayerId,
    },
    Headless,
}

impl DispatchCapabilities<'_> {
    fn has_presentation(&self) -> bool {
        matches!(self, Self::Interactive { .. })
    }

    fn engine_dump(&self, engine: &Engine) -> Reply {
        // Original-parity runners own a nonserializable RNG source. Their
        // established diagnostic policy removes it from a clone only; the
        // interactive endpoint deliberately retains its full-snapshot policy.
        let value = match self {
            Self::Headless => {
                engine_dump_json(&engine.diagnostic_snapshot_without_original_rng_replay())
            }
            Self::Interactive { .. } => engine_dump_json(engine),
        };
        value
            .map(ReplyBody::Json)
            .map_err(|error| RpcError::internal(format!("engine serialize: {error}")))
    }

    fn host_debug(&self, engine: &Engine, assets: &LevelAssets) -> Reply {
        match self {
            Self::Interactive {
                frontend,
                local_seat,
            } => Ok(snapshot_host_debug(engine, frontend, *local_seat, assets).into()),
            Self::Headless => Err(RpcError::unavailable_capability(
                "host-debug is unavailable in a headless runner",
            )),
        }
    }

    fn publish_selection(&mut self, selected: Option<engine_element::EntityId>) {
        if let Self::Interactive { frontend, .. } = self {
            frontend.set_selected_view_element(selected);
        }
    }
}

fn admit_external_actions(
    engine: &mut Engine,
    assets: &LevelAssets,
    actions: Vec<engine_api::ExternalAction>,
    journal: &mut Vec<engine_api::ExternalAction>,
) -> Result<Vec<engine_api::ExternalActionResult>, RpcError> {
    let output = engine
        .advance_frame(
            assets,
            engine_api::SimulationFrameInput::no_hourglass()
                .with_post_external_actions(actions.clone()),
        )
        .map_err(|error| {
            let message = format!("developer action frame admission failed: {error}");
            match error {
                engine_api::FrameAdvanceError::RankedSimulationSettingCommandRejected {
                    ..
                } => RpcError::invalid_request(message),
                engine_api::FrameAdvanceError::RankedSimulationConfigViolation { .. }
                | engine_api::FrameAdvanceError::SpellforgeMissionAborted { .. }
                | engine_api::FrameAdvanceError::DirectorCompletionRejected { .. }
                | engine_api::FrameAdvanceError::SoundBoundaryRejected { .. }
                | engine_api::FrameAdvanceError::RecordedDropAleRouteRejected { .. } => {
                    RpcError::internal(message)
                }
            }
        })?;
    journal.extend(actions);
    Ok(output.external_action_results)
}

fn dispatch_command(
    payload: CommandRequest,
    engine: &mut Engine,
    assets: &LevelAssets,
    selected_view_element: &mut Option<engine_element::EntityId>,
    net: Option<&crate::multiplayer::NetChannels>,
    frame_commands: &mut FrameCommands,
    external_actions: &mut Vec<engine_api::ExternalAction>,
) -> Reply {
    match payload {
        CommandRequest::Native { name, args, this } => {
            let results = admit_external_actions(
                engine,
                assets,
                vec![engine_api::ExternalAction::Native {
                    name,
                    args,
                    this_actor: this,
                }],
                external_actions,
            )?;
            match results.into_iter().next() {
                Some(engine_api::ExternalActionResult::Native(result)) => result
                    .map(|value| ReplyBody::Json(serde_json::json!({"return": value})))
                    .map_err(RpcError::invalid_request),
                _ => Err(RpcError::internal(
                    "native frame admission returned no native result",
                )),
            }
        }
        CommandRequest::Batch(calls) => {
            let actions = calls
                .into_iter()
                .map(|call| engine_api::ExternalAction::Native {
                    name: call.op,
                    args: call.args,
                    this_actor: call.this,
                })
                .collect();
            let results = admit_external_actions(engine, assets, actions, external_actions)?
                .into_iter()
                .map(|result| match result {
                    engine_api::ExternalActionResult::Native(Ok(value)) => {
                        serde_json::json!({"return": value})
                    }
                    engine_api::ExternalActionResult::Native(Err(error)) => {
                        serde_json::json!({"error": error})
                    }
                    _ => serde_json::json!({"error": "non-native batch result"}),
                })
                .collect::<Vec<_>>();
            Ok(ReplyBody::Json(serde_json::json!({"results": results})))
        }
        CommandRequest::Console(cmd) => {
            // HTTP forces the full developer parser, but it has no live
            // `DevState`. Presentation-only commands therefore stay outside
            // the authoritative journal and report that limitation.
            let Some(command) = robin_engine::console::parse_with_final(&cmd, false) else {
                return Ok(ReplyBody::Json(frame_console_response_to_json(
                    engine_api::FrameConsoleResponse::Unknown,
                )));
            };
            if command.is_host_only() {
                return Ok(ReplyBody::Json(frame_console_response_to_json(
                    engine_api::FrameConsoleResponse::NotImplemented(
                        "host-only console command over HTTP".to_owned(),
                    ),
                )));
            }
            let results = admit_external_actions(
                engine,
                assets,
                vec![engine_api::ExternalAction::ConsoleCommand {
                    command,
                    selected_view_element: *selected_view_element,
                }],
                external_actions,
            )?;
            match results.into_iter().next() {
                Some(engine_api::ExternalActionResult::ConsoleCommand {
                    response,
                    selected_view_element: selected,
                }) => {
                    *selected_view_element = selected;
                    Ok(ReplyBody::Json(frame_console_response_to_json(response)))
                }
                _ => Err(RpcError::internal(
                    "console frame admission returned no console result",
                )),
            }
        }
        CommandRequest::Player(cmd) => {
            // In multiplayer, route the command over the wire so every
            // peer applies it at the same `target_frame`.  The local
            // engine doesn't mutate here; the echo lands via
            // `drain_net_inputs` at `sim_frame + INPUT_DELAY_FRAMES`.
            if let Some(net) = net {
                net.send_input(cmd)
                    .map_err(RpcError::unavailable_capability)?;
            } else {
                frame_commands.push(cmd);
            }
            Ok(ReplyBody::Json(serde_json::json!({"ok": true})))
        }
    }
}

fn dispatch_query(
    query: QueryRequest,
    replay: Option<ReplayStatus>,
    engine: &Engine,
    assets: &LevelAssets,
) -> Reply {
    match query {
        QueryRequest::State => Ok(ReplyBody::Json(snapshot_state(engine, replay))),
        QueryRequest::EngineDump => engine_dump_json(engine)
            .map(ReplyBody::Json)
            .map_err(|e| RpcError::internal(format!("engine serialize: {e}"))),
        QueryRequest::LevelAssets => level_assets_json(engine, assets)
            .map(ReplyBody::Json)
            .map_err(|e| RpcError::internal(format!("level assets serialize: {e}"))),
        QueryRequest::Script => Ok(ReplyBody::Json(snapshot_script(engine))),
        QueryRequest::Decompile { class } => {
            Ok(ReplyBody::Json(decompile_script(engine, class.as_deref())))
        }
    }
}

impl SessionIngress {
    fn dispatch_process(&self, request: ProcessRequest, response: Responder) {
        let Some((exports, launches)) = &self.replay_capabilities else {
            response.send(Err(RpcError::unavailable_capability(
                "replay transport capabilities were not attached",
            )));
            return;
        };
        match request {
            ProcessRequest::ExportReplay => start_replay_export(exports, response),
            ProcessRequest::LoadReplay { data, paused } => {
                response.send(decode_load_replay(launches, &data, paused))
            }
        }
    }
}

fn snapshot_state(engine: &Engine, replay: Option<ReplayStatus>) -> serde_json::Value {
    let replay = replay.map(|s| {
        serde_json::json!({
            "frame": s.frame,
            "total": s.total,
            "paused": s.paused,
        })
    });
    serde_json::json!({
        "frame": engine.frame_counter(),
        "map": engine.mission_map_name(),
        "replay": replay,
    })
}

fn snapshot_host_debug(
    engine: &Engine,
    frontend: &crate::host::HostFrontend,
    local_seat: robin_engine::player_command::PlayerId,
    assets: &LevelAssets,
) -> serde_json::Value {
    let selected_action = engine.selected_action_for_seat(local_seat);
    let selected_pc = engine.hero_selection(local_seat).first().copied();
    let selected_pc_state = selected_pc.and_then(|id| {
        engine.get_entity(id).map(|entity| {
            serde_json::json!({
                "id": id,
                "kind": entity.kind(),
                "pc_current_action": entity.pc_data().map(|pc| pc.current_action),
                "actor_action_state": entity.actor_data().map(|actor| actor.action_state),
                "position_map": entity.element_data().position_map(),
                "position_3d": entity.element_data().position(),
                "layer": entity.element_data().layer(),
                "direction": entity.element_data().direction(),
            })
        })
    });
    let preview = frontend.trajectory_preview();
    let last_preview_point = preview.points().last().map(|point| {
        serde_json::json!({
            "position": point.position,
            "time": point.time,
        })
    });
    let bow_hover = match (
        selected_action,
        selected_pc,
        frontend.input.feedback.focused_entity_id,
    ) {
        (engine_profiles::Action::Bow, Some(pc_id), Some(target_id)) => {
            let (target_status, shoot_mode) =
                engine.can_shoot_with_bow_at(assets, pc_id, target_id);
            Some(serde_json::json!({
                "target_id": target_id,
                "target_status": format!("{target_status:?}"),
                "shoot_mode": format!("{shoot_mode:?}"),
                "range_debug": bow_range_debug(engine, assets, pc_id, target_id),
            }))
        }
        _ => None,
    };

    serde_json::json!({
        "frame": engine.frame_counter(),
        "selected_action": selected_action,
        "selection": engine.hero_selection(local_seat),
        "selected_pc": selected_pc_state,
        "valid_trajectory": preview.is_valid(),
        "trajectory_preview_points_len": preview.points().len(),
        "trajectory_preview_start": preview.start(),
        "trajectory_preview_last": last_preview_point,
        "trajectory_preview_layer": preview.layer(),
        "net_crumpled": preview.crumpled(),
        "time_no_mouse_move": preview.hover_ticks(),
        "mouse_map_prev": preview.previous_mouse(),
        "trajectory_mark_count": preview.mark_count(),
        "bow_hover": bow_hover,
        "input": {
            "focused_entity_id": frontend.input.feedback.focused_entity_id,
            "target_drag": frontend.input.gestures.target_drag,
            "double_status_bar_entity_id": frontend.input.feedback.double_status_bar_entity_id,
            "selected_layer": frontend.input.spatial_hit().selected_layer,
            "selected_sector_idx": frontend.input.spatial_hit().selected_sector_idx,
            "selected_patch_idx": frontend.input.spatial_hit().selected_patch_idx,
            "hovered_door_idx": frontend.input.spatial_hit().hovered_door_idx,
            "valid_position_for_move": frontend.input.spatial_hit().valid_position_for_move,
            "mouse_opacity": frontend.input.feedback.mouse_opacity,
            "mouse_shadow_color": frontend.input.feedback.mouse_shadow_color,
            "left_mouse_down": frontend.input.left_mouse_down(),
            "right_mouse_down": frontend.input.controls.right_mouse_down,
            "is_dragging": frontend.input.is_dragging(),
            "is_alt": frontend.input.controls.is_alt,
        },
    })
}

fn bow_debug_ground_y_raw(point: engine_coordinates::WorldPoint3D) -> f32 {
    point.y
}

fn bow_debug_ground_y_projected(point: engine_coordinates::WorldPoint3D) -> f32 {
    point.to_map().y
}

fn game_sector_0_to_15_with_aspect(x: f32, y: f32, aspect_ratio: f32) -> u8 {
    const COS_PI_SIXTEENTH: f32 = 0.980_785_25;
    const SIN_PI_SIXTEENTH: f32 = 0.195_090_32;
    const TAN_PI_EIGHTH: f32 = 0.414_213_57;

    let mut rotated_x = x * COS_PI_SIXTEENTH * aspect_ratio - y * SIN_PI_SIXTEENTH;
    let mut rotated_y = x * SIN_PI_SIXTEENTH * aspect_ratio + y * COS_PI_SIXTEENTH;

    let west = rotated_x < 0.0;
    if west {
        rotated_x = -rotated_x;
    }

    let south = rotated_y > 0.0;
    if !south {
        rotated_y = -rotated_y;
    }

    let east_west = rotated_y < rotated_x;
    let skew = if east_west {
        rotated_y > rotated_x * TAN_PI_EIGHTH
    } else {
        rotated_x > rotated_y * TAN_PI_EIGHTH
    };

    let mut sector = 0u8;
    if west {
        sector |= 8;
    }
    if west ^ south {
        sector |= 4;
    }
    if west ^ south ^ east_west {
        sector |= 2;
    }
    if west ^ south ^ east_west ^ skew {
        sector |= 1;
    }
    sector
}

fn bow_profile_debug(
    engine: &Engine,
    assets: &LevelAssets,
    entity_id: engine_element::EntityId,
) -> Option<serde_json::Value> {
    let entity = engine.get_entity(entity_id)?;
    let (bow_profile_idx, shooting_ability) = match entity {
        engine_element::Entity::Pc(pc) => {
            let idx = usize::from(pc.pc.profile_index);
            let profile = assets.profile_manager.characters.get(idx)?;
            if profile.shooting_weapon_id == 0 {
                return None;
            }
            (profile.shooting_weapon_id, profile.shooting as u32)
        }
        engine_element::Entity::Soldier(soldier) => {
            let idx = usize::from(soldier.soldier.soldier_profile_index);
            let profile = assets.profile_manager.soldiers.get(idx)?;
            if profile.shooting_weapon_id == 0 {
                return None;
            }
            (profile.shooting_weapon_id, profile.shooting as u32)
        }
        _ => return None,
    };

    let bow_profile = assets.profile_manager.get_bow(bow_profile_idx)?;
    let bow_state = engine_weapons::BowState::new(bow_profile_idx, bow_profile, 1);
    Some(serde_json::json!({
        "bow_profile_idx": bow_profile_idx,
        "shooting_ability": shooting_ability,
        "normal_range": bow_profile.normal_shoot.range,
        "long_range": bow_profile.long_shoot.range,
        "has_long_shoot": bow_profile.has_long_shoot,
        "max_range": bow_state.get_max_range(bow_profile),
    }))
}

fn bow_target_points_debug(
    engine: &Engine,
    target_id: engine_element::EntityId,
) -> Option<serde_json::Value> {
    let target = engine.get_entity(target_id)?;
    let range_target = if target.is_human() {
        target.compute_belt_point()
    } else {
        Some(target.element_data().position())
    };
    let preview_target = if target.is_human() {
        target.compute_belt_point()
    } else if target.is_fx_target() {
        target.compute_target_center()
    } else {
        Some(target.element_data().position())
    };

    Some(serde_json::json!({
        "id": target_id,
        "kind": target.kind(),
        "is_human": target.is_human(),
        "is_fx_target": target.is_fx_target(),
        "position_3d": target.element_data().position(),
        "position_map": target.element_data().position_map(),
        "belt_point": target.compute_belt_point(),
        "eyes_point": target.compute_eyes_point(None),
        "fx_center": target.compute_target_center(),
        "range_target_point": range_target,
        "preview_target_point": preview_target,
    }))
}

fn bow_range_math_debug(
    hand_point: engine_coordinates::WorldPoint3D,
    target_point: engine_coordinates::WorldPoint3D,
    max_range: f32,
    forest_target: bool,
) -> serde_json::Value {
    const THROW_ANGLE_BOW: f32 = 0.3;
    let rel_height = hand_point.z - target_point.z;
    let base_radius = if rel_height > 0.0 {
        max_range + rel_height * THROW_ANGLE_BOW.tan()
    } else {
        max_range
    };
    let radius = if forest_target {
        base_radius * 2.0
    } else {
        base_radius
    };

    let dx = target_point.x - hand_point.x;
    let dy_raw = bow_debug_ground_y_raw(target_point) - bow_debug_ground_y_raw(hand_point);
    let dy_projected =
        bow_debug_ground_y_projected(target_point) - bow_debug_ground_y_projected(hand_point);
    let dz = target_point.z - hand_point.z;
    let dy_range_raw = dy_raw * engine_position_interface::INVERSE_ASPECT_RATIO_PROJECTILES;
    let dy_range_projected =
        dy_projected * engine_position_interface::INVERSE_ASPECT_RATIO_PROJECTILES;
    let square_distance_raw = dx * dx + dy_range_raw * dy_range_raw;
    let square_distance_projected = dx * dx + dy_range_projected * dy_range_projected;
    let radius_square = radius * radius;
    let dist_3d_raw = (dx * dx + dy_raw * dy_raw + dz * dz).sqrt();
    let dist_3d_projected = (dx * dx + dy_projected * dy_projected + dz * dz).sqrt();

    serde_json::json!({
        "hand_point": hand_point,
        "target_point": target_point,
        "target_delta": {
            "dx": dx,
            "dy_raw_game": dy_raw,
            "dy_projected_y_minus_z": dy_projected,
            "dz": dz,
        },
        "range": {
            "max_range": max_range,
            "rel_height": rel_height,
            "throw_angle_bow": THROW_ANGLE_BOW,
            "base_radius": base_radius,
            "forest_target": forest_target,
            "radius": radius,
            "radius_square": radius_square,
            "dy_raw_times_projectile_aspect": dy_range_raw,
            "dy_projected_times_projectile_aspect": dy_range_projected,
            "square_distance_raw_game_y": square_distance_raw,
            "square_distance_projected_y_minus_z": square_distance_projected,
            "in_range_raw_game_y": square_distance_raw < radius_square,
            "in_range_projected_y_minus_z": square_distance_projected < radius_square,
            "dist_3d_raw_game_y": dist_3d_raw,
            "dist_3d_projected_y_minus_z": dist_3d_projected,
        },
        "direction": {
            "iso_sector_raw_game_y": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_raw),
            "rust_iso_sector_projected_y_minus_z": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_projected),
            "game_sector_aspect_raw_game_y": game_sector_0_to_15_with_aspect(
                dx,
                dy_raw,
                engine_position_interface::ASPECT_RATIO,
            ),
            "game_sector_aspect_projected_y_minus_z": game_sector_0_to_15_with_aspect(
                dx,
                dy_projected,
                engine_position_interface::ASPECT_RATIO,
            ),
        },
    })
}

fn bow_range_debug(
    engine: &Engine,
    assets: &LevelAssets,
    pc_id: engine_element::EntityId,
    target_id: engine_element::EntityId,
) -> serde_json::Value {
    let Some(shooter) = engine.get_entity(pc_id) else {
        return serde_json::json!({"error": "missing_shooter", "pc_id": pc_id});
    };
    let Some(target) = engine.get_entity(target_id) else {
        return serde_json::json!({"error": "missing_target", "target_id": target_id});
    };
    let Some(hand_point) = shooter.compute_hand_point(None) else {
        return serde_json::json!({"error": "missing_shooter_hand_point", "pc_id": pc_id});
    };

    let bow_profile = bow_profile_debug(engine, assets, pc_id);
    let max_range = bow_profile
        .as_ref()
        .and_then(|profile| profile.get("max_range"))
        .and_then(serde_json::Value::as_u64)
        .map(|v| v as f32);
    let range_target_point = if target.is_human() {
        target.compute_belt_point()
    } else {
        Some(target.element_data().position())
    };
    let preview_target_point = if target.is_human() {
        target.compute_belt_point()
    } else if target.is_fx_target() {
        target.compute_target_center()
    } else {
        Some(target.element_data().position())
    };
    let forest_target = !target.is_human() && engine.weather().is_forest_level;
    let range_math = match (range_target_point, max_range) {
        (Some(point), Some(max_range)) => Some(bow_range_math_debug(
            hand_point,
            point,
            max_range,
            forest_target,
        )),
        _ => None,
    };
    let preview_direction = preview_target_point.map(|point| {
        let dx = point.x - shooter.element_data().position().x;
        let dy_raw = point.y - shooter.element_data().position().y;
        let dy_projected =
            bow_debug_ground_y_projected(point) - bow_debug_ground_y_projected(shooter.element_data().position());
        serde_json::json!({
            "source_position_3d": shooter.element_data().position(),
            "preview_target_point": point,
            "dx": dx,
            "dy_raw_game": dy_raw,
            "dy_projected_y_minus_z": dy_projected,
            "iso_sector_raw_game_y": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_raw),
            "rust_iso_sector_projected_y_minus_z": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_projected),
            "game_sector_aspect_raw_game_y": game_sector_0_to_15_with_aspect(
                dx,
                dy_raw,
                engine_position_interface::ASPECT_RATIO,
            ),
            "game_sector_aspect_projected_y_minus_z": game_sector_0_to_15_with_aspect(
                dx,
                dy_projected,
                engine_position_interface::ASPECT_RATIO,
            ),
        })
    });

    serde_json::json!({
        "shooter": {
            "id": pc_id,
            "kind": shooter.kind(),
            "position_3d": shooter.element_data().position(),
            "position_map": shooter.element_data().position_map(),
            "hand_point": hand_point,
            "direction": shooter.element_data().direction(),
            "posture": shooter.element_data().posture(),
            "pc_current_action": shooter.pc_data().map(|pc| pc.current_action),
            "actor_action_state": shooter.actor_data().map(|actor| actor.action_state),
        },
        "target": bow_target_points_debug(engine, target_id),
        "bow_profile": bow_profile,
        "forest_target": forest_target,
        "range_math": range_math,
        "preview_direction": preview_direction,
    })
}

fn engine_dump_json(engine: &Engine) -> Result<serde_json::Value, String> {
    crate::json_value::to_json_value(engine).map_err(|e| e.to_string())
}

fn level_assets_json(engine: &Engine, assets: &LevelAssets) -> Result<serde_json::Value, String> {
    let mut root = serde_json::Map::new();
    root.insert("schema".into(), serde_json::json!("level-assets.v1"));
    root.insert(
        "counts".into(),
        serde_json::json!({
            "level_grid": {
                "lines": assets.navigation.level_grid.lines.len(),
                "sectors": assets.navigation.level_grid.sectors.len(),
                "masks": assets.navigation.level_grid.masks.len(),
                "jump_lines": assets.navigation.level_grid.jump_lines.len(),
                "blocks": assets.navigation.level_grid.blocks.len(),
                "layers": assets.navigation.level_grid.layers.len(),
                "level_repulsive_points": assets.navigation.level_grid.level_repulsive_points.len(),
                "shadow_data": assets.navigation.level_grid.shadow_data.len(),
            },
            "pathfinder_graph": {
                "nodes": assets.navigation.pathfinder_graph.nodes.len(),
                "layers": assets.navigation.pathfinder_graph.layers.len(),
                "links": assets.navigation.pathfinder_graph.static_data.links.len(),
                "link_configs": assets.navigation.pathfinder_graph.static_data.link_configs.len(),
                "move_layers": assets.navigation.pathfinder_graph.static_data.move_layers.len(),
                "alternative_move_layers": assets.navigation.pathfinder_graph.static_data.alternative_move_layers.len(),
            },
            "profiles": {
                "characters": assets.profile_manager.characters.len(),
                "soldiers": assets.profile_manager.soldiers.len(),
                "civilians": assets.profile_manager.civilians.len(),
                "hth_weapons": assets.profile_manager.hth_weapons.len(),
                "bows": assets.profile_manager.bows.len(),
                "missions": assets.profile_manager.missions.len(),
            },
            "mission_script_programs": assets.scripts.mission_programs.len(),
            "hiking_paths": assets.navigation.hiking_paths.len(),
            "static_sight_obstacles": assets.environment.static_sight_obstacles.len(),
            "accessory_sprite_prototypes": assets.accessory_sprite_prototypes.len(),
            "water_zones": assets.environment.water_zones.zones.len(),
            "material_sectors": assets.environment.material_sectors.sectors.len(),
            "script_locations": assets.scripts.location_count,
            "script_points": assets.scripts.point_count,
            "script_buildings": assets.scripts.building_count,
            "script_hiking_paths": assets.scripts.hiking_path_count,
        }),
    );
    root.insert(
        "pixel_opacity_attached".into(),
        serde_json::json!(assets.attachments.pixel_opacity.is_some()),
    );
    insert_json(&mut root, "fast_grid_runtime", engine.fast_grid())?;

    let mut asset = serde_json::Map::new();
    insert_json(&mut asset, "sprite_scriptor", &*assets.sprite_scriptor)?;
    insert_json(&mut asset, "level_grid", &*assets.navigation.level_grid)?;
    insert_json(
        &mut asset,
        "pathfinder_graph",
        &*assets.navigation.pathfinder_graph,
    )?;
    insert_json(&mut asset, "hiking_paths", &*assets.navigation.hiking_paths)?;
    insert_json(&mut asset, "profile_manager", &*assets.profile_manager)?;
    insert_json(&mut asset, "bank_signature", &assets.bank_signature)?;
    insert_json(
        &mut asset,
        "mission_script_programs",
        &*assets.scripts.mission_programs,
    )?;
    insert_json(&mut asset, "peasant_firstnames", &assets.peasant_firstnames)?;
    insert_json(&mut asset, "peasant_surnames", &assets.peasant_surnames)?;
    insert_json(
        &mut asset,
        "accessory_sprite_prototypes",
        &assets.accessory_sprite_prototypes,
    )?;
    insert_json(
        &mut asset,
        "exclamation_durations",
        &assets.audio.exclamation_durations(),
    )?;
    insert_json(
        &mut asset,
        "source_durations",
        &assets.audio.source_durations(),
    )?;
    insert_json(
        &mut asset,
        "sound_source_required_ids",
        &assets.audio.sound_source_required_ids,
    )?;
    insert_json(
        &mut asset,
        "patch_entity_handles",
        &assets.entities.patch_animation_entities,
    )?;
    insert_json(
        &mut asset,
        "scroll_entity_ids",
        &assets.entities.scroll_entity_ids,
    )?;
    insert_json(
        &mut asset,
        "all_soldier_entity_ids",
        &assets.entities.soldier_entity_ids,
    )?;
    insert_json(
        &mut asset,
        "soldier_subordinate_ids",
        &assets.entities.soldier_subordinate_ids,
    )?;
    insert_json(&mut asset, "water_zones", &assets.environment.water_zones)?;
    insert_json(
        &mut asset,
        "material_sectors",
        &assets.environment.material_sectors,
    )?;
    insert_json(
        &mut asset,
        "static_sight_obstacles",
        &*assets.environment.static_sight_obstacles,
    )?;
    insert_json(
        &mut asset,
        "script_location_count",
        &assets.scripts.location_count,
    )?;
    insert_json(
        &mut asset,
        "script_point_count",
        &assets.scripts.point_count,
    )?;
    insert_json(
        &mut asset,
        "script_location_positions",
        &assets.scripts.location_positions,
    )?;
    insert_json(
        &mut asset,
        "script_location_layers",
        &assets.scripts.location_layers,
    )?;
    insert_json(
        &mut asset,
        "script_location_sectors",
        &assets.scripts.location_sectors,
    )?;
    insert_json(
        &mut asset,
        "script_building_count",
        &assets.scripts.building_count,
    )?;
    insert_json(
        &mut asset,
        "script_hiking_path_count",
        &assets.scripts.hiking_path_count,
    )?;
    insert_json(
        &mut asset,
        "script_zone_grid_indices",
        &assets.scripts.zone_grid_indices,
    )?;
    root.insert("assets".into(), serde_json::Value::Object(asset));

    Ok(serde_json::Value::Object(root))
}

fn insert_json<T>(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: &T,
) -> Result<(), String>
where
    T: serde::Serialize + ?Sized,
{
    object.insert(
        key.into(),
        crate::json_value::to_json_value(value).map_err(|e| e.to_string())?,
    );
    Ok(())
}

// ──────────────────────────────────────────────────────────────────
// Replay export transport adapter
// ──────────────────────────────────────────────────────────────────

fn start_replay_export(exports: &crate::replay_service::ReplayExports, response_tx: Responder) {
    response_tx.send(Ok(ReplyBody::ReplayExport(exports.export())));
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
/// 3. Read the pixels back (`Renderer::capture_frame_rgba`).
/// 4. Consume this struct via [`PendingScreenshot::respond`], handing
///    over the pixels so the request replies with `image/png`.
/// 5. Call `Renderer::reset_render_target` to clear the offscreen
///    target for the next render pass (screenshot or live).
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
    /// Absolute seek — no-op if `target == sim_frame`, decomposes into
    /// a forward or back step otherwise.  Replay scrubbing uses this.
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

fn can_capture_presented_ui(request: &ScreenshotRequest) -> bool {
    !request.hide_ui && !request.full_map && request.flags == ScreenshotFlags::default()
}

/// Merge a request's `Some(x)` overrides onto `debug`, mutating in
/// place.  Apply this to a **cloned** `DevState` so the live state
/// stays untouched — the caller keeps the original and passes the
/// clone to `render_frame`.
pub fn apply_screenshot_flags(debug: &mut engine_api::DebugFlags, flags: &ScreenshotFlags) {
    macro_rules! set {
        ($name:ident, $field:ident) => {
            if let Some(v) = flags.$name {
                debug.$field = v;
            }
        };
    }
    set!(view_cones, all_view_cones);
    set!(pc_sight, pc_sight);
    set!(motion_graph, motion_graph_display);
    set!(surface, surface_display);
    set!(all_obstacles, all_obstacles_display);
    set!(elevation, elevation_display);
    set!(noise, noise_display);
    set!(sound_source, sound_source_display);
    set!(actor_info, actor_info_display);
    set!(script_zones, script_zone_display);
    set!(door, door_display);
    set!(projection_areas, projection_areas_display);
    set!(railroad, railroad_display);
    set!(probability, prob_display);
    set!(company_number, company_number_display);
    set!(combat_energy, combat_energy_display);
    set!(light_zones, display_light_zones);
    set!(animation_lines, display_animation_lines);
    set!(seek_points, display_seek_points);
    set!(fps, fps_display);
    set!(sprite_masks, sprite_masks_display);
    set!(entity_ids, entity_ids);
}

/// Apply optional crop + resize, then encode as PNG.  Nearest-neighbour
/// scaling — good enough for a dev-inspection endpoint and avoids
/// pulling in an image crate.
fn encode_png(src_w: u32, src_h: u32, rgba: &[u8], req: &ScreenshotRequest) -> Reply {
    // Optional bottom-panel crop: strip the HUD strip before any resize.
    let (src, mut used_w, mut used_h) =
        if req.hide_ui && !req.full_map && src_h > PANNEL_HEIGHT as u32 {
            let new_h = src_h - PANNEL_HEIGHT as u32;
            let stride = (src_w as usize) * 4;
            let cropped: Vec<u8> = rgba[..stride * new_h as usize].to_vec();
            (Cow::Owned(cropped), src_w, new_h)
        } else {
            (Cow::Borrowed(rgba), src_w, src_h)
        };

    let (target_w, target_h) =
        screenshot_target_dimensions(used_w, used_h, req).map_err(RpcError::invalid_request)?;

    let resized;
    let pixels: &[u8] = if (target_w, target_h) != (used_w, used_h) {
        let mut out = vec![0u8; (target_w * target_h * 4) as usize];
        for dy in 0..target_h {
            let sy = (dy * used_h / target_h).min(used_h - 1);
            for dx in 0..target_w {
                let sx = (dx * used_w / target_w).min(used_w - 1);
                let si = ((sy * used_w + sx) * 4) as usize;
                let di = ((dy * target_w + dx) * 4) as usize;
                out[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
        resized = out;
        used_w = target_w;
        used_h = target_h;
        &resized
    } else {
        &src
    };

    let mut png_bytes: Vec<u8> = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, used_w, used_h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| RpcError::internal(format!("png header: {e}")))?;
        writer
            .write_image_data(pixels)
            .map_err(|e| RpcError::internal(format!("png data: {e}")))?;
    }
    Ok(ReplyBody::Binary {
        content_type: "image/png",
        data: png_bytes,
    })
}

fn screenshot_target_dimensions(
    src_w: u32,
    src_h: u32,
    req: &ScreenshotRequest,
) -> Result<(u32, u32), String> {
    let (Some(max_w), Some(max_h)) = (req.width, req.height) else {
        return Ok((src_w, src_h));
    };
    if max_w == 0 || max_h == 0 {
        return Err("screenshot width/height must be > 0".into());
    }

    let max_w = max_w as u32;
    let max_h = max_h as u32;
    let height_for_max_w = ((src_h as u64 * max_w as u64) / src_w as u64) as u32;
    if height_for_max_w <= max_h {
        Ok((max_w, height_for_max_w.max(1)))
    } else {
        let width_for_max_h = ((src_w as u64 * max_h as u64) / src_h as u64) as u32;
        Ok((width_for_max_h.max(1), max_h))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
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

    fn screenshot_request(width: Option<u16>, height: Option<u16>) -> ScreenshotRequest {
        ScreenshotRequest {
            width,
            height,
            ..ScreenshotRequest::default()
        }
    }

    #[test]
    fn screenshot_dimensions_fit_width_limited_bounds() {
        let req = screenshot_request(Some(1280), Some(720));
        assert_eq!(
            screenshot_target_dimensions(1024, 768, &req).unwrap(),
            (960, 720)
        );
    }

    #[test]
    fn screenshot_dimensions_fit_height_limited_bounds() {
        let req = screenshot_request(Some(640), Some(480));
        assert_eq!(
            screenshot_target_dimensions(1920, 1080, &req).unwrap(),
            (640, 360)
        );
    }

    #[test]
    fn screenshot_dimensions_leave_size_when_bounds_missing() {
        let req = screenshot_request(Some(640), None);
        assert_eq!(
            screenshot_target_dimensions(1024, 768, &req).unwrap(),
            (1024, 768)
        );
    }

    #[test]
    fn screenshot_dimensions_reject_zero_bounds() {
        let req = screenshot_request(Some(0), Some(720));
        assert!(screenshot_target_dimensions(1024, 768, &req).is_err());
    }

    #[test]
    fn only_plain_ui_screenshots_use_presented_modal_frame() {
        let plain = ScreenshotRequest::default();
        assert!(can_capture_presented_ui(&plain));

        let hidden = ScreenshotRequest {
            hide_ui: true,
            ..plain.clone()
        };
        assert!(!can_capture_presented_ui(&hidden));

        let full_map = ScreenshotRequest {
            full_map: true,
            ..plain.clone()
        };
        assert!(!can_capture_presented_ui(&full_map));

        let overridden = ScreenshotRequest {
            flags: ScreenshotFlags {
                view_cones: Some(true),
                ..ScreenshotFlags::default()
            },
            ..plain
        };
        assert!(!can_capture_presented_ui(&overridden));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn screenshot_query_parses_frame_and_full_map() {
        let req = parse_screenshot_query("frame=10&full_map=1&hide_ui=true&entity_ids=0");
        assert_eq!(req.frame, Some(10));
        assert!(req.full_map);
        assert!(req.hide_ui);
        assert_eq!(req.flags.entity_ids, Some(false));
    }
}

fn frame_console_response_to_json(response: engine_api::FrameConsoleResponse) -> serde_json::Value {
    use engine_api::FrameConsoleResponse as R;

    match response {
        R::Ok(message) => serde_json::json!({"kind": "ok", "message": message}),
        R::Unknown => serde_json::json!({"kind": "unknown"}),
        R::NotImplemented(command) => {
            serde_json::json!({"kind": "not_implemented", "command": command})
        }
        R::LoadCampaignRequested(path) => serde_json::json!({
            "kind": "host_followup",
            "variant": "LoadCampaignRequested",
            "path": path,
        }),
        R::DeityInvoked => serde_json::json!({
            "kind": "host_followup",
            "variant": "DeityInvoked",
        }),
    }
}

fn snapshot_script(engine: &Engine) -> serde_json::Value {
    let Some(script) = engine.mission_script() else {
        return serde_json::json!({"loaded": false});
    };
    let scb = script.scb();
    let counts = script.instance_counts();
    let classes: Vec<_> = scb
        .classes
        .iter()
        .map(|c| {
            let funcs: Vec<&str> = c.functions.iter().map(|f| f.name.as_str()).collect();
            let members: Vec<&str> = c.member_variables.iter().map(|m| m.name.as_str()).collect();
            serde_json::json!({
                "name": c.class_name,
                "source_filename": c.source_file,
                "functions": funcs,
                "members": members,
                "quad_count": c.quads.len(),
            })
        })
        .collect();
    serde_json::json!({
        "loaded": true,
        "version": scb.version,
        "class_count": classes.len(),
        "actor_instances": counts.actors,
        "zone_instances": counts.zones,
        "target_instances": counts.targets,
        "scroll_instances": counts.scrolls,
        "waypoint_instances": counts.waypoints,
        "classes": classes,
    })
}

fn decompile_script(engine: &Engine, class: Option<&str>) -> serde_json::Value {
    let Some(script) = engine.mission_script() else {
        return serde_json::json!({"error": "no mission script loaded"});
    };
    let scb = script.scb();
    let source = if let Some(name) = class {
        // Single-class mode: rebuild a minimal ScbFile holding just
        // this class so the existing whole-file decompiler can run on
        // it without us reaching into its private per-class entry
        // points.
        let Some(c) = scb.classes.iter().find(|c| c.class_name == name) else {
            return serde_json::json!({"error": format!("class not found: {name}")});
        };
        let scb_one = engine_scb::ScbFile {
            version: scb.version,
            classes: vec![c.clone()],
        };
        assets_decompile::decompile(&scb_one)
    } else {
        assets_decompile::decompile(scb)
    };
    serde_json::json!({"source": source})
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
