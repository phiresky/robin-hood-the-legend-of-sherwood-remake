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
    decompile_script, engine_dump_json, frame_console_response_to_json, level_assets_json,
    snapshot_host_debug, snapshot_script, snapshot_state,
};
#[cfg(any(feature = "script-rpc", target_arch = "wasm32"))]
use crate::http_server::diagnostics::{info_json, list_natives_json};
pub use crate::http_server::screenshot::apply_screenshot_flags;
use crate::http_server::screenshot::{can_capture_presented_ui, encode_png};
use robin_engine::element as engine_element;
use robin_engine::engine as engine_api;
use robin_engine::player_command::{DialogResult, FrameCommands, ModalKind, PlayerCommand};
use robin_engine::replay_rankability::InputTaintKind;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
use std::thread;

use robin_engine::engine::{Engine, LevelAssets};

/// Default port. Reasonably uncommon and easy to remember; change with
/// `--http-server <port>` or set 0 to disable.
pub const DEFAULT_PORT: u16 = 17640;

pub mod diagnostics;
#[cfg(any(test, all(feature = "script-rpc", not(target_arch = "wasm32"))))]
pub mod query;
pub mod screenshot;

mod dispatch;
mod error;
mod pending;
mod transport;
mod types;
use dispatch::drain_pre_engine;
pub use error::{RpcError, RpcErrorKind};
pub use pending::{PendingScreenshot, PendingStep};
#[cfg(target_arch = "wasm32")]
use transport::BROWSER_QUEUE;
pub use transport::HttpTransport;
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
use transport::browser_rejection_reason;
#[cfg(all(any(feature = "script-rpc", test), not(target_arch = "wasm32")))]
use transport::relay;
#[cfg(target_arch = "wasm32")]
use transport::resolve_deferred_reply;
use transport::{HttpServer, Queue};
use types::{
    CommandRequest, DeferredRequest, ProcessRequest, QueryRequest, RoutedRequest,
    ranked_input_taint,
};
pub use types::{
    HttpModalDismissal, HttpPayload, HttpRequest, NativeCall, ReplayStatus, Reply, ReplyBody,
    ScreenshotFlags, ScreenshotRequest, StepKind, StepModalPolicy, StepRequest,
};

#[cfg(test)]
mod tests;
#[cfg(target_arch = "wasm32")]
pub mod wasm_rpc;

mod request_lifetime;
pub use request_lifetime::Responder;

mod ingress;
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
mod native_routes;
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
mod native_transport;
#[cfg(any(test, feature = "script-rpc", target_arch = "wasm32"))]
mod request_decode;
use ingress::RequestRouter;
pub use ingress::SessionIngress;
#[cfg(all(feature = "script-rpc", not(target_arch = "wasm32")))]
use native_transport::NativeRequest;
