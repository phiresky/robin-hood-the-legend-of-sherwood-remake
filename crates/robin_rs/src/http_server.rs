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
//!   | GET    | `/script`           | —                                            | mission-script class & function listing                |
//!   | GET    | `/script/decompile` | `?class=<name>` (optional)                   | `{source: "..."}` — pseudocode for one or all classes  |
//!   | POST   | `/native`           | `{op, args, this?}`                          | `{return}` or `{error}`                                |
//!   | POST   | `/batch`            | `{calls: [{op, args, this?}]}`               | `{results: [...]}`                                     |
//!   | POST   | `/console`          | `{command: "..."}`                           | `{kind, message?}`                                     |
//!   | POST   | `/command`          | externally-tagged `PlayerCommand` JSON       | `{ok: true}` or `{error}`                              |
//!   | GET    | `/screenshot`       | `?w=&h=&hide_ui=&view_cones=&pc_sight=&…`    | `image/png` of the next rendered frame                 |
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
use robin_engine::player_command::PlayerCommand;

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
    /// `GET /engine-dump` — full serialized engine for ad-hoc debug.
    EngineDump,
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
    /// Output width bound. Used with `height` as an aspect-preserving maximum.
    pub width: Option<u16>,
    /// Output height bound. Used with `width` as an aspect-preserving maximum.
    pub height: Option<u16>,
    /// Crop the bottom HUD panel before encoding.
    pub hide_ui: bool,
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
    /// Optional `script_this` override for the call (overrides the
    /// per-request `this` field on the GameHost for the duration of the
    /// dispatch, restored after).
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
            (Method::Get, "/engine-dump") => relay(&queue, HttpPayload::EngineDump),
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
        width: query_param(query, "w").and_then(|s| s.parse().ok()),
        height: query_param(query, "h").and_then(|s| s.parse().ok()),
        hide_ui: query_flag(query, "hide_ui").unwrap_or(false),
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
            {"method": "GET",  "path": "/script",             "desc": "mission-script class & function listing"},
            {"method": "GET",  "path": "/script/decompile",   "desc": "decompile to TypeScript-like pseudocode (?class=Foo)"},
            {"method": "POST", "path": "/native",             "desc": "invoke one native: {op, args, this?}"},
            {"method": "POST", "path": "/batch",              "desc": "invoke many natives on one tick: {calls: [{op, args, this?}]}"},
            {"method": "POST", "path": "/console",            "desc": "run a debug-console command: {command: '...'}"},
            {"method": "POST", "path": "/command",            "desc": "apply a PlayerCommand (externally-tagged JSON enum)"},
            {"method": "GET",  "path": "/screenshot",         "desc": "PNG of next rendered frame. Query: w, h (aspect-preserving max bounds), hide_ui, view_cones, pc_sight, motion_graph, all_obstacles, elevation, noise, sound_source, actor_info, script_zones, door, projection_areas, railroad, probability, company_number, combat_energy, light_zones, animation_lines, seek_points, fps, sprite_masks, entity_ids (bool flags)"},
            {"method": "POST", "path": "/step-forward",       "desc": "Run N engine ticks with --start-paused. Body {n: N} (default 1). Any modal dialog / popup / debriefing / sherwood report / pause-all queued before or during the step is dismissed silently; the reply includes `modals_dismissed`."},
            {"method": "POST", "path": "/step-back",          "desc": "Rewind N frames via the rewind buffer. Body {n: N} (default 1). Fails if target frame is older than the oldest retained snapshot."},
        ],
    })
}

fn list_natives_json() -> serde_json::Value {
    let mut entries = Vec::new();
    for i in 0u32..512 {
        if let Ok(n) = robin_engine::natives::NativeFn::try_from(i) {
            let name: &'static str = n.into();
            let sig = robin_engine::natives::native_signature_by_name(name);
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

/// Parse a `rhrec-…` / legacy JSONL replay payload and stash it in
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
        robin_engine::replay::ReplayData::from_reader(std::io::Cursor::new(trimmed.as_bytes()))
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
    manager: &mut robin_engine::engine_manager::EngineManager,
    display: &mut robin_engine::engine::HostDisplayState,
    assets: &LevelAssets,
    input: &mut robin_engine::engine::InputState,
    selected_view_element: &mut Option<robin_engine::element::EntityId>,
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
            other => {
                let reply = dispatch_in_engine(
                    other,
                    engine,
                    display,
                    assets,
                    input,
                    selected_view_element,
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
    display: &mut robin_engine::engine::HostDisplayState,
    assets: &LevelAssets,
    input: &mut robin_engine::engine::InputState,
    selected_view_element: &mut Option<robin_engine::element::EntityId>,
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
            let mut dev = robin_engine::engine::DevState::default();
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
        HttpPayload::EngineDump => engine_dump_json(engine)
            .map(ReplyBody::Json)
            .map_err(|e| format!("engine serialize: {e}")),
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

fn engine_dump_json(engine: &Engine) -> Result<serde_json::Value, String> {
    crate::json_value::to_json_value(engine).map_err(|e| e.to_string())
}

// ──────────────────────────────────────────────────────────────────
// Replay sideband — current recording path + pending replay to load
// ──────────────────────────────────────────────────────────────────

/// A replay queued by `load-replay`, consumed by
/// [`crate::game_session::init_replay_and_rollback`] on next mission start.
pub struct PendingReplay {
    pub data: robin_engine::replay::ReplayData,
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
    let data = robin_engine::replay::ReplayData::from_reader(std::io::Cursor::new(&bytes[..]))
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
    /// The client's requested dev-flag overrides for this screenshot.
    pub fn flags(&self) -> &ScreenshotFlags {
        &self.request.flags
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
pub fn take_pending_screenshots() -> Vec<PendingScreenshot> {
    std::mem::take(
        &mut *pending_screenshots()
            .lock()
            .expect("screenshot queue poisoned"),
    )
}

/// Merge a request's `Some(x)` overrides onto `debug`, mutating in
/// place.  Apply this to a **cloned** `DevState` so the live state
/// stays untouched — the caller keeps the original and passes the
/// clone to `render_frame`.
pub fn apply_screenshot_flags(
    debug: &mut robin_engine::engine::DebugFlags,
    flags: &ScreenshotFlags,
) {
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
    use robin_engine::engine::PANNEL_HEIGHT;

    // Optional bottom-panel crop: strip the HUD strip before any resize.
    let (src, mut used_w, mut used_h) = if req.hide_ui && src_h > PANNEL_HEIGHT as u32 {
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
}

fn console_response_to_json(resp: robin_engine::engine::ConsoleResponse) -> serde_json::Value {
    use robin_engine::engine::ConsoleResponse as R;
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
        let scb_one = robin_engine::scb::ScbFile {
            version: scb.version,
            classes: vec![c.clone()],
        };
        robin_assets::decompile::decompile(&scb_one)
    } else {
        robin_assets::decompile::decompile(scb)
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
    use robin_engine::player_command::PlayerCommand;
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
