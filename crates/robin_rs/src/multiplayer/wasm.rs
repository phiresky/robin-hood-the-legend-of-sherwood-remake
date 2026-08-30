//! Browser iroh multiplayer client.
//!
//! Browsers cannot use iroh's UDP discovery paths. The endpoint therefore
//! dials the one host-signed HTTPS relay route from a browser join ticket and
//! carries the native game's unchanged ALPN, bidirectional stream, framing,
//! admission, rollback, and snapshot protocol over the relay WebSocket.
//!
//! Browser hosting and DHT discovery remain deliberately unsupported.
//! TODO(browser-webrtc): if iroh gains a production WebRTC path, add it below
//! this endpoint abstraction instead of inventing a second game protocol.

use super::join_ticket::BrowserJoinTicket;
use super::{
    InboundFramePolicy, NET_PROTOCOL_VERSION, NetEvent, NetFrameClass, NetMsg, NetOutbound,
    decode_msg, encode_msg, net_frame_class,
};
use futures::future::{Either, select};
use futures::{FutureExt as _, pin_mut};
use gloo_timers::future::TimeoutFuture;
use iroh::endpoint::{Connection, ReadExactError, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use robin_engine::multiplayer::{BrowserPeerAuth, browser_seat_proof_message};
use robin_engine::player_command::PlayerId;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use wasm_bindgen::JsCast as _;

const GAME_ALPN: &[u8] = b"robinhood/game/0";
const OUTGOING_POLL_MS: u32 = 4;
const INITIAL_CONNECT_TIMEOUT_MS: u32 = 15_000;
const RELAY_ONLINE_TIMEOUT_MS: u32 = 15_000;
const MAX_RECONNECT_BACKOFF_MS: u32 = 10_000;

/// Browser-side handle to the live single-threaded iroh task.
pub struct ClientHandle {
    pub assigned_seat: Rc<RefCell<Option<PlayerId>>>,
    pub session_id: Rc<RefCell<Option<robin_engine::multiplayer::MultiplayerSessionId>>>,
    pub mission_seed: Rc<RefCell<Option<u64>>>,
    pub mission_sim_config: Rc<RefCell<Option<robin_engine::engine::SimConfig>>>,
    pub speech_timing_locale: Rc<RefCell<Option<Option<String>>>>,
    pub mission_id: Rc<RefCell<Option<String>>>,
    startup_error: Rc<RefCell<Option<String>>>,
    cancellation: Rc<Cell<bool>>,
}

impl ClientHandle {
    pub fn session_id(&self) -> Option<robin_engine::multiplayer::MultiplayerSessionId> {
        *self.session_id.borrow()
    }

    pub fn assigned_seat(&self) -> Option<PlayerId> {
        *self.assigned_seat.borrow()
    }

    pub fn mission_seed(&self) -> Option<u64> {
        *self.mission_seed.borrow()
    }

    pub fn mission_sim_config(&self) -> Option<robin_engine::engine::SimConfig> {
        *self.mission_sim_config.borrow()
    }

    pub fn mission_id(&self) -> Option<String> {
        self.mission_id.borrow().clone()
    }

    pub fn speech_timing_locale(&self) -> Option<String> {
        self.speech_timing_locale.borrow().clone().flatten()
    }

    /// The outer option distinguishes a pending handshake from an explicit
    /// `None`, which authoritatively selects base `Data/Sounds` timing.
    pub fn speech_timing_authority(&self) -> Option<Option<String>> {
        self.speech_timing_locale.borrow().clone()
    }

    pub fn startup_error(&self) -> Option<String> {
        self.startup_error.borrow().clone()
    }

    pub fn shutdown(&mut self) {
        self.cancellation.set(true);
    }
}

impl Drop for ClientHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Start a relay-only browser connection without blocking the JavaScript
/// event loop. Mission setup asynchronously waits on the authoritative
/// `Welcome` fields exposed by the returned handle.
pub fn connect_client(
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    robin_engine::multiplayer::validate_display_name(&nickname).map_err(std::io::Error::other)?;
    let ticket =
        BrowserJoinTicket::decode_authenticated(addr.as_ref()).map_err(std::io::Error::other)?;
    let server_addr = ticket.endpoint_addr().map_err(std::io::Error::other)?;
    let assigned_seat = Rc::new(RefCell::new(None));
    let session_id = Rc::new(RefCell::new(None));
    let mission_seed = Rc::new(RefCell::new(None));
    let mission_sim_config = Rc::new(RefCell::new(None));
    let speech_timing_locale = Rc::new(RefCell::new(None));
    let mission_id = Rc::new(RefCell::new(None));
    let startup_error = Rc::new(RefCell::new(None));
    let cancellation = Rc::new(Cell::new(false));

    wasm_bindgen_futures::spawn_local(run_client_io(
        ticket,
        server_addr,
        nickname,
        incoming_tx,
        outgoing_rx,
        Rc::clone(&assigned_seat),
        Rc::clone(&session_id),
        Rc::clone(&mission_seed),
        Rc::clone(&mission_sim_config),
        Rc::clone(&speech_timing_locale),
        Rc::clone(&mission_id),
        Rc::clone(&startup_error),
        Rc::clone(&cancellation),
    ));

    Ok(ClientHandle {
        assigned_seat,
        session_id,
        mission_seed,
        mission_sim_config,
        speech_timing_locale,
        mission_id,
        startup_error,
        cancellation,
    })
}

async fn write_frame(send: &mut SendStream, message: &NetMsg) -> Result<(), String> {
    let bytes = encode_msg(message);
    let class = net_frame_class(message);
    if bytes.len() > class.absolute_limit() {
        return Err(format!(
            "outbound {class:?} frame of {} bytes exceeds {}-byte limit",
            bytes.len(),
            class.absolute_limit()
        ));
    }
    let len = u32::try_from(bytes.len()).map_err(|_| "outbound frame exceeds u32".to_string())?;
    let mut header = [0_u8; 5];
    header[0] = class as u8;
    header[1..].copy_from_slice(&len.to_le_bytes());
    send.write_all(&header)
        .await
        .map_err(|error| format!("write frame header: {error}"))?;
    send.write_all(&bytes)
        .await
        .map_err(|error| format!("write frame body: {error}"))?;
    Ok(())
}

async fn read_frame(recv: &mut RecvStream) -> Result<Option<NetMsg>, String> {
    let mut header = [0_u8; 5];
    match recv.read_exact(&mut header).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(error) => return Err(format!("read frame header: {error}")),
    }
    let class = NetFrameClass::from_byte(header[0])?;
    let len = u32::from_le_bytes(header[1..].try_into().expect("four-byte frame length")) as usize;
    let limit = InboundFramePolicy::ServerToClient
        .limit(class)
        .ok_or_else(|| format!("server may not send {class:?} frames"))?;
    if len > limit {
        return Err(format!(
            "inbound {class:?} frame of {len} bytes exceeds {limit}-byte browser limit"
        ));
    }
    let mut bytes = vec![0; len];
    recv.read_exact(&mut bytes)
        .await
        .map_err(|error| format!("read frame body: {error}"))?;
    let message = decode_msg(&bytes).map_err(|error| format!("decode frame: {error}"))?;
    if net_frame_class(&message) != class {
        return Err(format!(
            "declared {class:?} frame decoded as {:?}",
            net_frame_class(&message)
        ));
    }
    Ok(Some(message))
}

async fn with_timeout<T>(millis: u32, future: impl Future<Output = T>) -> Result<T, ()> {
    let future = future.fuse();
    let timeout = TimeoutFuture::new(millis).fuse();
    pin_mut!(future, timeout);
    futures::select! {
        result = future => Ok(result),
        () = timeout => Err(()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_client_io(
    ticket: BrowserJoinTicket,
    server_addr: EndpointAddr,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    mut outgoing_rx: Receiver<NetOutbound>,
    assigned: Rc<RefCell<Option<PlayerId>>>,
    session_id_slot: Rc<RefCell<Option<robin_engine::multiplayer::MultiplayerSessionId>>>,
    mission_seed_slot: Rc<RefCell<Option<u64>>>,
    sim_config_slot: Rc<RefCell<Option<robin_engine::engine::SimConfig>>>,
    speech_timing_locale_slot: Rc<RefCell<Option<Option<String>>>>,
    mission_id_slot: Rc<RefCell<Option<String>>>,
    startup_error: Rc<RefCell<Option<String>>>,
    cancellation: Rc<Cell<bool>>,
) {
    let transport_key = SecretKey::generate();
    let browser_auth = match browser_peer_auth(&ticket, transport_key.public()).await {
        Ok(auth) => auth,
        Err(error) => {
            publish_startup_error(&startup_error, &incoming_tx, error);
            return;
        }
    };
    let endpoint = match Endpoint::builder(presets::N0)
        .secret_key(transport_key)
        .bind()
        .await
    {
        Ok(endpoint) => endpoint,
        Err(error) => {
            publish_startup_error(
                &startup_error,
                &incoming_tx,
                format!("start browser iroh endpoint: {error}"),
            );
            return;
        }
    };

    if with_timeout(RELAY_ONLINE_TIMEOUT_MS, endpoint.online())
        .await
        .is_err()
    {
        publish_startup_error(
            &startup_error,
            &incoming_tx,
            "iroh relay did not become reachable within 15 seconds; browser multiplayer requires WebSocket relay access"
                .to_string(),
        );
        endpoint.close().await;
        return;
    }

    let first = initial_handshake(
        &endpoint,
        &server_addr,
        &nickname,
        &browser_auth,
        &cancellation,
    )
    .await;
    let (
        mut session,
        your_seat,
        mission_id,
        mission_seed,
        sim_config,
        speech_timing_locale,
        session_id,
    ) = match first {
        Ok(result) => result,
        Err(error) => {
            publish_startup_error(&startup_error, &incoming_tx, error);
            endpoint.close().await;
            return;
        }
    };
    if let Err(error) = mark_invitation_redeemed(ticket.payload().session_id.as_str()).await {
        publish_startup_error(&startup_error, &incoming_tx, error);
        endpoint.close().await;
        return;
    }

    *assigned.borrow_mut() = Some(your_seat);
    *session_id_slot.borrow_mut() = Some(session_id);
    *mission_seed_slot.borrow_mut() = Some(mission_seed);
    *sim_config_slot.borrow_mut() = Some(sim_config);
    *speech_timing_locale_slot.borrow_mut() = Some(speech_timing_locale.clone());
    *mission_id_slot.borrow_mut() = Some(mission_id.clone());
    let _ = incoming_tx.send(NetEvent::AssignedLocalSeat(your_seat));
    let _ = incoming_tx.send(NetEvent::MissionConfig {
        mission_id: mission_id.clone(),
        rng_seed: mission_seed,
        sim_config,
        speech_timing_locale: speech_timing_locale.clone(),
    });

    let mut backoff_ms = 500_u32;
    while !cancellation.get() {
        match run_session(session, &incoming_tx, &mut outgoing_rx, &cancellation).await {
            SessionEnd::OutgoingClosed => break,
            SessionEnd::Drop(reason) => {
                let discarded = discard_session_outbound(&mut outgoing_rx);
                tracing::warn!(
                    %reason,
                    discarded,
                    "browser multiplayer session ended; reconnecting through iroh relay"
                );
                let _ = incoming_tx.send(NetEvent::Note(format!(
                    "iroh relay disconnected: {reason}; reconnecting..."
                )));
                let _ = incoming_tx.send(NetEvent::Disconnected);
            }
        }

        sleep_or_cancel(backoff_ms, &cancellation).await;
        backoff_ms = (backoff_ms.saturating_mul(2)).min(MAX_RECONNECT_BACKOFF_MS);

        session = loop {
            if cancellation.get() {
                endpoint.close().await;
                return;
            }
            match handshake(
                &endpoint,
                &server_addr,
                &nickname,
                &browser_auth,
                session_id,
            )
            .await
            {
                Ok((
                    next,
                    next_seat,
                    next_mission,
                    next_seed,
                    next_config,
                    next_speech_timing_locale,
                    next_session_id,
                )) => {
                    if let Err(error) = validate_reconnect_state(
                        your_seat,
                        &mission_id,
                        mission_seed,
                        sim_config,
                        speech_timing_locale.as_deref(),
                        session_id,
                        next_seat,
                        &next_mission,
                        next_seed,
                        next_config,
                        next_speech_timing_locale.as_deref(),
                        next_session_id,
                    ) {
                        let _ = incoming_tx.send(NetEvent::Fatal(error));
                        endpoint.close().await;
                        return;
                    }
                    let discarded = discard_session_outbound(&mut outgoing_rx);
                    if discarded != 0 {
                        tracing::warn!(
                            discarded,
                            "discarded browser commands queued for the abandoned prediction future"
                        );
                    }
                    *assigned.borrow_mut() = Some(next_seat);
                    *speech_timing_locale_slot.borrow_mut() =
                        Some(next_speech_timing_locale.clone());
                    let _ = incoming_tx.send(NetEvent::Reconnected);
                    let _ = incoming_tx.send(NetEvent::AssignedLocalSeat(next_seat));
                    let _ = incoming_tx.send(NetEvent::MissionConfig {
                        mission_id: next_mission,
                        rng_seed: next_seed,
                        sim_config: next_config,
                        speech_timing_locale: next_speech_timing_locale,
                    });
                    backoff_ms = 500;
                    break next;
                }
                Err(error) => {
                    tracing::warn!(%error, backoff_ms, "browser iroh relay reconnect failed");
                    sleep_or_cancel(backoff_ms, &cancellation).await;
                    backoff_ms = (backoff_ms.saturating_mul(2)).min(MAX_RECONNECT_BACKOFF_MS);
                }
            }
        };
    }

    endpoint.close().await;
}

fn publish_startup_error(
    slot: &Rc<RefCell<Option<String>>>,
    incoming_tx: &Sender<NetEvent>,
    error: String,
) {
    *slot.borrow_mut() = Some(error.clone());
    let _ = incoming_tx.send(NetEvent::Fatal(error));
}

async fn initial_handshake(
    endpoint: &Endpoint,
    server_addr: &EndpointAddr,
    nickname: &str,
    browser_auth: &BrowserPeerAuth,
    cancellation: &Cell<bool>,
) -> Result<Handshake, String> {
    let started = web_time::Instant::now();
    let mut backoff_ms = 50_u32;
    let mut last_error = "host has not accepted the connection".to_string();
    let expected_session_id = robin_engine::multiplayer::MultiplayerSessionId(
        BrowserJoinTicket::decode_authenticated(&browser_auth.join_code)?.session_id()?,
    );
    while started.elapsed().as_millis() < u128::from(INITIAL_CONNECT_TIMEOUT_MS) {
        if cancellation.get() {
            return Err("browser multiplayer connection cancelled".to_string());
        }
        match with_timeout(
            5_000,
            handshake(
                endpoint,
                server_addr,
                nickname,
                browser_auth,
                expected_session_id,
            ),
        )
        .await
        {
            Ok(Ok(result)) => return Ok(result),
            Ok(Err(error)) => last_error = error,
            Err(()) => last_error = "iroh relay connection attempt timed out".to_string(),
        }
        sleep_or_cancel(backoff_ms, cancellation).await;
        backoff_ms = (backoff_ms.saturating_mul(2)).min(500);
    }
    Err(format!(
        "could not reach the host through the iroh WebSocket relay within 15 seconds: {last_error}"
    ))
}

type Handshake = (
    ClientSession,
    PlayerId,
    String,
    u64,
    robin_engine::engine::SimConfig,
    Option<String>,
    robin_engine::multiplayer::MultiplayerSessionId,
);

struct ClientSession {
    _connection: Connection,
    send: SendStream,
    recv: RecvStream,
}

async fn handshake(
    endpoint: &Endpoint,
    server_addr: &EndpointAddr,
    nickname: &str,
    browser_auth: &BrowserPeerAuth,
    expected_session_id: robin_engine::multiplayer::MultiplayerSessionId,
) -> Result<Handshake, String> {
    let connection = endpoint
        .connect(server_addr.clone(), GAME_ALPN)
        .await
        .map_err(|error| format!("iroh relay connect: {error}"))?;
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|error| format!("open multiplayer stream: {error}"))?;
    write_frame(
        &mut send,
        &NetMsg::Hello {
            protocol_version: NET_PROTOCOL_VERSION,
            nickname: nickname.to_string(),
            browser_auth: Some(browser_auth.clone()),
        },
    )
    .await
    .map_err(|error| format!("send Hello: {error}"))?;

    match read_frame(&mut recv).await? {
        Some(NetMsg::Welcome {
            your_seat,
            mission_id,
            mission_seed,
            sim_config,
            speech_timing_locale,
            host_nickname,
            session_id,
        }) => {
            if session_id != expected_session_id {
                return Err("host Welcome session does not match the signed invitation".to_string());
            }
            tracing::info!(
                ?your_seat,
                seed = mission_seed,
                host = %host_nickname,
                "browser welcomed through iroh WebSocket relay"
            );
            Ok((
                ClientSession {
                    _connection: connection,
                    send,
                    recv,
                },
                your_seat,
                mission_id,
                mission_seed,
                sim_config,
                speech_timing_locale,
                session_id,
            ))
        }
        Some(NetMsg::Reject { reason }) => Err(format!("host rejected connection: {reason}")),
        Some(other) => Err(format!("expected Welcome, got {other:?}")),
        None => Err("host closed the stream before Welcome".to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_reconnect_state(
    expected_seat: PlayerId,
    expected_mission_id: &str,
    expected_seed: u64,
    expected_config: robin_engine::engine::SimConfig,
    expected_speech_timing_locale: Option<&str>,
    expected_session_id: robin_engine::multiplayer::MultiplayerSessionId,
    seat: PlayerId,
    mission_id: &str,
    seed: u64,
    config: robin_engine::engine::SimConfig,
    speech_timing_locale: Option<&str>,
    session_id: robin_engine::multiplayer::MultiplayerSessionId,
) -> Result<(), String> {
    if seat != expected_seat
        || mission_id != expected_mission_id
        || seed != expected_seed
        || config != expected_config
        || speech_timing_locale != expected_speech_timing_locale
        || session_id != expected_session_id
    {
        return Err(format!(
            "browser reconnect joined incompatible seat {seat:?} mission `{mission_id}` seed {seed} config {config:?} speech timing {speech_timing_locale:?} session {session_id:?}; expected seat {expected_seat:?} mission `{expected_mission_id}` seed {expected_seed} config {expected_config:?} speech timing {expected_speech_timing_locale:?} session {expected_session_id:?}"
        ));
    }
    Ok(())
}

enum SessionEnd {
    Drop(String),
    OutgoingClosed,
}

async fn run_session(
    session: ClientSession,
    incoming_tx: &Sender<NetEvent>,
    outgoing_rx: &mut Receiver<NetOutbound>,
    cancellation: &Cell<bool>,
) -> SessionEnd {
    let ClientSession {
        _connection,
        mut send,
        mut recv,
    } = session;
    let reader = async {
        loop {
            match read_frame(&mut recv).await {
                Ok(Some(message)) => {
                    if let Err(error) = handle_client_wire_msg(incoming_tx, message) {
                        return SessionEnd::Drop(error);
                    }
                }
                Ok(None) => {
                    return SessionEnd::Drop("host closed the multiplayer stream".to_string());
                }
                Err(error) => return SessionEnd::Drop(error),
            }
        }
    }
    .boxed_local();
    let writer = async {
        loop {
            if cancellation.get() {
                return SessionEnd::OutgoingClosed;
            }
            match outgoing_rx.try_recv() {
                Ok(outgoing) => {
                    if let Err(error) = send_client_outgoing(&mut send, outgoing).await {
                        return SessionEnd::Drop(error);
                    }
                }
                Err(TryRecvError::Empty) => TimeoutFuture::new(OUTGOING_POLL_MS).await,
                Err(TryRecvError::Disconnected) => return SessionEnd::OutgoingClosed,
            }
        }
    }
    .boxed_local();

    match select(reader, writer).await {
        Either::Left((end, _writer)) => end,
        Either::Right((end, _reader)) => end,
    }
}

fn handle_client_wire_msg(incoming_tx: &Sender<NetEvent>, message: NetMsg) -> Result<(), String> {
    match message {
        NetMsg::BroadcastInput {
            server_frame,
            origin_frame,
            target_frame,
            input,
        } => {
            let _ = incoming_tx.send(NetEvent::Input {
                server_frame,
                origin_frame,
                target_frame,
                input,
            });
        }
        NetMsg::Note(note) => {
            let _ = incoming_tx.send(NetEvent::Note(note));
        }
        NetMsg::StateHash {
            frame,
            hash,
            clock_frame,
            ms_until_next_frame,
        } => {
            let _ = incoming_tx.send(NetEvent::PeerStateHash {
                frame,
                hash,
                clock_frame,
                ms_until_next_frame,
            });
        }
        NetMsg::InitialSnapshot {
            frame,
            engine_bytes,
        } => {
            let _ = incoming_tx.send(NetEvent::InitialSnapshot {
                frame,
                engine_bytes,
            });
        }
        NetMsg::BeginSim {
            frame,
            start_epoch_ms,
        } => {
            let _ = incoming_tx.send(NetEvent::BeginSim {
                frame,
                start_epoch_ms,
            });
        }
        NetMsg::ModalDecision {
            instance,
            kind,
            result,
            decision_frame,
        } => {
            incoming_tx
                .send(NetEvent::ModalDecision {
                    instance,
                    kind,
                    result,
                    decision_frame,
                })
                .map_err(|_| "browser modal decision channel is closed".to_string())?;
        }
        NetMsg::ReconnectRequired { reason } => {
            return Err(format!("host requires a full-snapshot reconnect: {reason}"));
        }
        NetMsg::PrepareSnapshotTransition { id, payload } => {
            incoming_tx
                .send(NetEvent::PrepareSnapshotTransition { id, payload })
                .map_err(|_| "browser snapshot transition channel is closed".to_string())?;
        }
        NetMsg::CommitSnapshotTransition { id } => {
            incoming_tx
                .send(NetEvent::CommitSnapshotTransition { id })
                .map_err(|_| "browser snapshot transition channel is closed".to_string())?;
        }
        NetMsg::ModalProposal { .. } | NetMsg::SnapshotTransitionReady { .. } => {
            return Err("host sent a client-only multiplayer message".to_string());
        }
        NetMsg::Reject { reason } => return Err(format!("host rejected session: {reason}")),
        other => {
            return Err(format!(
                "host sent invalid browser session message {other:?}"
            ));
        }
    }
    Ok(())
}

async fn send_client_outgoing(send: &mut SendStream, outgoing: NetOutbound) -> Result<(), String> {
    match outgoing {
        NetOutbound::Input {
            origin_frame,
            command,
        } => {
            write_frame(
                send,
                &NetMsg::Input {
                    origin_frame,
                    command,
                },
            )
            .await?;
        }
        NetOutbound::StateHash { .. }
        | NetOutbound::InitialSnapshot { .. }
        | NetOutbound::ModalDecision { .. }
        | NetOutbound::ReconnectForSnapshot { .. }
        | NetOutbound::ReconnectAllForSnapshot { .. }
        | NetOutbound::BeginSnapshotTransition { .. } => {
            return Err("browser client attempted a host-only multiplayer publication".to_string());
        }
        NetOutbound::ReadyToSim { frame } => {
            write_frame(send, &NetMsg::ReadyToSim { frame }).await?;
        }
        NetOutbound::ModalProposal {
            instance,
            kind,
            result,
            requested_frame,
        } => {
            write_frame(
                send,
                &NetMsg::ModalProposal {
                    instance,
                    kind,
                    result,
                    requested_frame,
                },
            )
            .await?;
        }
        NetOutbound::SnapshotTransitionReady { id } => {
            write_frame(send, &NetMsg::SnapshotTransitionReady { id }).await?;
        }
    }
    Ok(())
}

fn discard_session_outbound(outgoing_rx: &mut Receiver<NetOutbound>) -> usize {
    let mut discarded = 0;
    while outgoing_rx.try_recv().is_ok() {
        discarded += 1;
    }
    discarded
}

async fn sleep_or_cancel(millis: u32, cancellation: &Cell<bool>) {
    let mut elapsed = 0;
    while elapsed < millis && !cancellation.get() {
        let step = (millis - elapsed).min(50);
        TimeoutFuture::new(step).await;
        elapsed += step;
    }
}

fn js_error(prefix: &str, error: wasm_bindgen::JsValue) -> String {
    format!(
        "{prefix}: {}",
        error.as_string().unwrap_or_else(|| format!("{error:?}"))
    )
}

async fn browser_peer_auth(
    ticket: &BrowserJoinTicket,
    transport_endpoint_id: EndpointId,
) -> Result<BrowserPeerAuth, String> {
    let global = js_sys::global();
    let identity = js_sys::Reflect::get(
        &global,
        &wasm_bindgen::JsValue::from_str("robinMultiplayerIdentity"),
    )
    .map_err(|error| js_error("read browser multiplayer identity", error))?;
    if identity.is_null() || identity.is_undefined() {
        return Err(
            "browser multiplayer identity was not installed by the stable shell".to_string(),
        );
    }
    let raw_public = js_sys::Reflect::get(&identity, &wasm_bindgen::JsValue::from_str("publicKey"))
        .map_err(|error| js_error("read durable browser public key", error))?;
    if !raw_public.is_instance_of::<js_sys::Uint8Array>() {
        return Err("stable shell supplied a malformed durable browser public key".to_string());
    }
    let durable_public_key: [u8; 32] = js_sys::Uint8Array::new(&raw_public)
        .to_vec()
        .try_into()
        .map_err(|_| "durable browser public key must be 32 bytes".to_string())?;
    let raw_sign = js_sys::Reflect::get(&identity, &wasm_bindgen::JsValue::from_str("sign"))
        .map_err(|error| js_error("read durable browser signer", error))?;
    let sign = raw_sign
        .dyn_into::<js_sys::Function>()
        .map_err(|_| "stable shell supplied a malformed durable browser signer".to_string())?;
    let message = browser_seat_proof_message(
        ticket.session_id()?,
        *ticket.endpoint_addr()?.id.as_bytes(),
        *transport_endpoint_id.as_bytes(),
    );
    let promise = sign
        .call1(
            &identity,
            &js_sys::Uint8Array::from(message.as_slice()).into(),
        )
        .map_err(|error| js_error("request durable browser seat proof", error))?;
    let signature = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&promise))
        .await
        .map_err(|error| js_error("sign durable browser seat proof", error))?;
    if !signature.is_instance_of::<js_sys::Uint8Array>() {
        return Err("durable browser signer returned a malformed signature".to_string());
    }
    let signature = js_sys::Uint8Array::new(&signature).to_vec();
    if signature.len() != iroh::Signature::LENGTH {
        return Err(format!(
            "durable browser signer returned a {}-byte signature",
            signature.len()
        ));
    }
    Ok(BrowserPeerAuth {
        join_code: ticket.encode(),
        durable_public_key,
        signature,
    })
}

async fn mark_invitation_redeemed(session_id: &str) -> Result<(), String> {
    let global = js_sys::global();
    let raw_mark = js_sys::Reflect::get(
        &global,
        &wasm_bindgen::JsValue::from_str("robinMarkMultiplayerInvitationRedeemed"),
    )
    .map_err(|error| js_error("read invitation redemption store", error))?;
    let mark = raw_mark
        .dyn_into::<js_sys::Function>()
        .map_err(|_| "stable shell supplied a malformed invitation redemption store".to_string())?;
    let promise = mark
        .call1(
            &wasm_bindgen::JsValue::UNDEFINED,
            &wasm_bindgen::JsValue::from_str(session_id),
        )
        .map_err(|error| js_error("record invitation redemption", error))?;
    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&promise))
        .await
        .map_err(|error| js_error("persist invitation redemption", error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_reconnect_state;
    use robin_engine::player_command::PlayerId;

    #[test]
    fn reconnect_requires_the_same_seat_and_authority() {
        let expected = robin_engine::engine::SimConfig::default();
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "A",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "A",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_ok()
        );
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "A",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(2),
                "A",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_err()
        );
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "A",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "A",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([2; 32]),
            )
            .is_err()
        );
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "A",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "A",
                7,
                expected,
                Some("de-DE"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_err()
        );
    }
}
