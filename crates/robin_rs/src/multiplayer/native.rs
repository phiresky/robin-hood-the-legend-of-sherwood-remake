//! Native (non-wasm) WebSocket server / client for the multiplayer
//! transport.  Each external function spawns one or more OS threads
//! that own the socket I/O; the game loop talks to them through
//! [`super::NetChannels`].

use super::{
    FrameCursor, INPUT_DELAY_FRAMES, InitialSnapshot, NET_PROTOCOL_VERSION, NetEvent, NetMsg,
    NetOutbound, decode_msg, encode_msg,
};
use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tungstenite::Message as WsMessage;
use tungstenite::accept as ws_accept;
use tungstenite::client::IntoClientRequest;

type AsyncWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ─── Server ──────────────────────────────────────────────────────

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Handle to a running multiplayer server.
///
/// Shutdown is deterministic: cancellation stops the accept and outgoing
/// loops, active peer queues are closed, and every spawned worker is joined.
pub struct ServerHandle {
    /// `(local_seat, mission_seed)` the server is operating with.
    pub local_seat: PlayerId,
    pub mission_seed: u64,
    local_addr: SocketAddr,
    cancellation: Arc<AtomicBool>,
    peers: Arc<Mutex<ServerPeers>>,
    accept_thread: Option<JoinHandle<()>>,
    outgoing_thread: Option<JoinHandle<()>>,
    peer_threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
    active_connections: Arc<Mutex<HashMap<u64, TcpStream>>>,
}

impl ServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn shutdown(&mut self) {
        self.cancellation.store(true, Ordering::Release);

        if let Some(handle) = self.accept_thread.take()
            && handle.join().is_err()
        {
            tracing::error!("multiplayer accept worker panicked during shutdown");
        }

        // Closing peer queues wakes writers; shutting down the sockets wakes
        // readers and partial WebSocket handshakes immediately.
        self.peers.lock().unwrap().senders.clear();
        for stream in self.active_connections.lock().unwrap().values() {
            let _ = stream.shutdown(Shutdown::Both);
        }

        if let Some(handle) = self.outgoing_thread.take()
            && handle.join().is_err()
        {
            tracing::error!("multiplayer outgoing worker panicked during shutdown");
        }

        let peer_threads = std::mem::take(&mut *self.peer_threads.lock().unwrap());
        for handle in peer_threads {
            if handle.join().is_err() {
                tracing::error!("multiplayer peer worker panicked during shutdown");
            }
        }
        self.active_connections.lock().unwrap().clear();
    }

    #[cfg(test)]
    pub(crate) fn peer_worker_count(&self) -> usize {
        self.peer_threads.lock().unwrap().len()
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Per-peer state tracked by the server.  Wrapped in an `Arc<Mutex<>>`
/// so the accept thread, the broadcast loop, and each per-peer
/// receive thread can share access.
struct ServerPeers {
    /// Next [`PlayerId`] to assign for a peer with a nickname the
    /// server has not seen before.  Starts at 1 — seat 0 is the host.
    next_seat: u8,
    /// Active peers, keyed by their assigned [`PlayerId`].  The value
    /// is the sender used to push outbound frames into that peer's
    /// per-connection writer thread.
    senders: HashMap<u8, Sender<NetMsg>>,
    /// Nicknames per active seat (for `SeatJoined` rebroadcasts and
    /// so peers see each other's labels).  Mirrors what the host
    /// folds into [`PlayerCommand::ConnectSeat`].
    nicknames: HashMap<u8, String>,
    /// Seats that previously hosted a peer who has since
    /// disconnected, keyed by nickname.  When a fresh `Hello` arrives
    /// with a nickname that matches a disconnected slot, the server
    /// reassigns the old seat instead of allocating a new one.  Lets
    /// the rejoining peer take back ownership of their PCs (the sim's
    /// drop-in/drop-out preserved their selection / hotgroups across
    /// the disconnect).
    disconnected_seats: HashMap<String, u8>,
    expected_players: u32,
    host_ready_frame: Option<u32>,
    ready_seats: HashMap<u8, u32>,
    begin_sent: Option<(u32, u64)>,
}

impl ServerPeers {
    fn new(expected_players: u32) -> Self {
        Self {
            next_seat: 1,
            senders: HashMap::new(),
            nicknames: HashMap::new(),
            disconnected_seats: HashMap::new(),
            expected_players,
            host_ready_frame: None,
            ready_seats: HashMap::new(),
            begin_sent: None,
        }
    }
}

/// Joins a per-peer writer even when the handshake handler returns early.
struct PeerWriterGuard {
    seat: u8,
    peers: Arc<Mutex<ServerPeers>>,
    handle: Option<JoinHandle<()>>,
}

impl PeerWriterGuard {
    fn join(mut self) {
        self.close_and_join();
    }

    fn close_and_join(&mut self) {
        {
            let mut peers = self.peers.lock().unwrap();
            peers.senders.remove(&self.seat);
            peers.ready_seats.remove(&self.seat);
            peers.nicknames.remove(&self.seat);
        }
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            tracing::error!(seat = self.seat, "peer writer worker panicked");
        }
    }
}

impl Drop for PeerWriterGuard {
    fn drop(&mut self) {
        self.close_and_join();
    }
}

struct ActiveConnectionGuard {
    id: u64,
    connections: Arc<Mutex<HashMap<u64, TcpStream>>>,
}

impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        self.connections.lock().unwrap().remove(&self.id);
    }
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn maybe_begin_sim_locked(peers: &mut ServerPeers) -> Option<(u32, u64, Vec<Sender<NetMsg>>)> {
    if peers.begin_sent.is_some() {
        return None;
    }
    let host_frame = peers.host_ready_frame?;
    let active_peer_count = peers.senders.len() as u32;
    let expected_peer_count = peers.expected_players.saturating_sub(1);
    if active_peer_count < expected_peer_count {
        return None;
    }
    if !peers
        .senders
        .keys()
        .all(|seat| peers.ready_seats.contains_key(seat))
    {
        return None;
    }

    let begin_frame = peers
        .ready_seats
        .values()
        .copied()
        .fold(host_frame, u32::max);
    let start_epoch_ms = current_epoch_ms().saturating_add(500);
    let senders = peers.senders.values().cloned().collect();
    peers.begin_sent = Some((begin_frame, start_epoch_ms));
    Some((begin_frame, start_epoch_ms, senders))
}

/// Start a multiplayer server on `addr`.  The server runs the host
/// seat (seat 0) locally — the returned [`NetEvent`] stream will
/// receive each peer's inputs and seat-join/leave events.  The local
/// process should also push its own [`PlayerCommand`]s into
/// `outgoing_rx` via the sibling sender so they are broadcast to
/// peers and folded into the local input batch.
#[allow(clippy::too_many_arguments)]
pub fn start_server(
    addr: &str,
    host_nickname: String,
    mission_seed: u64,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
    frame_cursor: FrameCursor,
    initial_snapshot: InitialSnapshot,
    expected_players: u32,
) -> std::io::Result<ServerHandle> {
    let listener = TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    let local_addr = listener.local_addr()?;
    tracing::info!(
        addr = %local_addr,
        seed = mission_seed,
        "multiplayer server listening"
    );

    let peers = Arc::new(Mutex::new(ServerPeers::new(expected_players.max(1))));
    let cancellation = Arc::new(AtomicBool::new(false));
    let peer_threads = Arc::new(Mutex::new(Vec::new()));
    let active_connections = Arc::new(Mutex::new(HashMap::new()));

    // Spawn a thread that takes locally-produced PlayerCommands,
    // stamps them with seat 0 + a target frame, fans them out to every
    // peer's writer queue, AND echoes them back into incoming_tx so
    // the local game loop applies them in the same input order every
    // other machine does.  Target frame = current sim frame +
    // INPUT_DELAY_FRAMES so peers (which receive the broadcast over
    // the wire with some latency) still have time to apply at the
    // matching frame; if a peer is already past the target, the
    // rollback path picks up the slack.
    let outgoing_thread = {
        let peers = Arc::clone(&peers);
        let incoming_tx = incoming_tx.clone();
        let cursor = Arc::clone(&frame_cursor);
        let cancellation = Arc::clone(&cancellation);
        thread::Builder::new()
            .name("mp-server-outgoing".into())
            .spawn(move || {
                while !cancellation.load(Ordering::Acquire) {
                    let msg = match outgoing_rx.recv_timeout(WORKER_POLL_INTERVAL) {
                        Ok(msg) => msg,
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                    match msg {
                        NetOutbound::Input {
                            origin_frame,
                            command,
                        } => {
                            let now = cursor.load(Ordering::Relaxed);
                            let target = now.max(origin_frame).saturating_add(INPUT_DELAY_FRAMES);
                            let inp = PlayerInput::new(PlayerId::HOST, command);
                            broadcast_input(&peers, &incoming_tx, now, origin_frame, target, inp);
                        }
                        NetOutbound::StateHash {
                            frame,
                            hash,
                            clock_frame,
                            ms_until_next_frame,
                        } => {
                            // Authoritative-host state hash: broadcast as
                            // a wire `StateHash` to every peer.  No echo
                            // into our own incoming channel — the local
                            // game loop already has the value (it just
                            // computed the hash before pushing here).
                            let to_send: Vec<Sender<NetMsg>> = {
                                let p = peers.lock().unwrap();
                                p.senders.values().cloned().collect()
                            };
                            for sender in to_send {
                                let _ = sender.send(NetMsg::StateHash {
                                    frame,
                                    hash,
                                    clock_frame,
                                    ms_until_next_frame,
                                });
                            }
                        }
                        NetOutbound::InitialSnapshot {
                            frame,
                            engine_bytes,
                        } => {
                            // A peer can complete the WebSocket handshake
                            // before mission setup has produced the
                            // frame-0 snapshot.  Push the snapshot to all
                            // currently-connected peers as soon as it
                            // exists; later peers still receive it through
                            // the handshake cache.
                            let to_send: Vec<Sender<NetMsg>> = {
                                let p = peers.lock().unwrap();
                                p.senders.values().cloned().collect()
                            };
                            for sender in to_send {
                                let _ = sender.send(NetMsg::InitialSnapshot {
                                    frame,
                                    engine_bytes: engine_bytes.clone(),
                                });
                            }
                        }
                        NetOutbound::ReadyToSim { frame } => {
                            let begin = {
                                let mut p = peers.lock().unwrap();
                                p.host_ready_frame = Some(frame);
                                maybe_begin_sim_locked(&mut p)
                            };
                            if let Some((begin_frame, start_epoch_ms, senders)) = begin {
                                tracing::info!(
                                    frame = begin_frame,
                                    start_epoch_ms,
                                    "multiplayer: ready barrier complete"
                                );
                                let _ = incoming_tx.send(NetEvent::BeginSim {
                                    frame: begin_frame,
                                    start_epoch_ms,
                                });
                                for sender in senders {
                                    let _ = sender.send(NetMsg::BeginSim {
                                        frame: begin_frame,
                                        start_epoch_ms,
                                    });
                                }
                            }
                        }
                        NetOutbound::ModalDismiss { kind, result } => {
                            let _ = incoming_tx.send(NetEvent::ModalDismiss {
                                kind: kind.clone(),
                                result,
                            });
                            let to_send: Vec<Sender<NetMsg>> = {
                                let p = peers.lock().unwrap();
                                p.senders.values().cloned().collect()
                            };
                            for sender in to_send {
                                let _ = sender.send(NetMsg::ModalDismiss {
                                    kind: kind.clone(),
                                    result,
                                });
                            }
                        }
                    }
                }
                tracing::info!("server outgoing-pump thread stopped");
            })?
    };

    // Accept loop — for each new connection, spawn handler threads.
    let peers_for_accept = Arc::clone(&peers);
    let host_nick_for_accept = host_nickname;
    let cursor_for_accept = Arc::clone(&frame_cursor);
    let snapshot_for_accept = std::sync::Arc::clone(&initial_snapshot);
    let cancellation_for_accept = Arc::clone(&cancellation);
    let peer_threads_for_accept = Arc::clone(&peer_threads);
    let active_connections_for_accept = Arc::clone(&active_connections);
    let accept_thread = match thread::Builder::new()
        .name("mp-accept".into())
        .spawn(move || {
            let mut next_connection_id = 0_u64;
            while !cancellation_for_accept.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let connection_id = next_connection_id;
                        let Some(next_id) = next_connection_id.checked_add(1) else {
                            tracing::error!("multiplayer connection id overflow");
                            break;
                        };
                        next_connection_id = next_id;
                        let shutdown_stream = match stream.try_clone() {
                            Ok(stream) => stream,
                            Err(e) => {
                                tracing::warn!("failed to track accepted peer socket: {e}");
                                continue;
                            }
                        };
                        active_connections_for_accept
                            .lock()
                            .unwrap()
                            .insert(connection_id, shutdown_stream);
                        let peers = Arc::clone(&peers_for_accept);
                        let incoming_tx = incoming_tx.clone();
                        let host_nick = host_nick_for_accept.clone();
                        let cursor = Arc::clone(&cursor_for_accept);
                        let snapshot = std::sync::Arc::clone(&snapshot_for_accept);
                        let cancellation = Arc::clone(&cancellation_for_accept);
                        let active_connections = Arc::clone(&active_connections_for_accept);
                        let handle =
                            thread::Builder::new()
                                .name("mp-handshake".into())
                                .spawn(move || {
                                    let _connection_guard = ActiveConnectionGuard {
                                        id: connection_id,
                                        connections: active_connections,
                                    };
                                    if let Err(e) = handle_incoming_peer(
                                        stream,
                                        peers,
                                        incoming_tx,
                                        host_nick,
                                        mission_seed,
                                        cursor,
                                        snapshot,
                                        Arc::clone(&cancellation),
                                    ) && !cancellation.load(Ordering::Acquire)
                                    {
                                        tracing::warn!("incoming peer handler ended: {e}");
                                    }
                                });
                        match handle {
                            Ok(handle) => peer_threads_for_accept.lock().unwrap().push(handle),
                            Err(e) => {
                                active_connections_for_accept
                                    .lock()
                                    .unwrap()
                                    .remove(&connection_id);
                                tracing::error!("failed to spawn peer worker: {e}");
                            }
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(WORKER_POLL_INTERVAL);
                    }
                    Err(_e) if cancellation_for_accept.load(Ordering::Acquire) => break,
                    Err(e) => {
                        tracing::warn!("accept error: {e}");
                        break;
                    }
                }
            }
        }) {
        Ok(handle) => handle,
        Err(e) => {
            cancellation.store(true, Ordering::Release);
            let _ = outgoing_thread.join();
            return Err(e);
        }
    };

    Ok(ServerHandle {
        local_seat: PlayerId::HOST,
        mission_seed,
        local_addr,
        cancellation,
        peers,
        accept_thread: Some(accept_thread),
        outgoing_thread: Some(outgoing_thread),
        peer_threads,
        active_connections,
    })
}

/// Send a [`NetMsg::BroadcastInput`] to every peer plus echo it into
/// the local game-loop event stream.  Drops senders that error out
/// (signals a disconnected peer; the per-peer reader thread emits
/// `SeatLeft` on the way out).
fn broadcast_input(
    peers: &Arc<Mutex<ServerPeers>>,
    incoming_tx: &Sender<NetEvent>,
    server_frame: u32,
    origin_frame: u32,
    target_frame: u32,
    inp: PlayerInput,
) {
    // Local fan-in: feed the input back into our own game loop.
    let _ = incoming_tx.send(NetEvent::Input {
        server_frame,
        origin_frame,
        target_frame,
        input: inp.clone(),
    });

    // Send to every peer.  Hold the lock briefly to clone the
    // sender list; the actual sends happen unlocked.
    let to_send: Vec<(u8, Sender<NetMsg>)> = {
        let p = peers.lock().unwrap();
        p.senders.iter().map(|(k, v)| (*k, v.clone())).collect()
    };
    for (seat, sender) in to_send {
        if sender
            .send(NetMsg::BroadcastInput {
                server_frame,
                origin_frame,
                target_frame,
                input: inp.clone(),
            })
            .is_err()
        {
            tracing::warn!(seat, "broadcast send to peer failed");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_incoming_peer(
    stream: TcpStream,
    peers: Arc<Mutex<ServerPeers>>,
    incoming_tx: Sender<NetEvent>,
    host_nickname: String,
    mission_seed: u64,
    frame_cursor: FrameCursor,
    initial_snapshot: InitialSnapshot,
    cancellation: Arc<AtomicBool>,
) -> Result<(), String> {
    if let Err(e) = stream.set_nodelay(true) {
        tracing::warn!("failed to set TCP_NODELAY on peer stream: {e}");
    }
    let peer_addr = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    tracing::info!(peer = %peer_addr, "incoming connection");

    // Dup the TCP stream BEFORE the WebSocket handshake so the writer
    // thread can wrap its own half once the handshake completes.
    // Both halves see the same TCP connection (full-duplex), and we
    // discipline ourselves: the reader thread only ever reads; the
    // writer thread only ever writes.  WebSocket framing is
    // direction-independent so per-half WebSocket instances don't
    // confuse each other.
    let writer_stream = stream
        .try_clone()
        .map_err(|e| format!("dup peer stream: {e}"))?;
    if let Err(e) = writer_stream.set_nodelay(true) {
        tracing::warn!("failed to set TCP_NODELAY on peer writer stream: {e}");
    }

    let mut ws = ws_accept(stream).map_err(|e| format!("ws upgrade: {e}"))?;

    // Receive Hello.  Reject anything else.
    let hello_frame = ws.read().map_err(|e| format!("read Hello: {e}"))?;
    let hello_bytes = match hello_frame {
        WsMessage::Binary(b) => b,
        other => return Err(format!("expected binary Hello, got {other:?}")),
    };
    let nickname = match decode_msg(&hello_bytes).map_err(|e| format!("decode Hello: {e}"))? {
        NetMsg::Hello {
            protocol_version,
            nickname,
        } => {
            if protocol_version != NET_PROTOCOL_VERSION {
                return Err(format!(
                    "protocol mismatch (peer={protocol_version}, server={NET_PROTOCOL_VERSION})"
                ));
            }
            nickname
        }
        other => return Err(format!("expected Hello, got {other:?}")),
    };

    // Assign a seat — reuse the previously-held one if this nickname
    // is a returning peer.  Otherwise allocate the next fresh seat.
    let (assigned_seat_u8, write_rx, is_rejoin) = {
        let mut p = peers.lock().unwrap();
        let (seat, rejoin) = if let Some(prior) = p.disconnected_seats.remove(&nickname) {
            tracing::info!(
                nickname = %nickname,
                seat = prior,
                "peer rejoining: reassigning prior seat"
            );
            (prior, true)
        } else {
            let next = p.next_seat;
            p.next_seat = next.checked_add(1).ok_or("seat overflow")?;
            (next, false)
        };
        let (write_tx, write_rx) = channel::<NetMsg>();
        p.senders.insert(seat, write_tx);
        p.nicknames.insert(seat, nickname.clone());
        (seat, write_rx, rejoin)
    };
    let assigned_seat = PlayerId(assigned_seat_u8);
    let _ = is_rejoin;

    // Spawn the writer thread, owned by this peer.  It drains
    // `write_rx` (frames the broadcast loop / handshake pushes) and
    // writes them onto the duplicated TCP half.
    let writer_cancellation = Arc::clone(&cancellation);
    let writer_handle = thread::Builder::new()
        .name(format!("mp-peer-{assigned_seat_u8}-tx"))
        .spawn(move || {
            let mut writer_ws = tungstenite::WebSocket::from_raw_socket(
                writer_stream,
                tungstenite::protocol::Role::Server,
                None,
            );
            while !writer_cancellation.load(Ordering::Acquire) {
                let msg = match write_rx.recv_timeout(WORKER_POLL_INTERVAL) {
                    Ok(msg) => msg,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                let bytes = encode_msg(&msg);
                if let Err(e) = writer_ws.send(WsMessage::Binary(bytes.into())) {
                    tracing::warn!(seat = assigned_seat_u8, "writer send failed: {e}");
                    break;
                }
                if let Err(e) = writer_ws.flush() {
                    tracing::warn!(seat = assigned_seat_u8, "writer flush failed: {e}");
                    break;
                }
            }
            tracing::info!(seat = assigned_seat_u8, "peer writer thread stopped");
        })
        .map_err(|e| format!("spawn peer writer: {e}"))?;
    let writer = PeerWriterGuard {
        seat: assigned_seat_u8,
        peers: Arc::clone(&peers),
        handle: Some(writer_handle),
    };

    // Send Welcome to this peer.  Goes through the writer queue so
    // the writer thread is the only thing that touches the outbound
    // half of the socket.  If the host has cached an initial-state
    // snapshot we follow up with that — mid-mission joiners adopt
    // it instead of trying to reproduce engine init from seed alone.
    {
        let p = peers.lock().unwrap();
        if let Some(sender) = p.senders.get(&assigned_seat_u8) {
            sender
                .send(NetMsg::Welcome {
                    your_seat: assigned_seat,
                    mission_seed,
                    host_nickname: host_nickname.clone(),
                })
                .map_err(|_| "writer queue closed before Welcome")?;
            let snapshot_frame = if let Some((frame, engine)) =
                initial_snapshot.lock().ok().and_then(|g| g.clone())
            {
                let encode_start = web_time::Instant::now();
                match bincode::serde::encode_to_vec(&engine, bincode::config::standard()) {
                    Ok(bytes) => {
                        tracing::info!(
                            seat = assigned_seat_u8,
                            frame,
                            bytes = bytes.len(),
                            encode_us = encode_start.elapsed().as_micros(),
                            "sending initial snapshot to peer"
                        );
                        let _ = sender.send(NetMsg::InitialSnapshot {
                            frame,
                            engine_bytes: bytes,
                        });
                        Some(frame)
                    }
                    Err(e) => {
                        tracing::warn!(
                            seat = assigned_seat_u8,
                            frame,
                            "failed to serialize initial snapshot for peer: {e}"
                        );
                        None
                    }
                }
            } else {
                None
            };
            if let Some((frame, start_epoch_ms)) = p.begin_sent {
                let begin_frame =
                    snapshot_frame.map_or(frame, |snapshot_frame| snapshot_frame.max(frame));
                let begin_start_epoch_ms = if begin_frame != frame {
                    current_epoch_ms().saturating_add(100)
                } else {
                    start_epoch_ms
                };
                let _ = sender.send(NetMsg::BeginSim {
                    frame: begin_frame,
                    start_epoch_ms: begin_start_epoch_ms,
                });
            }
        }
    }

    // Broadcast ConnectSeat as a regular tagged BroadcastInput.
    // Routing through `broadcast_input` stamps `target_frame` from the
    // shared cursor (so the local echo and every peer apply the
    // ConnectSeat at the same simulation frame), keeping the seat's
    // arrival deterministic across machines.  Receivers fold it
    // into the engine's `seats` vec just like any other input.
    {
        let now = frame_cursor.load(Ordering::Relaxed);
        let target = now.saturating_add(INPUT_DELAY_FRAMES);
        let inp = PlayerInput::new(
            PlayerId::HOST,
            PlayerCommand::ConnectSeat {
                player_id: assigned_seat,
                nickname: nickname.clone(),
            },
        );
        broadcast_input(&peers, &incoming_tx, now, now, target, inp);
    }

    // Reader loop on the original WebSocket half.  Every Input
    // received gets stamped with the peer's assigned seat (defensive
    // — the client tags its own outgoing too, but we don't trust the
    // wire) and a target frame derived from the server's current sim
    // frame at receive time, before broadcasting.
    let result = run_server_peer_reader(
        &mut ws,
        assigned_seat,
        Arc::clone(&peers),
        incoming_tx.clone(),
        Arc::clone(&frame_cursor),
        Arc::clone(&cancellation),
    );

    // On disconnect: drop the peer slot, broadcast SeatLeft.  The
    // writer thread will exit once we drop its sender.  The nickname
    // is parked in `disconnected_seats` so a future `Hello` from the
    // same nickname is reassigned the same seat — the sim preserves
    // the seat's selection / hotgroups across the disconnect, so the
    // rejoining peer takes back ownership of the PCs they were
    // controlling.
    {
        let mut p = peers.lock().unwrap();
        p.senders.remove(&assigned_seat_u8);
        p.ready_seats.remove(&assigned_seat_u8);
        if let Some(nick) = p.nicknames.remove(&assigned_seat_u8) {
            p.disconnected_seats.insert(nick, assigned_seat_u8);
        }
    }
    if !cancellation.load(Ordering::Acquire) {
        let now = frame_cursor.load(Ordering::Relaxed);
        let target = now.saturating_add(INPUT_DELAY_FRAMES);
        let inp = PlayerInput::new(
            PlayerId::HOST,
            PlayerCommand::DisconnectSeat {
                player_id: assigned_seat,
            },
        );
        broadcast_input(&peers, &incoming_tx, now, now, target, inp);
    }

    writer.join();

    result
}

fn run_server_peer_reader(
    ws: &mut tungstenite::WebSocket<TcpStream>,
    seat: PlayerId,
    peers: Arc<Mutex<ServerPeers>>,
    incoming_tx: Sender<NetEvent>,
    frame_cursor: FrameCursor,
    cancellation: Arc<AtomicBool>,
) -> Result<(), String> {
    while !cancellation.load(Ordering::Acquire) {
        match ws.read() {
            Ok(WsMessage::Binary(b)) => match decode_msg(&b) {
                Ok(NetMsg::Input {
                    origin_frame,
                    command,
                }) => {
                    let now = frame_cursor.load(Ordering::Relaxed);
                    let target = now.max(origin_frame).saturating_add(INPUT_DELAY_FRAMES);
                    let inp = PlayerInput::new(seat, command);
                    broadcast_input(&peers, &incoming_tx, now, origin_frame, target, inp);
                }
                Ok(NetMsg::Note(s)) => {
                    tracing::info!(?seat, note = %s, "peer note");
                }
                Ok(NetMsg::ModalDismiss { kind, result }) => {
                    let _ = incoming_tx.send(NetEvent::ModalDismiss {
                        kind: kind.clone(),
                        result,
                    });
                    let to_send: Vec<Sender<NetMsg>> = {
                        let p = peers.lock().unwrap();
                        p.senders.values().cloned().collect()
                    };
                    for sender in to_send {
                        let _ = sender.send(NetMsg::ModalDismiss {
                            kind: kind.clone(),
                            result,
                        });
                    }
                }
                Ok(NetMsg::ReadyToSim { frame }) => {
                    let begin = {
                        let mut p = peers.lock().unwrap();
                        p.ready_seats.insert(seat.0, frame);
                        maybe_begin_sim_locked(&mut p)
                    };
                    if let Some((begin_frame, start_epoch_ms, senders)) = begin {
                        tracing::info!(
                            frame = begin_frame,
                            start_epoch_ms,
                            "multiplayer: ready barrier complete"
                        );
                        let _ = incoming_tx.send(NetEvent::BeginSim {
                            frame: begin_frame,
                            start_epoch_ms,
                        });
                        for sender in senders {
                            let _ = sender.send(NetMsg::BeginSim {
                                frame: begin_frame,
                                start_epoch_ms,
                            });
                        }
                    }
                }
                Ok(other) => {
                    tracing::debug!(?seat, ?other, "ignoring inbound message from peer");
                }
                Err(e) => {
                    tracing::warn!(?seat, "decode failure from peer: {e}");
                }
            },
            Ok(WsMessage::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    Ok(())
}

// ─── Client ──────────────────────────────────────────────────────

/// Handle to an active client connection.
pub struct ClientHandle {
    /// Seat assigned by the server.  `None` until the handshake
    /// completes.  Game loop reads this to set `host.local_seat`.
    pub assigned_seat: Arc<Mutex<Option<PlayerId>>>,
    /// Mission RNG seed announced by the server in `Welcome`.  The
    /// client adopts this seed for its engine init so the local sim
    /// rolls match the host's.
    pub mission_seed: Option<u64>,
    cancellation: Arc<AtomicBool>,
    io_thread: Option<JoinHandle<()>>,
}

impl ClientHandle {
    pub fn mission_seed(&self) -> Option<u64> {
        self.mission_seed
    }

    pub fn shutdown(&mut self) {
        self.cancellation.store(true, Ordering::Release);
        if let Some(handle) = self.io_thread.take()
            && handle.join().is_err()
        {
            tracing::error!("multiplayer client worker panicked during shutdown");
        }
    }
}

impl Drop for ClientHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Connect to a multiplayer server and run the I/O thread.  Returns
/// once the WebSocket handshake completes; the assigned seat is
/// reported through `incoming_tx` as a [`NetEvent::AssignedLocalSeat`].
pub fn connect_client<A: ToSocketAddrs + std::fmt::Display>(
    addr: A,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    let addr_str = addr.to_string();
    let assigned_seat = Arc::new(Mutex::new(None));
    let assigned_clone = Arc::clone(&assigned_seat);
    let cancellation = Arc::new(AtomicBool::new(false));
    let cancellation_for_thread = Arc::clone(&cancellation);
    let (handshake_tx, handshake_rx) = std::sync::mpsc::sync_channel(1);
    let io_thread = thread::Builder::new()
        .name("mp-client".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = handshake_tx.send(Err(format!("build tokio runtime: {e}")));
                    return;
                }
            };
            rt.block_on(async move {
                run_client_io_async(
                    addr_str,
                    nickname,
                    incoming_tx,
                    outgoing_rx,
                    assigned_clone,
                    handshake_tx,
                    Arc::clone(&cancellation_for_thread),
                )
                .await;
            });
        })?;

    let (your_seat, mission_seed) = match handshake_rx.recv() {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => {
            cancellation.store(true, Ordering::Release);
            let _ = io_thread.join();
            return Err(std::io::Error::other(format!("initial handshake: {err}")));
        }
        Err(e) => {
            cancellation.store(true, Ordering::Release);
            let _ = io_thread.join();
            return Err(std::io::Error::other(format!(
                "initial handshake channel closed: {e}"
            )));
        }
    };
    tracing::info!(%addr, ?your_seat, seed = mission_seed, "multiplayer client connected");

    Ok(ClientHandle {
        assigned_seat,
        mission_seed: Some(mission_seed),
        cancellation,
        io_thread: Some(io_thread),
    })
}

/// One round of (TCP connect → WebSocket upgrade → Hello → Welcome).
/// Returns the live async `WebSocket` plus the assigned seat and
/// mission seed.  Used both for the initial handshake and for the
/// auto-retry path after disconnects.
async fn handshake_async(addr: &str, nickname: &str) -> Result<(AsyncWs, PlayerId, u64), String> {
    let url = format!("ws://{addr}/");
    let request = url
        .into_client_request()
        .map_err(|e| format!("bad url: {e}"))?;
    let (mut ws, _resp) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    let hello = encode_msg(&NetMsg::Hello {
        protocol_version: NET_PROTOCOL_VERSION,
        nickname: nickname.to_string(),
    });
    ws.send(WsMessage::Binary(hello.into()))
        .await
        .map_err(|e| format!("send Hello: {e}"))?;

    let welcome_frame = ws
        .next()
        .await
        .ok_or_else(|| "connection closed before Welcome".to_string())?
        .map_err(|e| format!("read Welcome: {e}"))?;
    let welcome_bytes = match welcome_frame {
        WsMessage::Binary(b) => b,
        other => return Err(format!("expected binary Welcome, got {other:?}")),
    };
    match decode_msg(&welcome_bytes).map_err(|e| format!("decode Welcome: {e}"))? {
        NetMsg::Welcome {
            your_seat,
            mission_seed,
            host_nickname,
        } => {
            tracing::info!(
                ?your_seat,
                seed = mission_seed,
                host = %host_nickname,
                "welcomed by server"
            );
            Ok((ws, your_seat, mission_seed))
        }
        other => Err(format!("expected Welcome, got {other:?}")),
    }
}

async fn handshake_or_cancel(
    addr: &str,
    nickname: &str,
    cancellation: &AtomicBool,
) -> Option<Result<(AsyncWs, PlayerId, u64), String>> {
    tokio::select! {
        result = handshake_async(addr, nickname) => Some(result),
        _ = wait_for_cancel(cancellation) => None,
    }
}

/// Drive one connection until it ends, then auto-reconnect with
/// exponential backoff.  Returns when the channel side of the
/// outgoing queue closes (the game loop dropping `host.net`).
async fn run_client_io_async(
    addr: String,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
    assigned: Arc<Mutex<Option<PlayerId>>>,
    initial_handshake_tx: std::sync::mpsc::SyncSender<Result<(PlayerId, u64), String>>,
    cancellation: Arc<AtomicBool>,
) {
    let (outgoing_async_tx, mut outgoing_async_rx) =
        tokio::sync::mpsc::unbounded_channel::<NetOutbound>();
    let bridge_cancellation = Arc::clone(&cancellation);
    let outgoing_bridge = match thread::Builder::new()
        .name("mp-client-outgoing-bridge".into())
        .spawn(move || {
            while !bridge_cancellation.load(Ordering::Acquire) {
                let msg = match outgoing_rx.recv_timeout(WORKER_POLL_INTERVAL) {
                    Ok(msg) => msg,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                if outgoing_async_tx.send(msg).is_err() {
                    break;
                }
            }
        }) {
        Ok(handle) => handle,
        Err(e) => {
            let _ =
                initial_handshake_tx.send(Err(format!("spawn multiplayer outgoing bridge: {e}")));
            return;
        }
    };

    run_client_io_inner(
        addr,
        nickname,
        incoming_tx,
        &mut outgoing_async_rx,
        assigned,
        initial_handshake_tx,
        Arc::clone(&cancellation),
    )
    .await;

    cancellation.store(true, Ordering::Release);
    if outgoing_bridge.join().is_err() {
        tracing::error!("multiplayer client outgoing bridge panicked");
    }
}

async fn run_client_io_inner(
    addr: String,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_async_rx: &mut tokio::sync::mpsc::UnboundedReceiver<NetOutbound>,
    assigned: Arc<Mutex<Option<PlayerId>>>,
    initial_handshake_tx: std::sync::mpsc::SyncSender<Result<(PlayerId, u64), String>>,
    cancellation: Arc<AtomicBool>,
) {
    let (mut ws, your_seat, mission_seed) = {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut backoff = std::time::Duration::from_millis(50);
        loop {
            if cancellation.load(Ordering::Acquire) {
                let _ = initial_handshake_tx.send(Err("transport cancelled".into()));
                return;
            }
            let Some(handshake) = handshake_or_cancel(&addr, &nickname, &cancellation).await else {
                let _ = initial_handshake_tx.send(Err("transport cancelled".into()));
                return;
            };
            match handshake {
                Ok(result) => break result,
                Err(err) if tokio::time::Instant::now() < deadline => {
                    tracing::debug!("initial multiplayer handshake failed: {err}; retrying");
                    if sleep_or_cancel(backoff, &cancellation).await {
                        let _ = initial_handshake_tx.send(Err("transport cancelled".into()));
                        return;
                    }
                    backoff = (backoff * 2).min(std::time::Duration::from_millis(500));
                }
                Err(err) => {
                    let _ = initial_handshake_tx.send(Err(err));
                    return;
                }
            }
        }
    };

    *assigned.lock().unwrap() = Some(your_seat);
    let _ = incoming_tx.send(NetEvent::AssignedLocalSeat(your_seat));
    let _ = incoming_tx.send(NetEvent::MissionSeed(mission_seed));
    let _ = initial_handshake_tx.send(Ok((your_seat, mission_seed)));

    let mut backoff = std::time::Duration::from_millis(500);
    loop {
        match run_session_async(
            ws,
            &incoming_tx,
            outgoing_async_rx,
            Arc::clone(&cancellation),
        )
        .await
        {
            SessionEnd::Graceful => break,
            SessionEnd::Drop(reason) => {
                tracing::warn!("client session ended: {reason}; reconnecting...");
                let _ = incoming_tx.send(NetEvent::Note(format!(
                    "disconnected: {reason}; reconnecting..."
                )));
                let _ = incoming_tx.send(NetEvent::Disconnected);
            }
            SessionEnd::OutgoingClosed => return,
        }

        if sleep_or_cancel(backoff, &cancellation).await {
            return;
        }
        backoff = (backoff * 2).min(std::time::Duration::from_secs(10));

        ws = loop {
            if cancellation.load(Ordering::Acquire) {
                return;
            }
            let Some(handshake) = handshake_or_cancel(&addr, &nickname, &cancellation).await else {
                return;
            };
            match handshake {
                Ok((new_ws, new_seat, new_seed)) => {
                    tracing::info!(?new_seat, seed = new_seed, "client reconnected");
                    *assigned.lock().unwrap() = Some(new_seat);
                    let _ = incoming_tx.send(NetEvent::Reconnected);
                    let _ = incoming_tx.send(NetEvent::AssignedLocalSeat(new_seat));
                    let _ = incoming_tx.send(NetEvent::MissionSeed(new_seed));
                    backoff = std::time::Duration::from_millis(500);
                    break new_ws;
                }
                Err(e) => {
                    tracing::warn!("reconnect failed: {e}; will retry in {backoff:?}");
                    if sleep_or_cancel(backoff, &cancellation).await {
                        return;
                    }
                    backoff = (backoff * 2).min(std::time::Duration::from_secs(10));
                }
            }
        };
    }

    let _ = incoming_tx.send(NetEvent::Disconnected);
}

/// Why a client session ended.
enum SessionEnd {
    /// Server closed cleanly (Close frame received).
    Graceful,
    /// Network error / unexpected drop — caller should retry.
    Drop(String),
    /// The game loop dropped the outgoing channel — caller should
    /// stop the I/O thread entirely (no retry).
    OutgoingClosed,
}

/// Run one client session by selecting over inbound WebSocket frames
/// and outbound game-loop messages.  This avoids the old read-timeout
/// polling loop, so local inputs are sent as soon as the game loop
/// queues them.
async fn run_session_async(
    mut ws: AsyncWs,
    incoming_tx: &Sender<NetEvent>,
    outgoing_rx: &mut tokio::sync::mpsc::UnboundedReceiver<NetOutbound>,
    cancellation: Arc<AtomicBool>,
) -> SessionEnd {
    loop {
        tokio::select! {
            _ = wait_for_cancel(&cancellation) => return SessionEnd::OutgoingClosed,
            incoming = ws.next() => {
                let Some(incoming) = incoming else {
                    return SessionEnd::Graceful;
                };
                match incoming {
                    Ok(WsMessage::Binary(b)) => handle_client_wire_frame(incoming_tx, &b),
                    Ok(WsMessage::Close(_)) => return SessionEnd::Graceful,
                    Ok(_) => {}
                    Err(e) => return SessionEnd::Drop(format!("read: {e}")),
                }
            }
            outgoing = outgoing_rx.recv() => {
                let Some(outgoing) = outgoing else {
                    return SessionEnd::OutgoingClosed;
                };
                if let Err(e) = send_client_outgoing(&mut ws, outgoing).await {
                    return SessionEnd::Drop(e);
                }
            }
        }
    }
}

async fn wait_for_cancel(cancellation: &AtomicBool) {
    while !cancellation.load(Ordering::Acquire) {
        tokio::time::sleep(WORKER_POLL_INTERVAL).await;
    }
}

/// Sleep for a reconnect backoff, returning early when shutdown begins.
async fn sleep_or_cancel(duration: Duration, cancellation: &AtomicBool) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = wait_for_cancel(cancellation) => true,
    }
}

fn handle_client_wire_frame(incoming_tx: &Sender<NetEvent>, bytes: &[u8]) {
    match decode_msg(bytes) {
        Ok(NetMsg::BroadcastInput {
            server_frame,
            origin_frame,
            target_frame,
            input,
        }) => {
            let _ = incoming_tx.send(NetEvent::Input {
                server_frame,
                origin_frame,
                target_frame,
                input,
            });
        }
        Ok(NetMsg::Note(s)) => {
            let _ = incoming_tx.send(NetEvent::Note(s));
        }
        Ok(NetMsg::StateHash {
            frame,
            hash,
            clock_frame,
            ms_until_next_frame,
        }) => {
            let _ = incoming_tx.send(NetEvent::PeerStateHash {
                frame,
                hash,
                clock_frame,
                ms_until_next_frame,
            });
        }
        Ok(NetMsg::InitialSnapshot {
            frame,
            engine_bytes,
        }) => {
            let _ = incoming_tx.send(NetEvent::InitialSnapshot {
                frame,
                engine_bytes,
            });
        }
        Ok(NetMsg::BeginSim {
            frame,
            start_epoch_ms,
        }) => {
            let _ = incoming_tx.send(NetEvent::BeginSim {
                frame,
                start_epoch_ms,
            });
        }
        Ok(NetMsg::ModalDismiss { kind, result }) => {
            let _ = incoming_tx.send(NetEvent::ModalDismiss { kind, result });
        }
        Ok(other) => {
            tracing::debug!(?other, "ignoring unexpected wire message");
        }
        Err(e) => {
            tracing::warn!("decode error: {e}");
        }
    }
}

async fn send_client_outgoing(ws: &mut AsyncWs, outgoing: NetOutbound) -> Result<(), String> {
    match outgoing {
        NetOutbound::Input {
            origin_frame,
            command,
        } => {
            let frame = encode_msg(&NetMsg::Input {
                origin_frame,
                command,
            });
            ws.send(WsMessage::Binary(frame.into()))
                .await
                .map_err(|e| format!("send: {e}"))?;
        }
        NetOutbound::StateHash { .. } => {
            // Clients don't broadcast hashes.
        }
        NetOutbound::InitialSnapshot { .. } => {
            // Clients do not publish authoritative snapshots.
        }
        NetOutbound::ReadyToSim { frame } => {
            let frame = encode_msg(&NetMsg::ReadyToSim { frame });
            ws.send(WsMessage::Binary(frame.into()))
                .await
                .map_err(|e| format!("send: {e}"))?;
        }
        NetOutbound::ModalDismiss { kind, result } => {
            let frame = encode_msg(&NetMsg::ModalDismiss { kind, result });
            ws.send(WsMessage::Binary(frame.into()))
                .await
                .map_err(|e| format!("send: {e}"))?;
        }
    }
    Ok(())
}
