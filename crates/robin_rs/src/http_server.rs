//! Local script-RPC endpoint exposing the script VM, console, player
//! command pipeline, engine dump, decompiler, and per-frame
//! screenshot capture to external tools (debug shells, test harnesses,
//! AI drivers).
//!
//! Two transports share the same request/reply enums + per-tick drain:
//!
//! - **Native:** a `tiny_http` listener on `127.0.0.1:<port>` (see
//!   [`start_global`]).  Endpoints are:
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
//! A dedicated listener thread runs `tiny_http`'s blocking accept loop.
//! Each request is decoded into a [`HttpRequest`] and pushed onto a
//! shared FIFO with a one-shot `SyncSender` for the reply. The game
//! loop drains the queue once per tick (see
//! `game_session::drain_http_queue`), executes each request inline, and
//! sends the reply back. The listener serialises it to JSON (or raw
//! image/png bytes for `/screenshot`).
//!
//! Pause / level-loading / replay rewind / modal dialogs all suspend
//! the per-tick drain, so a request issued during those windows blocks
//! until the game resumes — bounded by a 60 s recv timeout on the
//! listener side.  Clients that want to fail fast instead of waiting
//! out a blocked main loop should pass a shorter HTTP timeout
//! themselves (e.g. `curl --max-time 2`).
//!
//! ### Screenshot pipeline
//!
//! `/screenshot` is special because it needs a rendered frame, not the
//! post-tick engine state.  The game loop:
//!
//! 1. [`drain_global`] moves screenshot requests from the request
//!    queue into a module-local pending list.  **No mutation** of the
//!    live `Engine`, `DevState`, or any host state happens here.
//! 2. Before the live frame is rendered, the main loop calls
//!    [`take_pending_screenshots`] and renders one throwaway frame
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
use robin_engine::engine_manager as engine_manager_api;
use robin_engine::natives as engine_natives;
use robin_engine::player_command::PlayerCommand;
use robin_engine::position_interface as engine_position_interface;
use robin_engine::profiles as engine_profiles;
use robin_engine::replay as engine_replay;
use robin_engine::scb as engine_scb;
use robin_engine::weapons as engine_weapons;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, SyncSender};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use robin_engine::engine::{Engine, LevelAssets};

/// Default port. Reasonably uncommon and easy to remember; change with
/// `--http-server <port>` or set 0 to disable.
pub const DEFAULT_PORT: u16 = 17640;

/// One pending request waiting for the game tick.
pub struct HttpRequest {
    pub payload: HttpPayload,
    pub response_tx: Responder,
}

/// Per-request payload — the transport layer parses each endpoint
/// down to one of these.  Distinct variants (rather than a generic
/// `serde_json::Value` body) keep the dispatch typed: each handler in
/// `dispatch_in_engine` does its own argument extraction once.
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
    StepForward { n: u32 },
    /// `POST /step-back` — rewind `n` frames synchronously.
    StepBack { n: u32 },
    /// `POST /go-to-frame` — absolute seek to `target` frame.
    /// Internally decomposes into a forward or backward step
    /// depending on the current frame.  Replay scrubbing uses this.
    GoToFrame { target: u32 },
    /// `POST /set-paused` / `robin.call("set-paused", {paused})` —
    /// toggle the mission loop's manual pause flag.
    SetPaused { paused: bool },
    /// `GET /get-replay` — snapshot the current recorder's byte
    /// stream.  Served from an in-memory mirror populated by the
    /// recorder's tee-writer; no filesystem read required, so the
    /// same path works on native and wasm.  Returns the raw JSONL
    /// text so callers don't have to base64-wrap binary data.
    GetReplay,
    /// `POST /load-replay` — stash replay bytes + a `paused` flag into
    /// a process-global slot that [`init_replay_and_rollback`] consumes
    /// on the next mission start.  The caller is responsible for
    /// triggering a mission restart (e.g. by sending a console command
    /// or by resetting the Game op) so the slot is actually picked up.
    LoadReplay { data: String, paused: bool },
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
#[derive(Clone, Default, Debug, serde::Deserialize)]
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

#[derive(Clone, Debug, serde::Deserialize)]
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
pub type Reply = Result<ReplyBody, String>;

/// One-shot reply channel.  Native uses a `mpsc::sync_channel` so the
/// listener thread can block on recv; wasm uses an async one-shot
/// channel that resolves the Promise returned by `rh_rpc`.
pub enum Responder {
    #[cfg(not(target_arch = "wasm32"))]
    Channel(SyncSender<Reply>),
    #[cfg(target_arch = "wasm32")]
    Wasm(async_channel::Sender<Reply>),
}

impl Responder {
    pub fn send(self, reply: Reply) {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Channel(tx) => {
                if let Err(e) = tx.send(reply) {
                    tracing::debug!("script RPC: response dropped (listener gone): {e}");
                }
            }
            #[cfg(target_arch = "wasm32")]
            Self::Wasm(tx) => {
                if let Err(e) = tx.try_send(reply) {
                    tracing::debug!("script RPC: response dropped (wasm promise gone): {e}");
                }
            }
        }
    }
}

pub type Queue = Arc<Mutex<VecDeque<HttpRequest>>>;

pub struct HttpServer {
    pub queue: Queue,
    #[cfg(not(target_arch = "wasm32"))]
    pub bind_addr: std::net::SocketAddr,
}

static GLOBAL: OnceLock<HttpServer> = OnceLock::new();

/// Bring up the script-RPC transport and stash the queue in a
/// process-global so the per-tick drain can reach it without threading
/// the queue through every signature.  Re-calls are silently ignored.
///
/// Native: binds a loopback HTTP listener on `port` (0 disables).
/// Wasm: ignores `port`; just installs the empty queue so `rh_rpc`
/// has somewhere to push.
pub fn start_global(port: u16) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = port;
        if GLOBAL.get().is_some() {
            return Ok(());
        }
        let _ = GLOBAL.set(HttpServer {
            queue: Arc::new(Mutex::new(VecDeque::new())),
        });
        tracing::info!("script RPC: wasm bridge ready (rh_rpc)");
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if port == 0 {
            tracing::info!("script HTTP server: disabled (--http-server 0)");
            return Ok(());
        }
        if GLOBAL.get().is_some() {
            return Ok(());
        }
        let server = start(port)?;
        let _ = GLOBAL.set(server);
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start(port: u16) -> Result<HttpServer, String> {
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| {
        format!(
            "script HTTP server failed to bind 127.0.0.1:{port}: {e} \
             (another robin instance? pass `--http-server 0` to disable, \
             or `--http-server <port>` to pick a different port)"
        )
    })?;
    let bind_addr = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| "script HTTP server bound to non-IP address".to_string())?;
    tracing::info!("script HTTP server listening on http://{bind_addr}");

    let queue: Queue = Arc::new(Mutex::new(VecDeque::new()));
    let queue_for_thread = queue.clone();
    thread::Builder::new()
        .name("robin-http-server".into())
        .spawn(move || run_listener(server, queue_for_thread))
        .map_err(|e| format!("script HTTP server: failed to spawn listener thread: {e}"))?;
    Ok(HttpServer { queue, bind_addr })
}

#[cfg(not(target_arch = "wasm32"))]
fn run_listener(server: tiny_http::Server, queue: Queue) {
    use tiny_http::Method;

    for mut req in server.incoming_requests() {
        let path_full = req.url().to_string();
        let (path, query) = match path_full.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (path_full, String::new()),
        };
        let method = req.method().clone();

        let (code, body): (u16, ReplyBody) = match (&method, path.as_str()) {
            (Method::Get, "/") | (Method::Get, "/info") => (200, info_json().into()),
            (Method::Get, "/natives") => (200, list_natives_json().into()),
            (Method::Get, "/state") => relay(&queue, HttpPayload::State),
            (Method::Get, "/host-debug") => relay(&queue, HttpPayload::HostDebug),
            (Method::Get, "/engine-dump") => relay(&queue, HttpPayload::EngineDump),
            (Method::Get, "/level-assets") => relay(&queue, HttpPayload::LevelAssets),
            (Method::Get, "/script") => relay(&queue, HttpPayload::Script),
            (Method::Get, "/script/decompile") => {
                let class = query_param(&query, "class").map(str::to_string);
                relay(&queue, HttpPayload::Decompile { class })
            }
            (Method::Get, "/screenshot") => relay(
                &queue,
                HttpPayload::Screenshot(parse_screenshot_query(&query)),
            ),
            (Method::Post, "/native") => match read_json::<NativeCall>(&mut req) {
                Ok(c) => relay(
                    &queue,
                    HttpPayload::Native {
                        name: c.op,
                        args: c.args,
                        this: c.this,
                    },
                ),
                Err(e) => (400, serde_json::json!({"error": e}).into()),
            },
            (Method::Post, "/batch") => {
                #[derive(serde::Deserialize)]
                struct BatchBody {
                    calls: Vec<NativeCall>,
                }
                match read_json::<BatchBody>(&mut req) {
                    Ok(b) => relay(&queue, HttpPayload::Batch(b.calls)),
                    Err(e) => (400, serde_json::json!({"error": e}).into()),
                }
            }
            (Method::Post, "/console") => {
                #[derive(serde::Deserialize)]
                struct ConsoleBody {
                    command: String,
                }
                match read_json::<ConsoleBody>(&mut req) {
                    Ok(c) => relay(&queue, HttpPayload::Console(c.command)),
                    Err(e) => (400, serde_json::json!({"error": e}).into()),
                }
            }
            (Method::Post, "/command") => match read_json::<PlayerCommand>(&mut req) {
                Ok(c) => relay(&queue, HttpPayload::Command(c)),
                Err(e) => (400, serde_json::json!({"error": e}).into()),
            },
            (Method::Post, "/step-forward") => match parse_step_body(&mut req) {
                Ok(n) => relay(&queue, HttpPayload::StepForward { n }),
                Err(e) => (400, serde_json::json!({"error": e}).into()),
            },
            (Method::Post, "/step-back") => match parse_step_body(&mut req) {
                Ok(n) => relay(&queue, HttpPayload::StepBack { n }),
                Err(e) => (400, serde_json::json!({"error": e}).into()),
            },
            (Method::Post, "/go-to-frame") => {
                #[derive(serde::Deserialize)]
                struct GoToBody {
                    frame: u32,
                }
                match read_json::<GoToBody>(&mut req) {
                    Ok(b) => relay(&queue, HttpPayload::GoToFrame { target: b.frame }),
                    Err(e) => (400, serde_json::json!({"error": e}).into()),
                }
            }
            (Method::Post, "/set-paused") => {
                #[derive(serde::Deserialize)]
                struct SetPausedBody {
                    paused: bool,
                }
                match read_json::<SetPausedBody>(&mut req) {
                    Ok(b) => relay(&queue, HttpPayload::SetPaused { paused: b.paused }),
                    Err(e) => (400, serde_json::json!({"error": e}).into()),
                }
            }
            (Method::Get, "/get-replay") => relay(&queue, HttpPayload::GetReplay),
            (Method::Post, "/load-replay") => {
                #[derive(serde::Deserialize)]
                struct LoadReplayBody {
                    data: String,
                    #[serde(default)]
                    paused: bool,
                }
                match read_json::<LoadReplayBody>(&mut req) {
                    Ok(b) => relay(
                        &queue,
                        HttpPayload::LoadReplay {
                            data: b.data,
                            paused: b.paused,
                        },
                    ),
                    Err(e) => (400, serde_json::json!({"error": e}).into()),
                }
            }
            _ => (404, serde_json::json!({"error": "not found"}).into()),
        };

        let (content_type, bytes): (&[u8], Vec<u8>) = match body {
            ReplyBody::Json(v) => (
                &b"application/json"[..],
                serde_json::to_vec(&v)
                    .unwrap_or_else(|_| br#"{"error":"json encode failed"}"#.to_vec()),
            ),
            ReplyBody::Binary { content_type, data } => (content_type.as_bytes(), data),
        };
        let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type)
            .expect("static content-type header");
        let response = tiny_http::Response::from_data(bytes)
            .with_status_code(code)
            .with_header(header);
        if let Err(e) = req.respond(response) {
            tracing::warn!("script HTTP response failed: {e}");
        }
    }
}

/// Parse the body of `/step-forward` / `/step-back` into a tick count.
///
/// Accepts either a JSON object `{"n": N}` or an empty body (defaults
/// to `1`).  `N` must be a positive integer.
#[cfg(not(target_arch = "wasm32"))]
fn parse_step_body(req: &mut tiny_http::Request) -> Result<u32, String> {
    let mut body = String::new();
    std::io::Read::read_to_string(req.as_reader(), &mut body)
        .map_err(|e| format!("body read: {e}"))?;
    if body.trim().is_empty() {
        return Ok(1);
    }
    #[derive(serde::Deserialize)]
    struct StepBody {
        #[serde(default = "default_one")]
        n: u32,
    }
    fn default_one() -> u32 {
        1
    }
    let body: StepBody = serde_json::from_str(&body).map_err(|e| format!("bad json: {e}"))?;
    if body.n == 0 {
        return Err("n must be >= 1".into());
    }
    Ok(body.n)
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

#[cfg(not(target_arch = "wasm32"))]
fn read_json<T: serde::de::DeserializeOwned>(req: &mut tiny_http::Request) -> Result<T, String> {
    let mut body = String::new();
    std::io::Read::read_to_string(req.as_reader(), &mut body)
        .map_err(|e| format!("body read: {e}"))?;
    serde_json::from_str(&body).map_err(|e| format!("bad json: {e}"))
}

/// Send a payload to the game loop and wait for the reply.  Caps the
/// wait at 60 s so a wedged game doesn't hang the client forever.
#[cfg(not(target_arch = "wasm32"))]
fn relay(queue: &Queue, payload: HttpPayload) -> (u16, ReplyBody) {
    let (tx, rx) = mpsc::sync_channel::<Reply>(1);
    queue
        .lock()
        .expect("queue mutex poisoned")
        .push_back(HttpRequest {
            payload,
            response_tx: Responder::Channel(tx),
        });
    match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(Ok(body)) => (200, body),
        Ok(Err(msg)) => (400, serde_json::json!({"error": msg}).into()),
        Err(mpsc::RecvTimeoutError::Timeout) => (
            504,
            serde_json::json!({"error": "game loop did not process the request within 60s"}).into(),
        ),
        Err(mpsc::RecvTimeoutError::Disconnected) => (
            500,
            serde_json::json!({"error": "game loop dropped the response channel"}).into(),
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
            {"method": "POST", "path": "/step-forward",       "desc": "Run N engine ticks with --start-paused. Body {n: N} (default 1). Any modal dialog / popup / debriefing / sherwood report / pause-all queued before or during the step is dismissed silently; the reply includes `modals_dismissed`."},
            {"method": "POST", "path": "/step-back",          "desc": "Rewind N frames via the rewind buffer. Body {n: N} (default 1). Fails if target frame is older than the oldest retained snapshot."},
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
/// `--wait-for-command` idle phase, where only `load-replay` makes
/// sense.  Replies `503` to anything that needs engine state.
pub fn drain_pre_engine() {
    let Some(server) = GLOBAL.get() else { return };
    let pending: Vec<HttpRequest> = {
        let mut q = server.queue.lock().expect("queue mutex poisoned");
        q.drain(..).collect()
    };
    for req in pending {
        match req.payload {
            HttpPayload::LoadReplay { data, paused } => {
                let reply = decode_load_replay(&data, paused);
                req.response_tx.send(reply);
            }
            _ => req.response_tx.send(Err(
                "engine not ready — only `load-replay` / `info` work during --wait-for-command"
                    .into(),
            )),
        }
    }
}

/// Parse a `rhrec-…` / JSONL replay payload and stash it in
/// the pending slot.  Shared between the engine-dispatch path and
/// the wait-mode pre-engine drain.
fn decode_load_replay(data: &str, paused: bool) -> Reply {
    let trimmed = data.trim_start();
    let replay = if trimmed.starts_with(crate::replay_format::COMPACT_PREFIX) {
        let (hash, replay) = crate::replay_format::decode_compact(trimmed)
            .map_err(|e| format!("decode compact replay: {e}"))?;
        if hash != crate::replay_format::ENGINE_VERSION_HASH {
            tracing::warn!(
                "load-replay: replay was recorded on engine `{hash}`, \
                 current build is `{}` — desyncs possible",
                crate::replay_format::ENGINE_VERSION_HASH
            );
        }
        replay
    } else {
        engine_replay::ReplayData::from_reader(std::io::Cursor::new(trimmed.as_bytes()))
            .map_err(|e| format!("parse replay: {e}"))?
    };
    let frame_count = replay.frame_count();
    let seed = replay.header.rng_seed;
    set_pending_replay(PendingReplay {
        data: replay,
        paused,
    });
    Ok(ReplyBody::Json(serde_json::json!({
        "ok": true,
        "frames": frame_count,
        "rng_seed": seed,
        "paused": paused,
        "note": "pending — takes effect on next mission init (restart mission to apply)",
    })))
}

pub fn drain_global(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut crate::Host,
    assets: &LevelAssets,
    net: Option<&crate::multiplayer::NetChannels>,
) {
    let engine = &mut manager.engine;
    let Some(server) = GLOBAL.get() else { return };
    let pending: Vec<HttpRequest> = {
        let mut q = server.queue.lock().expect("queue mutex poisoned");
        q.drain(..).collect()
    };
    for req in pending {
        // `/screenshot` doesn't reply on the tick — it's deferred until
        // the frame is rendered.  Route to the pending-screenshot list
        // so the main loop's `screenshot_pre_render` / `…_capture_and_send`
        // pair can fulfil it; all other payloads dispatch synchronously
        // here and reply immediately.
        match req.payload {
            HttpPayload::Screenshot(request) => {
                pending_screenshots()
                    .lock()
                    .expect("screenshot queue poisoned")
                    .push(PendingScreenshot {
                        response_tx: req.response_tx,
                        request,
                    });
            }
            HttpPayload::StepForward { n } => {
                pending_steps()
                    .lock()
                    .expect("step queue poisoned")
                    .push(PendingStep {
                        response_tx: req.response_tx,
                        kind: StepKind::Forward { n },
                    });
            }
            HttpPayload::StepBack { n } => {
                pending_steps()
                    .lock()
                    .expect("step queue poisoned")
                    .push(PendingStep {
                        response_tx: req.response_tx,
                        kind: StepKind::Back { n },
                    });
            }
            HttpPayload::GoToFrame { target } => {
                pending_steps()
                    .lock()
                    .expect("step queue poisoned")
                    .push(PendingStep {
                        response_tx: req.response_tx,
                        kind: StepKind::GoToFrame { target },
                    });
            }
            HttpPayload::SetPaused { paused } => {
                pending_steps()
                    .lock()
                    .expect("step queue poisoned")
                    .push(PendingStep {
                        response_tx: req.response_tx,
                        kind: StepKind::SetPaused { paused },
                    });
            }
            HttpPayload::HostDebug => {
                req.response_tx.send(Ok(ReplyBody::Json(snapshot_host_debug(
                    engine, host, assets,
                ))));
            }
            other => {
                let reply = dispatch_in_engine(
                    other,
                    engine,
                    &mut host.engine_display,
                    assets,
                    &mut host.input,
                    &mut host.selected_view_element,
                    net,
                );
                req.response_tx.send(reply);
            }
        }
    }
}

fn dispatch_in_engine(
    payload: HttpPayload,
    engine: &mut Engine,
    display: &mut engine_api::HostDisplayState,
    assets: &LevelAssets,
    input: &mut engine_api::InputState,
    selected_view_element: &mut Option<engine_element::EntityId>,
    net: Option<&crate::multiplayer::NetChannels>,
) -> Reply {
    match payload {
        HttpPayload::Native { name, args, this } => engine
            .call_external_native_with_this(assets, &name, &args, this)
            .map(|v| ReplyBody::Json(serde_json::json!({"return": v}))),
        HttpPayload::Batch(calls) => {
            let mut results = Vec::with_capacity(calls.len());
            for c in calls {
                let r = engine.call_external_native_with_this(assets, &c.op, &c.args, c.this);
                results.push(match r {
                    Ok(v) => serde_json::json!({"return": v}),
                    Err(e) => serde_json::json!({"error": e}),
                });
            }
            Ok(ReplyBody::Json(serde_json::json!({"results": results})))
        }
        HttpPayload::Console(cmd) => {
            // The DevState is host-side and not in scope here; we use a
            // throwaway DevState since console cheats that mutate
            // `dev.debug.*` are hooked elsewhere via the in-game
            // overlay. Sim-affecting branches (CAMPAIGN, ARES, …) do
            // their work on the engine directly and don't read DevState
            // back.  See `console_dispatch.rs` for which commands fall
            // into which bucket.
            //
            // Route through `run_cheat_string` — the HTTP caller is
            // treated as the "WASM GUI" entry point, which always wants
            // the full dev cheat set regardless of `use_final`.
            let mut dev = engine_api::DevState::default();
            let resp = engine.run_cheat_string(assets, &mut dev, selected_view_element, &cmd);
            Ok(ReplyBody::Json(console_response_to_json(resp)))
        }
        HttpPayload::Command(cmd) => {
            // In multiplayer, route the command over the wire so every
            // peer applies it at the same `target_frame`.  The local
            // engine doesn't mutate here; the echo lands via
            // `drain_net_inputs` at `sim_frame + INPUT_DELAY_FRAMES`.
            if let Some(net) = net {
                net.send_input(cmd);
            } else {
                engine.apply_command(display, input, assets, &cmd);
            }
            Ok(ReplyBody::Json(serde_json::json!({"ok": true})))
        }
        HttpPayload::State => Ok(ReplyBody::Json(snapshot_state(engine))),
        HttpPayload::HostDebug => Err("host-debug must be routed via drain_global".into()),
        HttpPayload::EngineDump => engine_dump_json(engine)
            .map(ReplyBody::Json)
            .map_err(|e| format!("engine serialize: {e}")),
        HttpPayload::LevelAssets => level_assets_json(engine, assets)
            .map(ReplyBody::Json)
            .map_err(|e| format!("level assets serialize: {e}")),
        HttpPayload::Script => Ok(ReplyBody::Json(snapshot_script(engine))),
        HttpPayload::Decompile { class } => {
            Ok(ReplyBody::Json(decompile_script(engine, class.as_deref())))
        }
        // Routed through `drain_global`'s per-kind arm — should never
        // reach this generic dispatch path.
        HttpPayload::Screenshot(_) => Err("screenshot must be routed via drain_global".into()),
        HttpPayload::StepForward { .. }
        | HttpPayload::StepBack { .. }
        | HttpPayload::GoToFrame { .. }
        | HttpPayload::SetPaused { .. } => Err("step must be routed via drain_global".into()),
        HttpPayload::GetReplay => match get_current_replay() {
            Ok(content) => Ok(ReplyBody::Json(serde_json::json!({
                "content": content,
            }))),
            Err(e) => Err(e),
        },
        HttpPayload::LoadReplay { data, paused } => decode_load_replay(&data, paused),
    }
}

fn snapshot_state(engine: &Engine) -> serde_json::Value {
    let replay = replay_status().map(|s| {
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
    host: &crate::Host,
    assets: &LevelAssets,
) -> serde_json::Value {
    let selected_action = engine.selected_action_for_seat(host.local_seat);
    let selected_pc = engine.seat_selection(host.local_seat).first().copied();
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
    let last_preview_point = host.trajectory_preview_points.last().map(|point| {
        serde_json::json!({
            "position": point.position,
            "time": point.time,
        })
    });
    let bow_hover = match (selected_action, selected_pc, host.input.focused_entity_id) {
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
        "selection": engine.seat_selection(host.local_seat),
        "selected_pc": selected_pc_state,
        "valid_trajectory": host.valid_trajectory,
        "trajectory_preview_points_len": host.trajectory_preview_points.len(),
        "trajectory_preview_start": host.trajectory_preview_start,
        "trajectory_preview_last": last_preview_point,
        "trajectory_preview_layer": host.trajectory_preview_layer,
        "net_crumpled": host.net_crumpled,
        "time_no_mouse_move": host.time_no_mouse_move,
        "mouse_map_prev": host.mouse_map_prev,
        "trajectory_mark_count": host.trajectory_mark_count,
        "bow_hover": bow_hover,
        "input": {
            "focused_entity_id": host.input.focused_entity_id,
            "target_drag": host.input.target_drag,
            "double_status_bar_entity_id": host.input.double_status_bar_entity_id,
            "selected_layer": host.input.selected_layer,
            "selected_sector_idx": host.input.selected_sector_idx,
            "selected_patch_idx": host.input.selected_patch_idx,
            "hovered_door_idx": host.input.hovered_door_idx,
            "valid_position_for_move": host.input.valid_position_for_move,
            "mouse_opacity": host.input.mouse_opacity,
            "mouse_shadow_color": host.input.mouse_shadow_color,
            "left_mouse_down": host.input.left_mouse_down,
            "right_mouse_down": host.input.right_mouse_down,
            "is_dragging": host.input.is_dragging,
            "is_alt": host.input.is_alt,
        },
    })
}

fn bow_debug_ground_y_raw(point: engine_coordinates::WorldPoint3D) -> f32 {
    point.y
}

fn bow_debug_ground_y_projected(point: engine_coordinates::WorldPoint3D) -> f32 {
    point.to_map().y
}

fn cxx_sector_0_to_15_with_aspect(x: f32, y: f32, aspect_ratio: f32) -> u8 {
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
            "dy_raw_cxx": dy_raw,
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
            "square_distance_raw_cxx_y": square_distance_raw,
            "square_distance_projected_y_minus_z": square_distance_projected,
            "in_range_raw_cxx_y": square_distance_raw < radius_square,
            "in_range_projected_y_minus_z": square_distance_projected < radius_square,
            "dist_3d_raw_cxx_y": dist_3d_raw,
            "dist_3d_projected_y_minus_z": dist_3d_projected,
        },
        "direction": {
            "rust_iso_sector_raw_cxx_y": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_raw),
            "rust_iso_sector_projected_y_minus_z": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_projected),
            "cxx_get_sector_aspect_raw_cxx_y": cxx_sector_0_to_15_with_aspect(
                dx,
                dy_raw,
                engine_position_interface::ASPECT_RATIO,
            ),
            "cxx_get_sector_aspect_projected_y_minus_z": cxx_sector_0_to_15_with_aspect(
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
            "dy_raw_cxx": dy_raw,
            "dy_projected_y_minus_z": dy_projected,
            "rust_iso_sector_raw_cxx_y": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_raw),
            "rust_iso_sector_projected_y_minus_z": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_projected),
            "cxx_get_sector_aspect_raw_cxx_y": cxx_sector_0_to_15_with_aspect(
                dx,
                dy_raw,
                engine_position_interface::ASPECT_RATIO,
            ),
            "cxx_get_sector_aspect_projected_y_minus_z": cxx_sector_0_to_15_with_aspect(
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
            "posture": shooter.element_data().posture,
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
                "lines": assets.level_grid.lines.len(),
                "sectors": assets.level_grid.sectors.len(),
                "masks": assets.level_grid.masks.len(),
                "jump_lines": assets.level_grid.jump_lines.len(),
                "blocks": assets.level_grid.blocks.len(),
                "layers": assets.level_grid.layers.len(),
                "level_repulsive_points": assets.level_grid.level_repulsive_points.len(),
                "shadow_data": assets.level_grid.shadow_data.len(),
            },
            "pathfinder_graph": {
                "nodes": assets.pathfinder_graph.nodes.len(),
                "layers": assets.pathfinder_graph.layers.len(),
                "links": assets.pathfinder_graph.static_data.links.len(),
                "link_configs": assets.pathfinder_graph.static_data.link_configs.len(),
                "move_layers": assets.pathfinder_graph.static_data.move_layers.len(),
                "alternative_move_layers": assets.pathfinder_graph.static_data.alternative_move_layers.len(),
            },
            "profiles": {
                "characters": assets.profile_manager.characters.len(),
                "soldiers": assets.profile_manager.soldiers.len(),
                "civilians": assets.profile_manager.civilians.len(),
                "hth_weapons": assets.profile_manager.hth_weapons.len(),
                "bows": assets.profile_manager.bows.len(),
                "missions": assets.profile_manager.missions.len(),
            },
            "mission_script_programs": assets.mission_script_programs.len(),
            "hiking_paths": assets.hiking_paths.len(),
            "static_sight_obstacles": assets.static_sight_obstacles.len(),
            "accessory_sprite_prototypes": assets.accessory_sprite_prototypes.len(),
            "water_zones": assets.water_zones.zones.len(),
            "material_sectors": assets.material_sectors.sectors.len(),
            "script_locations": assets.script_location_count,
            "script_points": assets.script_point_count,
            "script_buildings": assets.script_building_count,
            "script_hiking_paths": assets.script_hiking_path_count,
        }),
    );
    root.insert(
        "pixel_opacity_attached".into(),
        serde_json::json!(assets.pixel_opacity.is_some()),
    );
    insert_json(&mut root, "fast_grid_runtime", engine.fast_grid())?;

    let mut asset = serde_json::Map::new();
    insert_json(&mut asset, "sprite_scriptor", &*assets.sprite_scriptor)?;
    insert_json(&mut asset, "level_grid", &*assets.level_grid)?;
    insert_json(&mut asset, "pathfinder_graph", &*assets.pathfinder_graph)?;
    insert_json(&mut asset, "hiking_paths", &*assets.hiking_paths)?;
    insert_json(&mut asset, "profile_manager", &*assets.profile_manager)?;
    insert_json(&mut asset, "bank_signature", &assets.bank_signature)?;
    insert_json(
        &mut asset,
        "mission_script_programs",
        &*assets.mission_script_programs,
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
        &assets.exclamation_durations,
    )?;
    insert_json(&mut asset, "source_durations", &assets.source_durations)?;
    insert_json(
        &mut asset,
        "sound_source_required_ids",
        &assets.sound_source_required_ids,
    )?;
    insert_json(
        &mut asset,
        "patch_entity_handles",
        &assets.patch_entity_handles,
    )?;
    insert_json(&mut asset, "scroll_entity_ids", &assets.scroll_entity_ids)?;
    insert_json(
        &mut asset,
        "all_soldier_entity_ids",
        &assets.all_soldier_entity_ids,
    )?;
    insert_json(
        &mut asset,
        "soldier_subordinate_ids",
        &assets.soldier_subordinate_ids,
    )?;
    insert_json(&mut asset, "water_zones", &assets.water_zones)?;
    insert_json(&mut asset, "material_sectors", &assets.material_sectors)?;
    insert_json(
        &mut asset,
        "static_sight_obstacles",
        &*assets.static_sight_obstacles,
    )?;
    insert_json(
        &mut asset,
        "script_location_count",
        &assets.script_location_count,
    )?;
    insert_json(&mut asset, "script_point_count", &assets.script_point_count)?;
    insert_json(
        &mut asset,
        "script_location_positions",
        &assets.script_location_positions,
    )?;
    insert_json(
        &mut asset,
        "script_location_layers",
        &assets.script_location_layers,
    )?;
    insert_json(
        &mut asset,
        "script_location_sectors",
        &assets.script_location_sectors,
    )?;
    insert_json(
        &mut asset,
        "script_building_count",
        &assets.script_building_count,
    )?;
    insert_json(
        &mut asset,
        "script_hiking_path_count",
        &assets.script_hiking_path_count,
    )?;
    insert_json(
        &mut asset,
        "script_zone_grid_indices",
        &assets.script_zone_grid_indices,
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
// Replay sideband — current recording path + pending replay to load
// ──────────────────────────────────────────────────────────────────

/// A replay queued by `load-replay`, consumed by
/// [`crate::game_session::init_replay_and_rollback`] on next mission start.
pub struct PendingReplay {
    pub data: engine_replay::ReplayData,
    /// Whether the caller asked for the mission to start paused so they
    /// can step through frame-by-frame with `step-forward`.
    pub paused: bool,
}

fn pending_replay_slot() -> &'static Mutex<Option<PendingReplay>> {
    static SLOT: OnceLock<Mutex<Option<PendingReplay>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Install a `PendingReplay`.  Overwrites any previous pending slot —
/// latest wins, caller is expected to only queue one at a time.
pub fn set_pending_replay(p: PendingReplay) {
    *pending_replay_slot()
        .lock()
        .expect("pending replay poisoned") = Some(p);
}

/// Take the pending replay, leaving the slot empty.
pub fn take_pending_replay() -> Option<PendingReplay> {
    pending_replay_slot()
        .lock()
        .expect("pending replay poisoned")
        .take()
}

/// Peek at the pending replay's mission id (the `.rhm` filename
/// stamped into the replay header, e.g. `"Dem_Lei_MP"`) without
/// consuming the slot.  Used by `--wait-for-command` to pick which
/// mission to launch; [`take_pending_replay`] consumes the slot
/// later before mission startup.
pub fn peek_pending_replay_mission_id() -> Option<String> {
    pending_replay_slot()
        .lock()
        .expect("pending replay poisoned")
        .as_ref()
        .map(|p| p.data.header.mission_id.clone())
}

/// Shared handle to the replay-recording mirror buffer.  Cloned into
/// [`crate::game_session::init_replay_and_rollback`]; wrapped inside a
/// tee-writer so every byte the `ReplayRecorder` emits is mirrored
/// here alongside the real file sink (native) or *instead of* one
/// (wasm — no filesystem).  `get-replay` serializes this buffer
/// straight back to the caller.
pub type ReplayBuffer = Arc<Mutex<Vec<u8>>>;

fn replay_buffer_slot() -> &'static ReplayBuffer {
    static SLOT: OnceLock<ReplayBuffer> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

/// Global mirror of the active recorder's byte stream.  Cleared by
/// [`reset_replay_buffer`] at mission init.  `get-replay` reads a
/// snapshot of this buffer without touching the filesystem.
pub fn replay_buffer_handle() -> ReplayBuffer {
    replay_buffer_slot().clone()
}

/// Clear the mirror buffer — call from [`crate::game_session`] just
/// before constructing a new recorder, so the first bytes in the new
/// buffer are that recorder's freshly-written header.
pub fn reset_replay_buffer() {
    replay_buffer_slot()
        .lock()
        .expect("replay buffer poisoned")
        .clear();
}

/// Snapshot of the current recorder's byte stream.  Empty `Vec` when
/// no recording is active (or when it's been explicitly reset and no
/// frames have been written yet).
pub fn replay_buffer_snapshot() -> Vec<u8> {
    replay_buffer_slot()
        .lock()
        .expect("replay buffer poisoned")
        .clone()
}

/// Per-frame replay-playback status surfaced to the script-RPC
/// `state` endpoint so JS timeline UIs can render a playhead without
/// polling a dedicated endpoint.  `None` when no replay is playing
/// (live gameplay).  Updated once per frame by
/// [`crate::game_session::publish_replay_status`].
#[derive(Clone, Copy, Debug)]
pub struct ReplayStatus {
    pub frame: u32,
    pub total: u32,
    pub paused: bool,
}

fn replay_status_slot() -> &'static Mutex<Option<ReplayStatus>> {
    static SLOT: OnceLock<Mutex<Option<ReplayStatus>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Publish (or clear, with `None`) the live replay-playback status.
/// Called from the game loop; cleared on mission end / when live
/// gameplay resumes.
pub fn set_replay_status(s: Option<ReplayStatus>) {
    *replay_status_slot().lock().expect("replay status poisoned") = s;
}

/// Most-recent [`ReplayStatus`] published by the game loop, or `None`
/// if no replay is currently playing.
pub fn replay_status() -> Option<ReplayStatus> {
    *replay_status_slot().lock().expect("replay status poisoned")
}

/// `GET /get-replay` backing: parse the active recorder's JSONL
/// buffer and return a compact `rhrec-{hash}-{base64}` share string.
///
/// The share string is ~10× smaller than the raw JSONL (zstd-bitcode
/// over the structured command stream) and is ready to paste into a
/// URL as-is — no additional encoding on the JS side.
fn get_current_replay() -> Result<String, String> {
    let bytes = replay_buffer_snapshot();
    if bytes.is_empty() {
        return Err("no active replay recording".into());
    }
    let data = engine_replay::ReplayData::from_reader(std::io::Cursor::new(&bytes[..]))
        .map_err(|e| format!("parse mirrored replay buffer: {e}"))?;
    crate::replay_format::encode_compact(&data, crate::replay_format::ENGINE_VERSION_HASH)
        .map_err(|e| format!("encode compact replay: {e}"))
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
    pub fn respond_err(self, msg: impl Into<String>) {
        self.response_tx.send(Err(msg.into()));
    }
}

fn pending_screenshots() -> &'static Mutex<Vec<PendingScreenshot>> {
    static SLOT: OnceLock<Mutex<Vec<PendingScreenshot>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Vec::new()))
}

// ──────────────────────────────────────────────────────────────────
// Step-forward / step-back pipeline
// ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// Run `n` ticks forward from the current frame.
    Forward { n: u32 },
    /// Rewind `n` frames from the current frame.
    Back { n: u32 },
    /// Absolute seek — no-op if `target == sim_frame`, decomposes into
    /// a forward or back step otherwise.  Replay scrubbing uses this.
    GoToFrame { target: u32 },
    /// Toggle the mission loop's manual pause flag. Queued with
    /// scrubbing so pause/play and seek requests apply in caller order.
    SetPaused { paused: bool },
}

/// A step-forward / step-back request waiting for the main loop to
/// drive the engine.  The main loop is expected to
/// [`take_pending_steps`] once per frame and, for each request, either:
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

    pub fn respond_err(self, msg: impl Into<String>) {
        self.response_tx.send(Err(msg.into()));
    }
}

fn pending_steps() -> &'static Mutex<Vec<PendingStep>> {
    static SLOT: OnceLock<Mutex<Vec<PendingStep>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Vec::new()))
}

/// Drain every step request queued since the last call.  The main
/// loop calls this once per frame immediately before the tick gate,
/// runs each step synchronously with the full rollback / rewind /
/// replay bookkeeping, and replies via the `PendingStep` handle.
pub fn take_pending_steps() -> Vec<PendingStep> {
    std::mem::take(&mut *pending_steps().lock().expect("step queue poisoned"))
}

/// Drain every screenshot request queued since the last call.  Safe to
/// call from the main render loop once per frame — returns an empty
/// `Vec` when nothing is pending.
pub fn take_pending_screenshots(sim_frame: u32) -> Vec<PendingScreenshot> {
    let mut queue = pending_screenshots()
        .lock()
        .expect("screenshot queue poisoned");
    let requests = std::mem::take(&mut *queue);
    let (ready, waiting) = requests
        .into_iter()
        .partition(|pending| pending.request.frame.is_none_or(|frame| sim_frame >= frame));
    *queue = waiting;
    ready
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

    let (target_w, target_h) = screenshot_target_dimensions(used_w, used_h, req)?;

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
            .map_err(|e| format!("png header: {e}"))?;
        writer
            .write_image_data(pixels)
            .map_err(|e| format!("png data: {e}"))?;
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

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

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

fn console_response_to_json(resp: engine_api::ConsoleResponse) -> serde_json::Value {
    use engine_api::ConsoleResponse as R;

    match resp {
        R::Ok(msg) => serde_json::json!({"kind": "ok", "message": msg}),
        R::Unknown => serde_json::json!({"kind": "unknown"}),
        R::NotImplemented(name) => {
            serde_json::json!({"kind": "not_implemented", "command": name})
        }
        // Anything host-driven (CAMPAIGN load, ARES advance with side-effects, …)
        // falls into this catch-all.  We surface the variant name as a
        // hint — the actual host-side dispatch isn't reachable from here.
        other => serde_json::json!({"kind": "host_followup", "variant": format!("{other:?}")}),
    }
}

fn snapshot_script(engine: &Engine) -> serde_json::Value {
    let Some(script) = engine.mission_script() else {
        return serde_json::json!({"loaded": false});
    };
    let scb = script.manager.scb();
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
        "actor_instances": script.actor_instances.len(),
        "zone_instances": script.zone_instances.len(),
        "target_instances": script.target_instances.len(),
        "scroll_instances": script.scroll_instances.len(),
        "waypoint_instances": script.waypoint_instances.len(),
        "classes": classes,
    })
}

fn decompile_script(engine: &Engine, class: Option<&str>) -> serde_json::Value {
    let Some(script) = engine.mission_script() else {
        return serde_json::json!({"error": "no mission script loaded"});
    };
    let scb = script.manager.scb();
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
// Requests land on the same `GLOBAL.queue` as the native transport,
// drain on the game tick, and resolve the Promise through an internal
// one-shot channel.

#[cfg(target_arch = "wasm32")]
pub mod wasm_rpc {
    use super::{
        GLOBAL, HttpPayload, HttpRequest, NativeCall, Reply, ReplyBody, Responder,
        ScreenshotRequest,
    };
    use wasm_bindgen::JsValue;

    fn reply_to_js(reply: Reply) -> Result<JsValue, JsValue> {
        match reply {
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
            Err(message) => Err(JsValue::from_str(&message)),
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
        let req: Req = serde_wasm_bindgen::from_value(request)
            .map_err(|e| JsValue::from_str(&format!("bad request: {e}")))?;
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
        let payload = decode_request(&req.method, req.params).map_err(|e| JsValue::from_str(&e))?;
        let server = GLOBAL
            .get()
            .ok_or_else(|| JsValue::from_str("RPC bridge not initialized"))?;
        let (tx, rx) = async_channel::bounded(1);
        server
            .queue
            .lock()
            .expect("queue mutex poisoned")
            .push_back(HttpRequest {
                payload,
                response_tx: Responder::Wasm(tx),
            });
        let reply = rx
            .recv()
            .await
            .map_err(|e| JsValue::from_str(&format!("RPC response dropped: {e}")))?;
        reply_to_js(reply)
    }

    fn decode_request(method: &str, params: serde_json::Value) -> Result<HttpPayload, String> {
        #[derive(serde::Deserialize, Default)]
        struct StepBody {
            #[serde(default = "one")]
            n: u32,
        }
        fn one() -> u32 {
            1
        }

        match method {
            "script" => Ok(HttpPayload::Script),
            "state" => Ok(HttpPayload::State),
            "host-debug" => Ok(HttpPayload::HostDebug),
            "level-assets" => Ok(HttpPayload::LevelAssets),
            "decompile" => {
                #[derive(serde::Deserialize, Default)]
                #[serde(default)]
                struct D {
                    class: Option<String>,
                }
                let d: D = if params.is_null() {
                    D::default()
                } else {
                    serde_json::from_value(params).map_err(|e| format!("decompile params: {e}"))?
                };
                Ok(HttpPayload::Decompile { class: d.class })
            }
            "native" => {
                let c: NativeCall =
                    serde_json::from_value(params).map_err(|e| format!("native params: {e}"))?;
                Ok(HttpPayload::Native {
                    name: c.op,
                    args: c.args,
                    this: c.this,
                })
            }
            "batch" => {
                #[derive(serde::Deserialize)]
                struct B {
                    calls: Vec<NativeCall>,
                }
                let b: B =
                    serde_json::from_value(params).map_err(|e| format!("batch params: {e}"))?;
                Ok(HttpPayload::Batch(b.calls))
            }
            "console" => {
                #[derive(serde::Deserialize)]
                struct C {
                    command: String,
                }
                let c: C =
                    serde_json::from_value(params).map_err(|e| format!("console params: {e}"))?;
                Ok(HttpPayload::Console(c.command))
            }
            "command" => {
                let cmd: PlayerCommand =
                    serde_json::from_value(params).map_err(|e| format!("command params: {e}"))?;
                Ok(HttpPayload::Command(cmd))
            }
            "screenshot" => {
                let ss: ScreenshotRequest = if params.is_null() {
                    ScreenshotRequest::default()
                } else {
                    serde_json::from_value(params).map_err(|e| format!("screenshot params: {e}"))?
                };
                Ok(HttpPayload::Screenshot(ss))
            }
            "step-forward" => {
                let s: StepBody = if params.is_null() {
                    StepBody::default()
                } else {
                    serde_json::from_value(params)
                        .map_err(|e| format!("step-forward params: {e}"))?
                };
                if s.n == 0 {
                    return Err("n must be >= 1".into());
                }
                Ok(HttpPayload::StepForward { n: s.n })
            }
            "step-back" => {
                let s: StepBody = if params.is_null() {
                    StepBody::default()
                } else {
                    serde_json::from_value(params).map_err(|e| format!("step-back params: {e}"))?
                };
                if s.n == 0 {
                    return Err("n must be >= 1".into());
                }
                Ok(HttpPayload::StepBack { n: s.n })
            }
            "go-to-frame" => {
                #[derive(serde::Deserialize)]
                struct G {
                    frame: u32,
                }
                let g: G = serde_json::from_value(params)
                    .map_err(|e| format!("go-to-frame params: {e}"))?;
                Ok(HttpPayload::GoToFrame { target: g.frame })
            }
            "set-paused" => {
                #[derive(serde::Deserialize)]
                struct P {
                    paused: bool,
                }
                let p: P = serde_json::from_value(params)
                    .map_err(|e| format!("set-paused params: {e}"))?;
                Ok(HttpPayload::SetPaused { paused: p.paused })
            }
            "get-replay" => Ok(HttpPayload::GetReplay),
            "load-replay" => {
                #[derive(serde::Deserialize)]
                struct L {
                    data: String,
                    #[serde(default)]
                    paused: bool,
                }
                let l: L = serde_json::from_value(params)
                    .map_err(|e| format!("load-replay params: {e}"))?;
                Ok(HttpPayload::LoadReplay {
                    data: l.data,
                    paused: l.paused,
                })
            }
            other => Err(format!("unknown method: {other}")),
        }
    }
}
