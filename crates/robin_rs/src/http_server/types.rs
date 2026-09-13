//! RPC request/reply payload types and their internal routing classification.

use super::*;

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
    pub(super) fn admit_unless_deferred(&self) -> bool {
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
pub(super) enum QueryRequest {
    State,
    EngineDump,
    LevelAssets,
    Script,
    Decompile { class: Option<String> },
}

pub(super) enum CommandRequest {
    Native {
        name: String,
        args: Vec<i32>,
        this: Option<i32>,
    },
    Batch(Vec<NativeCall>),
    Console(String),
    Player(PlayerCommand),
}

pub(super) enum DeferredRequest {
    Step(StepKind),
    Screenshot(ScreenshotRequest),
}

pub(super) enum ProcessRequest {
    ExportReplay,
    LoadReplay { data: String, paused: bool },
}

pub(super) enum RoutedRequest {
    Query(QueryRequest),
    HostDebug,
    Command(CommandRequest),
    Deferred(DeferredRequest),
    Process(ProcessRequest),
}

impl HttpPayload {
    pub(super) fn classify(self) -> RoutedRequest {
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

pub(super) fn ranked_input_taint(payload: &HttpPayload) -> Option<InputTaintKind> {
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
