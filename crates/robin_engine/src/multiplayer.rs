//! Multiplayer wire-format types and channel plumbing.
//!
//! This module defines the platform-pure layer of multiplayer
//! infrastructure: the wire-format enums (`NetMsg`, `NetEvent`,
//! `NetOutbound`), the cross-thread channel bundle (`NetChannels`), and
//! the protocol constants.  The actual transport (websocket I/O via
//! `tungstenite` on native, `web_sys::WebSocket` on wasm) lives in
//! `robin_rs::multiplayer::{native, wasm}` and feeds events into these
//! channels.
//!
//! `EngineManager` (this crate) owns a `NetChannels` and uses it to
//! route locally-sourced player commands over the wire and drain
//! peer-sourced inputs back into the engine at the correct frames.

use crate::engine::Engine;
use crate::player_command::{DialogResult, ModalKind, PlayerCommand, PlayerId, PlayerInput};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

/// Cross-thread snapshot of the local game loop's current sim frame.
///
/// Updated by the game loop at the top of every tick (just after
/// rewind/auto-replay accounting).  Read by the server's broadcast
/// pump and per-peer reader threads to stamp `BroadcastInput` with a
/// fresh `target_frame` so every peer applies the input at the same
/// frame.  An `Arc<AtomicU32>` is the simplest thread-safe handoff —
/// the rate is one update per tick (25 Hz) and reads happen at most
/// per inbound input frame.
pub type FrameCursor = Arc<AtomicU32>;

/// Shared encoded initial-state snapshot offered by the host to joining peers.
///
/// Encoding once at publication time guarantees that every peer admitted at
/// this boundary receives byte-identical authoritative state.
pub type InitialSnapshot = Arc<Mutex<Option<(u32, Vec<u8>)>>>;

/// Make a new [`FrameCursor`] starting at frame 0.
pub fn new_frame_cursor() -> FrameCursor {
    Arc::new(AtomicU32::new(0))
}

/// Number of frames of "input delay" the server adds when stamping
/// peer inputs with a target frame.  At 25 Hz this is ~80 ms.  The rollback path picks
/// up the slack on slower links by rewinding when an input arrives
/// late.  Tuneable; mirrors the `MAX_INPUT_DELAY` constant in classic
/// GGPO-style netcode.
pub const INPUT_DELAY_FRAMES: u32 = 2;

/// Wire-format protocol version. Bump on any breaking change to [`NetMsg`] or
/// an engine snapshot carried by it. Both sides exchange this in the
/// handshake; mismatches abort the connection.
pub const NET_PROTOCOL_VERSION: u32 = 29;

/// Default TCP port for the multiplayer server.
pub const DEFAULT_PORT: u16 = 7878;

/// Frame cadence at which the host samples its engine state hash and
/// broadcasts it for clients to verify against.  Matches the replay
/// recorder's `frame % 25 == 0` cadence (one hash per simulated
/// second at 25 Hz) so the same sampling point is reused.
pub const STATE_HASH_INTERVAL: u32 = 25;

/// Unpredictable identity for one host process's multiplayer mission session.
///
/// Modal traffic carries this value so a delayed packet from an earlier host
/// lifecycle can never resolve a UI surface in a replacement session.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct MultiplayerSessionId(pub [u8; 16]);

/// Stable identity for a host-authored outer-mission transition.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct SnapshotTransitionId {
    pub session_id: MultiplayerSessionId,
    pub sequence: u64,
}

/// Exact authoritative state retained by every participant before a
/// host-authored outer-mission transition is committed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub enum SnapshotTransitionPayload {
    Save {
        mission_id: u32,
        save_bytes: Vec<u8>,
    },
    CampaignExit {
        exit_code: crate::game_operation::GameCode,
        engine_bytes: Vec<u8>,
    },
}

/// Stable identity for one occurrence of a multiplayer modal.
///
/// `opened_frame` identifies the authoritative timeline boundary at which the
/// modal appeared. `occurrence` distinguishes repeated instances of the same
/// [`ModalKind`], including repeated unkeyed Sherwood reports.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct ModalInstanceId {
    pub session_id: MultiplayerSessionId,
    pub opened_frame: u32,
    pub occurrence: u64,
}

/// Client request retained for host presentation. Requests are advisory and
/// never resolve a modal without a later [`NetMsg::ModalDecision`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibleModalRequest {
    pub from: PlayerId,
    pub instance: ModalInstanceId,
    pub kind: ModalKind,
    pub result: DialogResult,
    pub requested_frame: u32,
}

#[derive(Debug)]
struct ModalOccurrenceState {
    kind: ModalKind,
    next_occurrence: u64,
    active: Option<ModalInstanceId>,
}

#[derive(Debug, Default)]
struct ModalSyncState {
    session_id: Option<MultiplayerSessionId>,
    occurrences: Vec<ModalOccurrenceState>,
    inbox: std::collections::VecDeque<NetEvent>,
    visible_requests: std::collections::VecDeque<VisibleModalRequest>,
}

/// One on-the-wire message.  Encoded as a bitcode binary blob inside
/// each WebSocket frame.
#[derive(Clone, Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub enum NetMsg {
    /// Client → server: opening handshake.
    Hello {
        protocol_version: u32,
        nickname: String,
    },
    /// Server → client: handshake response.  Tells the client which
    /// seat it owns and gives it the mission seed it must use to
    /// initialise its sim deterministically.
    Welcome {
        your_seat: PlayerId,
        session_id: MultiplayerSessionId,
        mission_id: String,
        mission_seed: u64,
        sim_config: crate::engine::SimConfig,
        /// Host-selected presentation pack used only to derive stable logical
        /// speech durations. Each peer still plays its own active language.
        speech_timing_locale: Option<String>,
        host_nickname: String,
    },
    /// Client → server: an input the client wants applied this tick,
    /// tagged with the sender's local frame at dispatch time.  The
    /// server uses `origin_frame` as a lower bound when assigning the
    /// shared target frame so a slightly-ahead client does not receive
    /// its own input in the past on localhost.
    Input {
        origin_frame: u32,
        command: PlayerCommand,
    },
    /// Server → all peers: a tagged input ready for engine dispatch
    /// at `target_frame`.
    BroadcastInput {
        /// Server/host sim frame observed when this input was stamped.
        server_frame: u32,
        /// Sender's local sim frame at dispatch time.
        origin_frame: u32,
        target_frame: u32,
        input: PlayerInput,
    },
    /// Either direction, advisory.
    Note(String),
    /// Server → all peers: deterministic engine state hash at the
    /// start of `frame` (pre-tick), broadcast every
    /// [`STATE_HASH_INTERVAL`] frames.
    StateHash {
        frame: u32,
        hash: Option<u64>,
        clock_frame: Option<u32>,
        ms_until_next_frame: Option<u32>,
    },
    /// Server → newly-handshaking peer: an authoritative engine
    /// snapshot for mid-mission joins. `engine_bytes` uses native bitcode.
    InitialSnapshot { frame: u32, engine_bytes: Vec<u8> },
    /// Client → server: this peer has loaded the mission, installed
    /// the host snapshot, and is ready to enter the synchronized sim.
    ReadyToSim { frame: u32 },
    /// Server → all peers: every expected player is loaded and ready;
    /// begin simulating `frame` at this wall-clock timestamp.
    BeginSim { frame: u32, start_epoch_ms: u64 },
    /// Client → server: a visible request for the host to choose this result.
    /// A proposal is never a vote and never changes local modal state.
    ModalProposal {
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        requested_frame: u32,
    },
    /// Server → clients: the sole authoritative result for one exact modal
    /// occurrence. `decision_frame` is the host timeline frame on which the
    /// decision was made and recorded.
    ModalDecision {
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        decision_frame: u32,
    },
    /// Server → peer: abandon the current prediction future and perform a
    /// complete transport handshake. The next session starts from the host's
    /// latest authoritative full snapshot.
    ReconnectRequired { reason: String },
    /// Server → clients: validate and retain these exact serialized save bytes
    /// before acknowledging a host-authored mission transition.
    PrepareSnapshotTransition {
        id: SnapshotTransitionId,
        payload: SnapshotTransitionPayload,
    },
    /// Client → server: the exact transition bytes decoded and validated.
    SnapshotTransitionReady { id: SnapshotTransitionId },
    /// Server → clients: every connected peer retained the same bytes; all
    /// participants may now leave the mission and re-handshake.
    CommitSnapshotTransition { id: SnapshotTransitionId },
}

/// One incoming wire event ready for the game loop.
#[derive(Clone, Debug)]
pub enum NetEvent {
    /// A peer's input arrived, ready to apply at `target_frame`.
    Input {
        server_frame: u32,
        origin_frame: u32,
        target_frame: u32,
        input: PlayerInput,
    },
    /// The server (or our own client connection) has decided we own
    /// this seat in the simulation.
    AssignedLocalSeat(PlayerId),
    /// Best-effort diagnostic from the network layer.
    Note(String),
    /// The connection ended.
    Disconnected,
    /// I/O thread successfully re-handshook with the server after a
    /// drop.  Followed by a fresh `AssignedLocalSeat`.
    Reconnected,
    /// Authoritative state hash and/or clock sample from the host at `frame`.
    PeerStateHash {
        frame: u32,
        hash: Option<u64>,
        clock_frame: Option<u32>,
        ms_until_next_frame: Option<u32>,
    },
    /// Mission construction state announced by the server in `Welcome`.
    /// Only the wasm path emits this; native captures it synchronously.
    MissionConfig {
        mission_id: String,
        rng_seed: u64,
        sim_config: crate::engine::SimConfig,
        speech_timing_locale: Option<String>,
    },
    /// Unrecoverable transport/session compatibility failure.
    Fatal(String),
    /// Authoritative initial-state snapshot from the host.
    InitialSnapshot {
        frame: u32,
        engine_bytes: Vec<u8>,
    },
    /// The server released the multiplayer start barrier.
    BeginSim {
        frame: u32,
        start_epoch_ms: u64,
    },
    /// A client asked the host to choose a modal result. Presentation may show
    /// this request, but only a host decision can close the modal.
    ModalProposal {
        from: PlayerId,
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        requested_frame: u32,
    },
    /// The host chose the result for one exact modal occurrence.
    ModalDecision {
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        decision_frame: u32,
    },
    PrepareSnapshotTransition {
        id: SnapshotTransitionId,
        payload: SnapshotTransitionPayload,
    },
    CommitSnapshotTransition {
        id: SnapshotTransitionId,
    },
}

/// What the game loop pushes into the outgoing channel.
#[derive(Clone, Debug)]
pub enum NetOutbound {
    Input {
        origin_frame: u32,
        command: PlayerCommand,
    },
    StateHash {
        frame: u32,
        hash: Option<u64>,
        clock_frame: Option<u32>,
        ms_until_next_frame: Option<u32>,
    },
    InitialSnapshot {
        frame: u32,
        engine_bytes: Vec<u8>,
    },
    ReadyToSim {
        frame: u32,
    },
    ModalProposal {
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        requested_frame: u32,
    },
    ModalDecision {
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
        decision_frame: u32,
    },
    /// The retained rollback horizon cannot incorporate an input from this
    /// seat. A client drops its whole live QUIC session and re-handshakes; the
    /// host drops the named peer so that peer follows the same reconnect path.
    /// The fresh handshake always carries the host's latest full snapshot.
    ReconnectForSnapshot {
        player_id: PlayerId,
        reason: String,
    },
    /// Host-only escalation: every connected client must discard its local
    /// future and rejoin from the same current authoritative snapshot.
    ReconnectAllForSnapshot {
        reason: String,
    },
    BeginSnapshotTransition {
        id: SnapshotTransitionId,
        payload: SnapshotTransitionPayload,
    },
    SnapshotTransitionReady {
        id: SnapshotTransitionId,
    },
}

/// Channel pair + frame cursor held by the [`crate::engine_manager::EngineManager`].
pub struct NetChannels {
    pub outgoing: Sender<NetOutbound>,
    pub incoming: Receiver<NetEvent>,
    pub deferred_events: Arc<Mutex<std::collections::VecDeque<NetEvent>>>,
    pub frame_cursor: FrameCursor,
    /// Latest authoritative engine snapshot the host wants to share
    /// with newly-handshaking peers.  Set once after mission init via
    /// [`Self::set_initial_snapshot`]; the server's handshake handler
    /// reads it and sends `NetMsg::InitialSnapshot` to each new peer
    /// immediately after `Welcome`.
    pub initial_snapshot: InitialSnapshot,
    modal_sync: Mutex<ModalSyncState>,
    next_transition_sequence: AtomicU64,
}

impl NetChannels {
    /// Build the channels + cursor.  Returns `(NetChannels,
    /// incoming_tx, outgoing_rx, frame_cursor, snapshot_arc)`; the
    /// transport thread keeps the latter four.
    pub fn new() -> (
        Self,
        Sender<NetEvent>,
        Receiver<NetOutbound>,
        FrameCursor,
        InitialSnapshot,
    ) {
        let (out_tx, out_rx) = channel::<NetOutbound>();
        let (in_tx, in_rx) = channel::<NetEvent>();
        let cursor = new_frame_cursor();
        let snapshot = Arc::new(std::sync::Mutex::new(None));
        let deferred_events = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        (
            Self {
                outgoing: out_tx,
                incoming: in_rx,
                deferred_events,
                frame_cursor: Arc::clone(&cursor),
                initial_snapshot: Arc::clone(&snapshot),
                modal_sync: Mutex::new(ModalSyncState::default()),
                next_transition_sequence: AtomicU64::new(0),
            },
            in_tx,
            out_rx,
            cursor,
            snapshot,
        )
    }

    /// Cache an initial-state snapshot the host will offer to every
    /// new peer that handshakes.
    pub fn set_initial_snapshot(&self, frame: u32, engine: &Engine) {
        if let Ok(mut slot) = self.initial_snapshot.lock() {
            *slot = Some((frame, engine.encode_native_snapshot()));
        }
    }

    /// Cache an authoritative host snapshot and push it to peers
    /// that already handshook before the cache was populated.
    pub fn publish_initial_snapshot(&self, frame: u32, engine: &Engine) {
        self.set_initial_snapshot(frame, engine);
        let engine_bytes = engine.encode_native_snapshot();
        let _ = self.outgoing.send(NetOutbound::InitialSnapshot {
            frame,
            engine_bytes,
        });
    }

    /// Announce that this process has loaded the mission, adopted any
    /// required initial snapshot, and is ready for the host-controlled
    /// sim start barrier.
    pub fn send_ready_to_sim(&self, frame: u32) {
        let _ = self.outgoing.send(NetOutbound::ReadyToSim { frame });
    }

    /// Poll a network event, including events deferred by nested UI
    /// loops that only consumed modal-specific messages.
    pub fn try_recv_event(&self) -> Result<NetEvent, std::sync::mpsc::TryRecvError> {
        if let Ok(mut deferred) = self.deferred_events.lock()
            && let Some(event) = deferred.pop_front()
        {
            return Ok(event);
        }
        self.incoming.try_recv()
    }

    /// Poll only the transport receiver.  Modal loops use this to
    /// avoid repeatedly re-reading their own deferred events.
    pub fn try_recv_transport_event(&self) -> Result<NetEvent, std::sync::mpsc::TryRecvError> {
        self.incoming.try_recv()
    }

    /// Push events back in front of the main game-loop drain.
    pub fn defer_events(&self, events: Vec<NetEvent>) {
        if events.is_empty() {
            return;
        }
        if let Ok(mut deferred) = self.deferred_events.lock() {
            for event in events.into_iter().rev() {
                deferred.push_front(event);
            }
        }
    }

    /// Update the frame cursor.  Call once per tick from the game
    /// loop with the engine's `sim_frame`.
    pub fn publish_frame(&self, frame: u32) {
        self.frame_cursor.store(frame, Ordering::Relaxed);
    }

    pub fn current_frame(&self) -> u32 {
        self.frame_cursor.load(Ordering::Relaxed)
    }

    /// Install the host-generated session identity learned during Welcome.
    /// Reinstalling the same identity on reconnect is idempotent; changing it
    /// in-place is a fatal session mismatch.
    pub fn install_session_id(&self, session_id: MultiplayerSessionId) -> Result<(), String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        match sync.session_id {
            Some(current) if current != session_id => Err(format!(
                "multiplayer session identity changed from {current:?} to {session_id:?}"
            )),
            Some(_) => Ok(()),
            None => {
                sync.session_id = Some(session_id);
                Ok(())
            }
        }
    }

    pub fn session_id(&self) -> Result<MultiplayerSessionId, String> {
        self.modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?
            .session_id
            .ok_or_else(|| "multiplayer session identity is not installed".to_string())
    }

    /// Return the stable token for the currently open occurrence of `kind`, or
    /// allocate the next occurrence when this is a newly opened modal.
    pub fn open_modal_instance(&self, kind: &ModalKind) -> Result<ModalInstanceId, String> {
        let opened_frame = self.frame_cursor.load(Ordering::Relaxed);
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        let session_id = sync
            .session_id
            .ok_or_else(|| "multiplayer session identity is not installed".to_string())?;
        if let Some(state) = sync
            .occurrences
            .iter_mut()
            .find(|state| state.kind == *kind)
        {
            if let Some(instance) = state.active {
                return Ok(instance);
            }
            state.next_occurrence = state
                .next_occurrence
                .checked_add(1)
                .ok_or_else(|| "multiplayer modal occurrence counter overflowed".to_string())?;
            let instance = ModalInstanceId {
                session_id,
                opened_frame,
                occurrence: state.next_occurrence,
            };
            state.active = Some(instance);
            return Ok(instance);
        }
        let instance = ModalInstanceId {
            session_id,
            opened_frame,
            occurrence: 1,
        };
        sync.occurrences.push(ModalOccurrenceState {
            kind: kind.clone(),
            next_occurrence: 1,
            active: Some(instance),
        });
        Ok(instance)
    }

    pub fn complete_modal_instance(
        &self,
        kind: &ModalKind,
        instance: ModalInstanceId,
    ) -> Result<(), String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        let state = sync
            .occurrences
            .iter_mut()
            .find(|state| state.kind == *kind)
            .ok_or_else(|| format!("no multiplayer modal occurrence exists for {kind:?}"))?;
        if state.active != Some(instance) {
            return Err(format!(
                "multiplayer modal completion mismatch for {kind:?}: active={:?}, completed={instance:?}",
                state.active
            ));
        }
        state.active = None;
        Ok(())
    }

    /// Route a modal event out of the ordinary simulation drain and into the
    /// presentation-side modal inbox.
    pub fn defer_modal_event(&self, event: NetEvent) -> Result<(), String> {
        if !matches!(
            event,
            NetEvent::ModalProposal { .. } | NetEvent::ModalDecision { .. }
        ) {
            return Err("attempted to route a non-modal event into the modal inbox".to_string());
        }
        self.modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?
            .inbox
            .push_back(event);
        Ok(())
    }

    pub fn try_recv_modal_event(&self) -> Result<NetEvent, std::sync::mpsc::TryRecvError> {
        if let Ok(mut sync) = self.modal_sync.lock()
            && let Some(event) = sync.inbox.pop_front()
        {
            return Ok(event);
        }
        self.try_recv_transport_event()
    }

    pub fn record_visible_modal_request(&self, request: VisibleModalRequest) -> Result<(), String> {
        self.modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?
            .visible_requests
            .push_back(request);
        Ok(())
    }

    pub fn take_visible_modal_requests(
        &self,
        instance: ModalInstanceId,
    ) -> Result<Vec<VisibleModalRequest>, String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        let mut matched = Vec::new();
        let mut retained = std::collections::VecDeque::new();
        while let Some(request) = sync.visible_requests.pop_front() {
            if request.instance == instance {
                matched.push(request);
            } else {
                retained.push_back(request);
            }
        }
        sync.visible_requests = retained;
        Ok(matched)
    }

    pub fn take_all_visible_modal_requests(&self) -> Result<Vec<VisibleModalRequest>, String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        Ok(sync.visible_requests.drain(..).collect())
    }

    /// Push a locally-produced [`PlayerCommand`] onto the wire.
    pub fn send_input(&self, cmd: PlayerCommand) {
        let origin_frame = self.frame_cursor.load(Ordering::Relaxed);
        let _ = self.outgoing.send(NetOutbound::Input {
            origin_frame,
            command: cmd,
        });
    }

    /// Push an authoritative state hash for `frame`.  Server-side only.
    pub fn send_state_hash(
        &self,
        frame: u32,
        hash: u64,
        clock_frame: u32,
        ms_until_next_frame: u32,
    ) {
        let _ = self.outgoing.send(NetOutbound::StateHash {
            frame,
            hash: Some(hash),
            clock_frame: Some(clock_frame),
            ms_until_next_frame: Some(ms_until_next_frame),
        });
    }

    /// Submit a visible client request without changing local modal state.
    /// Channel closure is an authoritative session failure and is propagated.
    pub fn propose_modal_dismiss(
        &self,
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
    ) -> Result<(), String> {
        let requested_frame = self.frame_cursor.load(Ordering::Relaxed);
        self.outgoing
            .send(NetOutbound::ModalProposal {
                instance,
                kind,
                result,
                requested_frame,
            })
            .map_err(|_| "multiplayer modal proposal channel is closed".to_string())
    }

    /// Publish the host's sole authoritative result for an exact modal.
    /// Channel closure is returned so the caller keeps the modal open instead
    /// of applying a local-only result.
    pub fn decide_modal_dismiss(
        &self,
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
    ) -> Result<(), String> {
        let decision_frame = self.frame_cursor.load(Ordering::Relaxed);
        self.outgoing
            .send(NetOutbound::ModalDecision {
                instance,
                kind,
                result,
                decision_frame,
            })
            .map_err(|_| "multiplayer modal decision channel is closed".to_string())
    }

    pub fn reconnect_for_snapshot(
        &self,
        player_id: PlayerId,
        reason: String,
    ) -> Result<(), String> {
        self.outgoing
            .send(NetOutbound::ReconnectForSnapshot { player_id, reason })
            .map_err(|_| "multiplayer snapshot reconnect channel is closed".to_string())
    }

    pub fn reconnect_all_for_snapshot(&self, reason: String) -> Result<(), String> {
        self.outgoing
            .send(NetOutbound::ReconnectAllForSnapshot { reason })
            .map_err(|_| "multiplayer snapshot reconnect channel is closed".to_string())
    }

    /// Begin a host-authoritative save/load transition. The payload is encoded
    /// by the caller exactly once and cloned unchanged to every peer.
    pub fn begin_snapshot_transition(
        &self,
        mission_id: u32,
        save_bytes: Vec<u8>,
    ) -> Result<SnapshotTransitionId, String> {
        let session_id = self.session_id()?;
        let sequence = self
            .next_transition_sequence
            .fetch_add(1, Ordering::Relaxed)
            .checked_add(1)
            .ok_or_else(|| "multiplayer snapshot transition counter overflowed".to_string())?;
        let id = SnapshotTransitionId {
            session_id,
            sequence,
        };
        self.outgoing
            .send(NetOutbound::BeginSnapshotTransition {
                id,
                payload: SnapshotTransitionPayload::Save {
                    mission_id,
                    save_bytes,
                },
            })
            .map_err(|_| "multiplayer snapshot transition channel is closed".to_string())?;
        Ok(id)
    }

    pub fn begin_campaign_exit_transition(
        &self,
        exit_code: crate::game_operation::GameCode,
        engine_bytes: Vec<u8>,
    ) -> Result<SnapshotTransitionId, String> {
        let session_id = self.session_id()?;
        let sequence = self
            .next_transition_sequence
            .fetch_add(1, Ordering::Relaxed)
            .checked_add(1)
            .ok_or_else(|| "multiplayer snapshot transition counter overflowed".to_string())?;
        let id = SnapshotTransitionId {
            session_id,
            sequence,
        };
        self.outgoing
            .send(NetOutbound::BeginSnapshotTransition {
                id,
                payload: SnapshotTransitionPayload::CampaignExit {
                    exit_code,
                    engine_bytes,
                },
            })
            .map_err(|_| "multiplayer campaign transition channel is closed".to_string())?;
        Ok(id)
    }

    pub fn acknowledge_snapshot_transition(&self, id: SnapshotTransitionId) -> Result<(), String> {
        self.outgoing
            .send(NetOutbound::SnapshotTransitionReady { id })
            .map_err(|_| "multiplayer snapshot transition channel is closed".to_string())
    }
}

/// Encode a [`NetMsg`] as a binary WebSocket payload.
pub fn encode_msg(msg: &NetMsg) -> Vec<u8> {
    bitcode::encode(msg)
}

/// Decode a binary WebSocket payload into a [`NetMsg`].
pub fn decode_msg(bytes: &[u8]) -> Result<NetMsg, String> {
    let message: NetMsg = bitcode::decode(bytes).map_err(|error| error.to_string())?;
    if let NetMsg::Welcome { sim_config, .. } = &message {
        sim_config
            .validate()
            .map_err(|error| format!("host sent invalid simulation configuration: {error}"))?;
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_includes_snapshot_transition_barrier() {
        // Version 29 adds the exact-byte prepare/ready/commit transition on
        // top of version 28's canonical speech-timing locale and version 27's
        // resolved difficulty, achievements, and authoritative trading.
        // Older peers fail before decoding incompatible snapshot/input bytes.
        assert_eq!(NET_PROTOCOL_VERSION, 29);
    }

    #[test]
    fn netmsg_roundtrips() {
        let msg = NetMsg::BroadcastInput {
            server_frame: 40,
            origin_frame: 41,
            target_frame: 42,
            input: PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown),
        };
        let bytes = encode_msg(&msg);
        let back = decode_msg(&bytes).expect("decode");
        match back {
            NetMsg::BroadcastInput {
                server_frame,
                origin_frame,
                target_frame,
                input,
            } => {
                assert_eq!(server_frame, 40);
                assert_eq!(origin_frame, 41);
                assert_eq!(target_frame, 42);
                assert_eq!(input.player_id, PlayerId(2));
                assert!(matches!(input.command, PlayerCommand::CrouchDown));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn hello_welcome_roundtrips() {
        let hello = NetMsg::Hello {
            protocol_version: NET_PROTOCOL_VERSION,
            nickname: "alice".into(),
        };
        let welcome = NetMsg::Welcome {
            your_seat: PlayerId(2),
            session_id: MultiplayerSessionId([7; 16]),
            mission_id: "Dem_Lei_MP".into(),
            mission_seed: 42,
            sim_config: crate::engine::SimConfig::default(),
            speech_timing_locale: Some("en-US".into()),
            host_nickname: "host".into(),
        };
        let h = decode_msg(&encode_msg(&hello)).unwrap();
        let w = decode_msg(&encode_msg(&welcome)).unwrap();
        match (h, w) {
            (
                NetMsg::Hello {
                    protocol_version,
                    nickname,
                },
                NetMsg::Welcome {
                    your_seat,
                    session_id,
                    mission_id,
                    mission_seed,
                    sim_config,
                    speech_timing_locale,
                    host_nickname,
                },
            ) => {
                assert_eq!(protocol_version, NET_PROTOCOL_VERSION);
                assert_eq!(nickname, "alice");
                assert_eq!(your_seat, PlayerId(2));
                assert_eq!(session_id, MultiplayerSessionId([7; 16]));
                assert_eq!(mission_id, "Dem_Lei_MP");
                assert_eq!(mission_seed, 42);
                assert_eq!(sim_config, crate::engine::SimConfig::default());
                assert_eq!(speech_timing_locale.as_deref(), Some("en-US"));
                assert_eq!(host_nickname, "host");
            }
            _ => panic!("wrong variants"),
        }
    }

    #[test]
    fn modal_proposal_and_decision_roundtrip_with_exact_identity() {
        let instance = ModalInstanceId {
            session_id: MultiplayerSessionId([9; 16]),
            opened_frame: 120,
            occurrence: 3,
        };
        let kind = ModalKind::Dialog { dialog_id: 44 };
        let proposal = decode_msg(&encode_msg(&NetMsg::ModalProposal {
            instance,
            kind: kind.clone(),
            result: DialogResult::Aborted,
            requested_frame: 123,
        }))
        .expect("decode proposal");
        let decision = decode_msg(&encode_msg(&NetMsg::ModalDecision {
            instance,
            kind: kind.clone(),
            result: DialogResult::Completed,
            decision_frame: 125,
        }))
        .expect("decode decision");

        assert!(matches!(
            proposal,
            NetMsg::ModalProposal {
                instance: decoded,
                kind: ModalKind::Dialog { dialog_id: 44 },
                result: DialogResult::Aborted,
                requested_frame: 123,
            } if decoded == instance
        ));
        assert!(matches!(
            decision,
            NetMsg::ModalDecision {
                instance: decoded,
                kind: ModalKind::Dialog { dialog_id: 44 },
                result: DialogResult::Completed,
                decision_frame: 125,
            } if decoded == instance
        ));
    }

    #[test]
    fn modal_instances_are_stable_until_completed_and_session_bound() {
        let (channels, _incoming, _outgoing, _cursor, _snapshot) = NetChannels::new();
        let session = MultiplayerSessionId([3; 16]);
        channels.install_session_id(session).unwrap();
        channels.publish_frame(17);
        let kind = ModalKind::SherwoodReport;

        let first = channels.open_modal_instance(&kind).unwrap();
        assert_eq!(channels.open_modal_instance(&kind).unwrap(), first);
        channels.complete_modal_instance(&kind, first).unwrap();
        channels.publish_frame(20);
        let second = channels.open_modal_instance(&kind).unwrap();

        assert_eq!(first.session_id, session);
        assert_eq!(first.opened_frame, 17);
        assert_eq!(first.occurrence, 1);
        assert_eq!(second.opened_frame, 20);
        assert_eq!(second.occurrence, 2);
        assert_ne!(first, second);
        assert!(
            channels
                .install_session_id(MultiplayerSessionId([4; 16]))
                .is_err()
        );
    }

    #[test]
    fn snapshot_reconnect_directive_roundtrips() {
        let reconnect = decode_msg(&encode_msg(&NetMsg::ReconnectRequired {
            reason: "rollback horizon".to_string(),
        }))
        .expect("decode reconnect directive");
        assert!(matches!(
            reconnect,
            NetMsg::ReconnectRequired { reason } if reason == "rollback horizon"
        ));
    }

    #[test]
    fn snapshot_transition_roundtrips_exact_bytes() {
        let id = SnapshotTransitionId {
            session_id: MultiplayerSessionId([8; 16]),
            sequence: 3,
        };
        let bytes = vec![0, 1, 2, 3, 254, 255];
        let decoded = decode_msg(&encode_msg(&NetMsg::PrepareSnapshotTransition {
            id,
            payload: SnapshotTransitionPayload::Save {
                mission_id: 42,
                save_bytes: bytes.clone(),
            },
        }))
        .expect("decode snapshot transition");
        assert!(matches!(
            decoded,
            NetMsg::PrepareSnapshotTransition {
                id: decoded_id,
                payload: SnapshotTransitionPayload::Save {
                    mission_id: 42,
                    save_bytes,
                },
            } if decoded_id == id && save_bytes == bytes
        ));
        assert!(matches!(
            decode_msg(&encode_msg(&NetMsg::SnapshotTransitionReady { id })).unwrap(),
            NetMsg::SnapshotTransitionReady { id: decoded_id } if decoded_id == id
        ));
        assert!(matches!(
            decode_msg(&encode_msg(&NetMsg::CommitSnapshotTransition { id })).unwrap(),
            NetMsg::CommitSnapshotTransition { id: decoded_id } if decoded_id == id
        ));
    }

    #[test]
    fn host_transition_api_queues_exact_save_and_campaign_bytes() {
        let (channels, _incoming, outgoing, _cursor, _snapshot) = NetChannels::new();
        let session_id = MultiplayerSessionId([11; 16]);
        channels.install_session_id(session_id).unwrap();

        let save_bytes = vec![9, 8, 7, 6];
        let save_id = channels
            .begin_snapshot_transition(41, save_bytes.clone())
            .unwrap();
        assert_eq!(save_id.sequence, 1);
        assert!(matches!(
            outgoing.recv().unwrap(),
            NetOutbound::BeginSnapshotTransition {
                id,
                payload: SnapshotTransitionPayload::Save {
                    mission_id: 41,
                    save_bytes: actual,
                },
            } if id == save_id && actual == save_bytes
        ));

        let engine_bytes = vec![1, 3, 3, 7];
        let exit_id = channels
            .begin_campaign_exit_transition(
                crate::game_operation::GameCode::LevelInterrupted,
                engine_bytes.clone(),
            )
            .unwrap();
        assert_eq!(exit_id.sequence, 2);
        assert!(matches!(
            outgoing.recv().unwrap(),
            NetOutbound::BeginSnapshotTransition {
                id,
                payload: SnapshotTransitionPayload::CampaignExit {
                    exit_code: crate::game_operation::GameCode::LevelInterrupted,
                    engine_bytes: actual,
                },
            } if id == exit_id && actual == engine_bytes
        ));
    }

    #[test]
    fn quit_updates_roundtrip_the_resolved_difficulty() {
        let msg = NetMsg::Input {
            origin_frame: 73,
            command: PlayerCommand::ApplyQuitMissionUpdates {
                exit_code: crate::game_operation::GameCode::LevelSucceeded,
                difficulty: crate::player_profile::DifficultyLevel::Hard,
                completed_at_unix_seconds: None,
                campaign_run_nonce: Some(1),
            },
        };

        let decoded = decode_msg(&encode_msg(&msg)).expect("decode quit-update command");
        assert!(matches!(
            decoded,
            NetMsg::Input {
                origin_frame: 73,
                command: PlayerCommand::ApplyQuitMissionUpdates {
                    exit_code: crate::game_operation::GameCode::LevelSucceeded,
                    difficulty: crate::player_profile::DifficultyLevel::Hard,
                    completed_at_unix_seconds: None,
                    campaign_run_nonce: Some(1),
                },
            }
        ));
    }

    #[test]
    fn welcome_roundtrips_host_authoritative_custom_difficulty_rules() {
        let mut rules = crate::player_profile::DifficultyRules::MEDIUM;
        rules.enemy_fighting_percent = 175;
        rules.reaction_time_percent = 65;
        rules.legacy_level = crate::player_profile::LegacyDifficultyLevel::Hard;
        let mut sim_config = crate::engine::SimConfig::default();
        sim_config.difficulty = crate::player_profile::DifficultyLevel::custom(rules).unwrap();
        let msg = NetMsg::Welcome {
            your_seat: PlayerId(1),
            session_id: MultiplayerSessionId([19; 16]),
            mission_id: "custom".to_owned(),
            mission_seed: 19,
            sim_config,
            host_nickname: "host".to_owned(),
        };

        let decoded = decode_msg(&encode_msg(&msg)).expect("decode custom Welcome");
        assert!(matches!(
            decoded,
            NetMsg::Welcome {
                sim_config: decoded_config,
                ..
            } if decoded_config == sim_config
        ));
    }

    #[test]
    fn welcome_rejects_invalid_host_difficulty_rules() {
        let mut rules = crate::player_profile::DifficultyRules::MEDIUM;
        rules.enemy_fighting_percent = 0;
        let mut sim_config = crate::engine::SimConfig::default();
        // Construct the malformed wire value directly to verify the network
        // boundary; ordinary callers must use `DifficultyLevel::custom`.
        sim_config.difficulty = crate::player_profile::DifficultyLevel::Custom(rules);
        let message = NetMsg::Welcome {
            your_seat: PlayerId(1),
            session_id: MultiplayerSessionId([1; 16]),
            mission_id: "invalid".to_owned(),
            mission_seed: 1,
            sim_config,
            host_nickname: "host".to_owned(),
        };

        let error = decode_msg(&encode_msg(&message)).unwrap_err();
        assert!(error.contains("enemy_fighting_percent"));
    }

    #[test]
    fn resolved_drop_ale_route_roundtrips_over_bitcode_wire() {
        let route = crate::gate::RecordedGatePath {
            source_sector: crate::sector::SectorNumber::new(133),
            source_sector_index: crate::fast_find_grid::SectorIndex::new(57),
            source_layer: 11,
            outcome: crate::gate::RecordedGateOutcome::Success(vec![crate::gate::GatePathStep {
                door_index: crate::gate::DoorIndex(7),
                direct: false,
            }]),
        };
        let msg = NetMsg::Input {
            origin_frame: 35_283,
            command: PlayerCommand::DropAleAt {
                actor: crate::element::EntityId::Pc(crate::entity_id::PcId(36)),
                target_pos: crate::coordinates::MapPoint::new(778.0, 1714.0),
                running: false,
                already_authorized: true,
                goal_override: Some((crate::sector::SectorNumber::new(0), 0)),
                goal_sector_index_override: crate::fast_find_grid::SectorIndex::new(0),
                recorded_gate_path: Some(route.clone()),
            },
        };

        let decoded = decode_msg(&encode_msg(&msg)).expect("decode resolved DropAle command");
        let NetMsg::Input {
            origin_frame: 35_283,
            command:
                PlayerCommand::DropAleAt {
                    actor,
                    target_pos,
                    running: false,
                    already_authorized: true,
                    goal_override: Some((goal_sector, 0)),
                    goal_sector_index_override,
                    recorded_gate_path: Some(decoded_route),
                },
        } = decoded
        else {
            panic!("resolved DropAle command must survive bitcode wire round-trip");
        };
        assert_eq!(
            actor,
            crate::element::EntityId::Pc(crate::entity_id::PcId(36))
        );
        assert_eq!(target_pos.x.to_bits(), 778.0_f32.to_bits());
        assert_eq!(target_pos.y.to_bits(), 1714.0_f32.to_bits());
        assert_eq!(goal_sector, crate::sector::SectorNumber::new(0));
        assert_eq!(
            goal_sector_index_override,
            crate::fast_find_grid::SectorIndex::new(0)
        );
        assert_eq!(decoded_route, route);
    }
}
