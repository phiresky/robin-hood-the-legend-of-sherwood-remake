//! The one multiplayer client session state machine, shared by the native
//! (iroh on tokio) and browser (iroh relay on wasm) clients.
//!
//! Handshake -> content admission -> Welcome -> session -> reconnect is
//! implemented once here, generic over [`ClientTransport`]. Each platform
//! supplies only what genuinely differs:
//!
//! - [`ClientTimer`]: how to sleep (tokio timers vs browser timers).
//! - [`ClientTransport`]: endpoint and `Hello` contents, how local outbound
//!   commands are received (tokio bridge vs polled `std::sync::mpsc`), how
//!   startup progress/failure is reported to the caller (blocking handshake
//!   channel vs handle slots), and timings.
//!
//! Session rules decided here, once, for both platforms:
//!
//! - A clean stream close is a transport drop and reconnects ([`reader_outcome`]).
//! - A host `ReconnectRequired` directive or a failed local publication drops
//!   the session and reconnects; a trust violation (`Reject`, wrong-direction
//!   message) is fatal.

use super::client_gameplay::{deliver, deliver_lifecycle};
use super::client_protocol::{
    ClientHandshake, ClientSessionMetadata, HandshakeAction, ReconnectIdentity, WelcomeData,
    validate_reconnect_content, validate_reconnect_state,
};
use super::content_transfer::{ContentDecision, accept_chunk};
use super::framing::{read_frame, write_frame};
use super::identity::GAME_ALPN;
use super::{InboundFramePolicy, MultiplayerError, NetEvent, NetMsg, NetOutbound};
use futures::future::{Either, select};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr};
use robin_engine::multiplayer::{DistributedModOffer, MultiplayerSessionId, NetFatal};
use robin_engine::player_command::PlayerId;
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Cancellation is a flag set by [`ClientHandle::shutdown`]; waiters poll it.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(20);
const INITIAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const INITIAL_BACKOFF: Duration = Duration::from_millis(50);
const MAX_INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const RECONNECT_BACKOFF: Duration = Duration::from_millis(500);
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(10);

// ─── Handle ──────────────────────────────────────────────────────

/// State published by the client worker and read by the game through its
/// [`ClientHandle`]. Cloned into the worker.
///
/// Not serde: live shared slots.
#[derive(Clone)]
pub(super) struct ClientSlots {
    pub(super) session_metadata: Arc<Mutex<Option<ClientSessionMetadata>>>,
    /// Present when the host requires content admission before Welcome. The
    /// game/menu must explicitly trust, download, validate, mount, and answer
    /// this exact offer; the transport never silently approves it.
    pub(super) content_offer: Arc<Mutex<Option<DistributedModOffer>>>,
    pub(super) startup_error: Arc<Mutex<Option<MultiplayerError>>>,
    pub(super) cancellation: Arc<AtomicBool>,
}

impl ClientSlots {
    pub(super) fn new() -> Self {
        Self {
            session_metadata: Arc::new(Mutex::new(None)),
            content_offer: Arc::new(Mutex::new(None)),
            startup_error: Arc::new(Mutex::new(None)),
            cancellation: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn set_startup_error(&self, error: MultiplayerError) {
        *slot(&self.startup_error) = Some(error);
    }
}

/// Lock a published slot. A panicking holder cannot leave a half-written
/// value (every write is a single assignment), so poisoning is ignored like
/// the native transport's non-poisoning locks.
fn slot<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Joins the platform worker (the native I/O thread) once cancellation is set.
pub(super) type ClientWorker = Box<dyn FnOnce() + Send + Sync>;

/// Handle to an active client connection.
pub struct ClientHandle {
    slots: ClientSlots,
    worker: Option<ClientWorker>,
}

impl ClientHandle {
    pub(super) fn new(slots: ClientSlots, worker: Option<ClientWorker>) -> Self {
        Self { slots, worker }
    }

    pub fn session_metadata(&self) -> Option<ClientSessionMetadata> {
        slot(&self.slots.session_metadata).clone()
    }

    pub fn content_offer(&self) -> Option<DistributedModOffer> {
        slot(&self.slots.content_offer).clone()
    }

    /// Error that ended the connection before an authoritative Welcome.
    pub fn startup_error(&self) -> Option<MultiplayerError> {
        slot(&self.slots.startup_error).clone()
    }

    pub fn shutdown(&mut self) {
        self.slots.cancellation.store(true, Ordering::Release);
        if let Some(join_worker) = self.worker.take() {
            join_worker();
        }
    }
}

impl Drop for ClientHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ─── Platform seams ──────────────────────────────────────────────

pub(super) trait ClientTimer {
    fn sleep(duration: Duration) -> impl Future<Output = ()>;
}

/// Deadlines of one platform. `None` means the phase is unbounded apart from
/// cancellation.
pub(super) struct ClientTimings {
    /// Each Hello write and Welcome/offer read.
    pub(super) handshake_frame: Option<Duration>,
    /// One whole initial connect + Hello + Welcome attempt.
    pub(super) initial_attempt: Duration,
    /// One whole reconnect attempt.
    pub(super) reconnect_attempt: Option<Duration>,
    pub(super) post_content_welcome: Duration,
    pub(super) content_write: Option<Duration>,
    pub(super) content_chunk_idle: Duration,
    pub(super) content_transfer: Duration,
    pub(super) content_decision: Option<Duration>,
    pub(super) content_readiness: Option<Duration>,
}

/// Why a client session ended.
#[derive(Debug)]
pub(super) enum SessionEnd {
    /// Network error, unexpected drop or clean stream close — caller retries.
    /// A clean close is not an authoritative end of the session: the host ends
    /// sessions with `Reject` or `ReconnectRequired`, so a bare FIN is treated
    /// like any other transport loss (see [`reader_outcome`]).
    Drop(MultiplayerError),
    /// A direction or session invariant failed. Retrying the same
    /// authenticated session cannot repair this violation.
    Fatal(MultiplayerError),
    /// The game loop dropped the outgoing channel or shutdown began — stop
    /// the I/O task entirely (no retry).
    OutgoingClosed,
}

/// Progress of the first handshake, reported to a caller that blocks on it.
#[derive(Debug)]
pub(super) enum InitialHandshake {
    Welcomed { seat: PlayerId, mission_seed: u64 },
    ContentOffered { full_mod_sha256: [u8; 32] },
}

/// Phase in which the connection failed before gameplay started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StartupFailure {
    /// No handshake completed.
    Connect,
    /// Host content admission failed after the offer was published.
    Admission,
    /// The Welcome could not be admitted.
    Welcome,
}

pub(super) trait ClientTransport {
    type Timer: ClientTimer;
    /// Receiver of the game loop's outbound commands.
    type Outbound;

    const TIMINGS: ClientTimings;
    /// Startup error when shutdown begins before the first handshake.
    const CANCELLED: &'static str;

    fn cancellation(&self) -> &AtomicBool;
    fn endpoint(&self) -> &Endpoint;
    fn server_addr(&self) -> &EndpointAddr;
    fn hello(&self) -> NetMsg;
    /// Session the host Welcome must belong to (signed browser invitation).
    fn expected_session(&self) -> Option<MultiplayerSessionId>;

    /// Next local outbound command; `None` once the game loop dropped its
    /// sender.
    async fn recv_outbound(outbound: &mut Self::Outbound) -> Option<NetOutbound>;
    /// Throw away every queued outbound command; returns how many.
    fn discard_outbound(outbound: &mut Self::Outbound) -> usize;

    /// Wrap the last failure once the initial connect deadline has passed.
    fn initial_handshake_exhausted(last_error: MultiplayerError) -> MultiplayerError;

    fn publish_initial_handshake(&self, progress: InitialHandshake) -> Result<(), ()>;
    fn startup_failed(
        &self,
        slots: &ClientSlots,
        incoming: &Sender<NetEvent>,
        failure: StartupFailure,
        error: MultiplayerError,
    );
    /// Platform work between an admitted Welcome and its publication.
    async fn after_welcome(&self) -> Result<(), MultiplayerError>;
}

// ─── Timing helpers ──────────────────────────────────────────────

pub(super) async fn with_timeout<Tm: ClientTimer, F: Future>(
    timeout: Duration,
    future: F,
) -> Result<F::Output, ()> {
    let future = pin!(future);
    let timer = pin!(Tm::sleep(timeout));
    match select(future, timer).await {
        Either::Left((output, _)) => Ok(output),
        Either::Right(((), _)) => Err(()),
    }
}

async fn wait_for_cancel<Tm: ClientTimer>(cancellation: &AtomicBool) {
    while !cancellation.load(Ordering::Acquire) {
        Tm::sleep(CANCEL_POLL_INTERVAL).await;
    }
}

/// `None` when cancellation won the race.
async fn cancellable<Tm: ClientTimer, F: Future>(
    cancellation: &AtomicBool,
    future: F,
) -> Option<F::Output> {
    let future = pin!(future);
    let cancel = pin!(wait_for_cancel::<Tm>(cancellation));
    match select(future, cancel).await {
        Either::Left((output, _)) => Some(output),
        Either::Right(((), _)) => None,
    }
}

/// Sleep for a backoff, returning `true` when shutdown began first.
async fn sleep_or_cancel<Tm: ClientTimer>(cancellation: &AtomicBool, duration: Duration) -> bool {
    cancellable::<Tm, _>(cancellation, Tm::sleep(duration))
        .await
        .is_none()
}

async fn write_frame_within<Tm: ClientTimer>(
    send: &mut SendStream,
    message: &NetMsg,
    timeout: Option<Duration>,
    phase: &'static str,
) -> Result<(), MultiplayerError> {
    match timeout {
        None => write_frame(send, message).await,
        Some(timeout) => with_timeout::<Tm, _>(timeout, write_frame(send, message))
            .await
            .map_err(|()| MultiplayerError::timeout(phase, timeout))?,
    }
}

async fn read_frame_within<Tm: ClientTimer>(
    recv: &mut RecvStream,
    timeout: Option<Duration>,
    phase: &'static str,
) -> Result<Option<NetMsg>, MultiplayerError> {
    let read = read_frame(recv, InboundFramePolicy::ServerToClient);
    match timeout {
        None => read.await,
        Some(timeout) => with_timeout::<Tm, _>(timeout, read)
            .await
            .map_err(|()| MultiplayerError::timeout(phase, timeout))?,
    }
}

// ─── Handshake and content admission ─────────────────────────────

/// A live client session: the connection plus its single bidirectional
/// message stream.
#[derive(Debug)]
pub(super) struct ClientSession {
    // Held so the QUIC connection stays open for the streams' lifetime.
    _conn: Connection,
    send: SendStream,
    recv: RecvStream,
    protocol: ClientHandshake,
}

#[derive(Debug)]
pub(super) enum HandshakePrelude {
    Welcome {
        session: ClientSession,
        welcome: WelcomeData,
    },
    Content {
        session: ClientSession,
        offer: DistributedModOffer,
    },
}

/// One round of (connect → open stream → Hello → Welcome or content offer).
async fn handshake<T: ClientTransport>(
    transport: &T,
) -> Result<HandshakePrelude, MultiplayerError> {
    let server_addr = transport.server_addr();
    let conn = transport
        .endpoint()
        .connect(server_addr.clone(), GAME_ALPN)
        .await
        .map_err(|e| MultiplayerError::transport("connect", e))?;
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| MultiplayerError::transport("open stream", e))?;
    write_frame_within::<T::Timer>(
        &mut send,
        &transport.hello(),
        T::TIMINGS.handshake_frame,
        "client Hello",
    )
    .await
    .map_err(|e| e.context("send Hello"))?;
    let message = read_frame_within::<T::Timer>(
        &mut recv,
        T::TIMINGS.handshake_frame,
        "Welcome/content offer",
    )
    .await?;
    let mut protocol =
        ClientHandshake::new(server_addr.id.to_string(), transport.expected_session());
    let action = protocol.receive(message)?;
    let session = ClientSession {
        _conn: conn,
        send,
        recv,
        protocol,
    };
    Ok(match action {
        HandshakeAction::Welcome(welcome) => HandshakePrelude::Welcome { session, welcome },
        HandshakeAction::PrepareContent(offer) => HandshakePrelude::Content { session, offer },
    })
}

/// One bounded, cancellable handshake attempt; `None` when cancelled.
async fn attempt_handshake<T: ClientTransport>(
    transport: &T,
    timeout: Option<Duration>,
) -> Option<Result<HandshakePrelude, MultiplayerError>> {
    let attempt = async {
        match timeout {
            None => handshake(transport).await,
            Some(timeout) => with_timeout::<T::Timer, _>(timeout, handshake(transport))
                .await
                .unwrap_or_else(|()| {
                    Err(MultiplayerError::timeout("multiplayer handshake", timeout))
                }),
        }
    };
    cancellable::<T::Timer, _>(transport.cancellation(), attempt).await
}

async fn initial_handshake<T: ClientTransport>(
    transport: &T,
) -> Result<HandshakePrelude, MultiplayerError> {
    let started = web_time::Instant::now();
    let mut backoff = INITIAL_BACKOFF;
    loop {
        if transport.cancellation().load(Ordering::Acquire) {
            return Err(MultiplayerError::Cancelled(T::CANCELLED.into()));
        }
        let Some(attempt) = attempt_handshake(transport, Some(T::TIMINGS.initial_attempt)).await
        else {
            return Err(MultiplayerError::Cancelled(T::CANCELLED.into()));
        };
        let error = match attempt {
            Ok(prelude) => return Ok(prelude),
            Err(error) => error,
        };
        if started.elapsed() >= INITIAL_CONNECT_TIMEOUT {
            return Err(T::initial_handshake_exhausted(error));
        }
        tracing::debug!("initial multiplayer handshake failed: {error}; retrying");
        if sleep_or_cancel::<T::Timer>(transport.cancellation(), backoff).await {
            return Err(MultiplayerError::Cancelled(T::CANCELLED.into()));
        }
        backoff = (backoff * 2).min(MAX_INITIAL_BACKOFF);
    }
}

async fn read_welcome<T: ClientTransport>(
    mut session: ClientSession,
) -> Result<(ClientSession, WelcomeData), MultiplayerError> {
    session.protocol.content_ready()?;
    let message = read_frame_within::<T::Timer>(
        &mut session.recv,
        Some(T::TIMINGS.post_content_welcome),
        "post-content Welcome",
    )
    .await?;
    match session.protocol.receive(message)? {
        HandshakeAction::Welcome(welcome) => Ok((session, welcome)),
        HandshakeAction::PrepareContent(_) => {
            unreachable!("post-content phase only accepts Welcome")
        }
    }
}

async fn next_admission_outbound<T: ClientTransport>(
    transport: &T,
    outbound: &mut T::Outbound,
    timeout: Option<Duration>,
    phase: &'static str,
) -> Result<NetOutbound, MultiplayerError> {
    let received = cancellable::<T::Timer, _>(transport.cancellation(), async {
        match timeout {
            None => Ok(T::recv_outbound(outbound).await),
            Some(timeout) => with_timeout::<T::Timer, _>(timeout, T::recv_outbound(outbound))
                .await
                .map_err(|()| MultiplayerError::timeout(phase, timeout)),
        }
    })
    .await
    .ok_or_else(|| MultiplayerError::Cancelled(format!("{phase} cancelled").into()))??;
    received
        .ok_or_else(|| MultiplayerError::ChannelClosed(format!("{phase} channel closed").into()))
}

/// Complete first-use admission under game/menu control. The transport
/// validates every wire invariant and does not send `ContentReady` itself;
/// that acknowledgement must come from the consumer after durable staging,
/// full-package hash validation, and deterministic mount preparation.
enum ContentAdmissionCompletion {
    Join(ClientSession, WelcomeData),
    Prepared,
}

async fn complete_content_admission<T: ClientTransport>(
    transport: &T,
    mut session: ClientSession,
    offer: &DistributedModOffer,
    incoming: &Sender<NetEvent>,
    outbound: &mut T::Outbound,
) -> Result<ContentAdmissionCompletion, MultiplayerError> {
    let timings = &T::TIMINGS;
    let decision = next_admission_outbound(
        transport,
        outbound,
        timings.content_decision,
        "content decision",
    )
    .await?;
    let decision = ContentDecision::decode(offer, decision)?;
    write_frame_within::<T::Timer>(
        &mut session.send,
        &decision.message(offer),
        timings.content_write,
        decision.operation(),
    )
    .await?;
    let mut received = decision.resume_offset()?;

    let transfer_started = web_time::Instant::now();
    let transfer_exceeded = || MultiplayerError::DeadlineExceeded {
        phase: "content transfer".into(),
        limit: timings.content_transfer,
    };
    while received < offer.encoded_bytes {
        let remaining = timings
            .content_transfer
            .checked_sub(transfer_started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(transfer_exceeded)?;
        let read = read_frame_within::<T::Timer>(
            &mut session.recv,
            Some(timings.content_chunk_idle),
            "content chunk",
        );
        let message = cancellable::<T::Timer, _>(
            transport.cancellation(),
            with_timeout::<T::Timer, _>(remaining, read),
        )
        .await
        .ok_or_else(|| MultiplayerError::Cancelled("content transfer cancelled".into()))?
        .map_err(|()| transfer_exceeded())??;
        let (end, event) = accept_chunk(offer, received, message)?;
        deliver(incoming, event)?;
        received = end;
    }

    let ready = next_admission_outbound(
        transport,
        outbound,
        timings.content_readiness,
        "content readiness",
    )
    .await?;
    match ready {
        NetOutbound::ContentReady { full_mod_sha256 }
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            write_frame_within::<T::Timer>(
                &mut session.send,
                &NetMsg::ContentReady { full_mod_sha256 },
                timings.content_write,
                "content readiness",
            )
            .await?;
            let (session, welcome) = read_welcome::<T>(session).await?;
            Ok(ContentAdmissionCompletion::Join(session, welcome))
        }
        NetOutbound::ContentPrepared { full_mod_sha256 }
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            write_frame_within::<T::Timer>(
                &mut session.send,
                &NetMsg::ContentPrepared { full_mod_sha256 },
                timings.content_write,
                "content prepared acknowledgement",
            )
            .await?;
            Ok(ContentAdmissionCompletion::Prepared)
        }
        NetOutbound::ContentReject(reject) if reject.full_mod_sha256 == offer.full_mod_sha256 => {
            write_frame_within::<T::Timer>(
                &mut session.send,
                &NetMsg::ContentReject(reject.clone()),
                timings.content_write,
                "content rejection",
            )
            .await?;
            Err(MultiplayerError::ContentDeclined(
                format!(
                    "downloaded host content failed local admission: {}",
                    reject.reason
                )
                .into(),
            ))
        }
        other => Err(MultiplayerError::LocalState(
            format!("expected local ContentReady/ContentPrepared/ContentReject, got {other:?}")
                .into(),
        )),
    }
}

/// A reconnect may bypass byte transfer only for the identical offer that
/// this same live client session already admitted and mounted. Any content
/// change or content/no-content downgrade is a hard reconnect failure.
async fn resolve_reconnect_prelude<T: ClientTransport>(
    prelude: HandshakePrelude,
    admitted: Option<&DistributedModOffer>,
) -> Result<(ClientSession, WelcomeData), MultiplayerError> {
    let offered = match &prelude {
        HandshakePrelude::Welcome { .. } => None,
        HandshakePrelude::Content { offer, .. } => Some(offer),
    };
    validate_reconnect_content(offered, admitted)?;
    match prelude {
        HandshakePrelude::Welcome { session, welcome } => Ok((session, welcome)),
        HandshakePrelude::Content { mut session, offer } => {
            write_frame_within::<T::Timer>(
                &mut session.send,
                &NetMsg::ContentRequest(robin_engine::multiplayer::ContentRequest {
                    full_mod_sha256: offer.full_mod_sha256,
                    resume_offset: offer.encoded_bytes,
                }),
                T::TIMINGS.content_write,
                "reconnect content request",
            )
            .await?;
            write_frame_within::<T::Timer>(
                &mut session.send,
                &NetMsg::ContentReady {
                    full_mod_sha256: offer.full_mod_sha256,
                },
                T::TIMINGS.content_write,
                "reconnect content readiness",
            )
            .await?;
            read_welcome::<T>(session).await
        }
    }
}

// ─── Connection lifecycle ────────────────────────────────────────

/// Drive one connection: initial handshake and content admission, then each
/// session until it ends, auto-reconnecting with exponential backoff. Returns
/// when the game loop drops the outgoing queue, shutdown is requested, or a
/// fatal error was published.
pub(super) async fn run_client_io<T: ClientTransport>(
    transport: &T,
    outbound: &mut T::Outbound,
    incoming: Sender<NetEvent>,
    slots: &ClientSlots,
) {
    let prelude = match initial_handshake(transport).await {
        Ok(prelude) => prelude,
        Err(error) => {
            transport.startup_failed(slots, &incoming, StartupFailure::Connect, error);
            return;
        }
    };

    let (mut session, welcome, admitted_offer) = match prelude {
        HandshakePrelude::Welcome { session, welcome } => (session, welcome, None),
        HandshakePrelude::Content { session, offer } => {
            *slot(&slots.content_offer) = Some(offer.clone());
            if deliver(&incoming, NetEvent::ContentOffer(offer.clone())).is_err() {
                return;
            }
            if transport
                .publish_initial_handshake(InitialHandshake::ContentOffered {
                    full_mod_sha256: offer.full_mod_sha256,
                })
                .is_err()
            {
                return;
            }
            match complete_content_admission(transport, session, &offer, &incoming, outbound).await
            {
                Ok(ContentAdmissionCompletion::Join(session, welcome)) => {
                    (session, welcome, Some(offer))
                }
                Ok(ContentAdmissionCompletion::Prepared) => {
                    let _ = incoming.send(NetEvent::Note(format!(
                        "verified and cached host content {} without joining a gameplay seat",
                        robin_engine::spellforge::hex_hash(&offer.full_mod_sha256)
                    )));
                    return;
                }
                Err(error) => {
                    transport.startup_failed(
                        slots,
                        &incoming,
                        StartupFailure::Admission,
                        error.context("distributed-mod admission failed"),
                    );
                    return;
                }
            }
        }
    };
    let admitted_session =
        match ClientSessionMetadata::from_welcome(&welcome, admitted_offer.clone()) {
            Ok(session) => session,
            Err(error) => {
                transport.startup_failed(slots, &incoming, StartupFailure::Welcome, error);
                return;
            }
        };
    if let Err(error) = transport.after_welcome().await {
        transport.startup_failed(slots, &incoming, StartupFailure::Welcome, error);
        return;
    }
    let WelcomeData {
        seat: your_seat,
        mission_id,
        mission_seed,
        sim_config,
        speech_timing_locale,
        session_id,
    } = welcome;

    *slot(&slots.session_metadata) = Some(admitted_session);
    *slot(&slots.content_offer) = admitted_offer.clone();
    if admitted_offer.is_none()
        && transport
            .publish_initial_handshake(InitialHandshake::Welcomed {
                seat: your_seat,
                mission_seed,
            })
            .is_err()
    {
        return;
    }
    if deliver_lifecycle(
        &incoming,
        [
            NetEvent::AssignedLocalSeat(your_seat),
            NetEvent::MissionConfig {
                mission_id: mission_id.clone(),
                rng_seed: mission_seed,
                sim_config,
                speech_timing_locale: speech_timing_locale.clone(),
            },
        ],
    )
    .is_err()
    {
        return;
    }

    let mut backoff = RECONNECT_BACKOFF;
    loop {
        match run_session(transport, session, outbound, &incoming).await {
            SessionEnd::Drop(reason) => {
                let discarded = T::discard_outbound(outbound);
                tracing::warn!("client session ended: {reason}; reconnecting...");
                if discarded != 0 {
                    tracing::warn!(
                        discarded,
                        "multiplayer: discarded outbound commands from abandoned prediction session"
                    );
                }
                if deliver_lifecycle(
                    &incoming,
                    [
                        NetEvent::Note(format!("disconnected: {reason}; reconnecting...")),
                        NetEvent::Disconnected,
                    ],
                )
                .is_err()
                {
                    return;
                }
            }
            SessionEnd::Fatal(error) => {
                let _ = incoming.send(NetEvent::Fatal(NetFatal::new(error)));
                return;
            }
            SessionEnd::OutgoingClosed => return,
        }

        if sleep_or_cancel::<T::Timer>(transport.cancellation(), backoff).await {
            return;
        }
        backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);

        session = loop {
            if transport.cancellation().load(Ordering::Acquire) {
                return;
            }
            let Some(attempt) = attempt_handshake(transport, T::TIMINGS.reconnect_attempt).await
            else {
                return;
            };
            let (failure_kind, error) = match attempt {
                Ok(prelude) => {
                    match resolve_reconnect_prelude::<T>(prelude, admitted_offer.as_ref()).await {
                        Ok((new_session, welcome)) => {
                            let WelcomeData {
                                seat: new_seat,
                                mission_id: new_mission_id,
                                mission_seed: new_seed,
                                sim_config: new_config,
                                speech_timing_locale: new_speech_timing_locale,
                                session_id: new_session_id,
                            } = welcome;
                            if let Err(error) = validate_reconnect_state(
                                ReconnectIdentity {
                                    seat: your_seat,
                                    mission_id: &mission_id,
                                    seed: mission_seed,
                                    config: sim_config,
                                    speech_timing_locale: speech_timing_locale.as_deref(),
                                    session_id,
                                },
                                ReconnectIdentity {
                                    seat: new_seat,
                                    mission_id: &new_mission_id,
                                    seed: new_seed,
                                    config: new_config,
                                    speech_timing_locale: new_speech_timing_locale.as_deref(),
                                    session_id: new_session_id,
                                },
                            ) {
                                let _ = incoming.send(NetEvent::Fatal(NetFatal::new(error)));
                                return;
                            }
                            tracing::info!(?new_seat, seed = new_seed, "client reconnected");
                            // Reconnect validation proved the published identity is unchanged.
                            if deliver_lifecycle(
                                &incoming,
                                [
                                    NetEvent::Reconnected,
                                    NetEvent::AssignedLocalSeat(new_seat),
                                    NetEvent::MissionConfig {
                                        mission_id: new_mission_id,
                                        rng_seed: new_seed,
                                        sim_config: new_config,
                                        speech_timing_locale: new_speech_timing_locale,
                                    },
                                ],
                            )
                            .is_err()
                            {
                                return;
                            }
                            let discarded = T::discard_outbound(outbound);
                            if discarded != 0 {
                                tracing::warn!(
                                    discarded,
                                    "multiplayer: discarded commands queued while transport was reconnecting"
                                );
                            }
                            backoff = RECONNECT_BACKOFF;
                            break new_session;
                        }
                        Err(error) => ("content handshake", error),
                    }
                }
                Err(error) => ("transport", error),
            };
            tracing::warn!("reconnect {failure_kind} failed: {error}; will retry in {backoff:?}");
            if sleep_or_cancel::<T::Timer>(transport.cancellation(), backoff).await {
                return;
            }
            backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
        };
    }
}

/// Run one client session by racing a whole-session reader loop against a
/// whole-session writer loop, so local inputs are sent as soon as the game
/// loop queues them. The reader and writer each own their stream half for the
/// session's lifetime — a select over individual `read_frame` calls would drop
/// partially-read frames when another branch fires first.
async fn run_session<T: ClientTransport>(
    transport: &T,
    session: ClientSession,
    outbound: &mut T::Outbound,
    incoming: &Sender<NetEvent>,
) -> SessionEnd {
    let ClientSession {
        _conn,
        mut send,
        mut recv,
        protocol: _,
    } = session;
    let reader = async {
        loop {
            let message = match reader_outcome(
                read_frame(&mut recv, InboundFramePolicy::ServerToClient).await,
            ) {
                Ok(message) => message,
                Err(end) => return end,
            };
            if let Err(error) = handle_client_wire_msg(incoming, message) {
                return if matches!(error, MultiplayerError::ReconnectRequired { .. }) {
                    SessionEnd::Drop(error)
                } else {
                    SessionEnd::Fatal(error)
                };
            }
        }
    };
    let writer = async {
        loop {
            let Some(outgoing) = T::recv_outbound(outbound).await else {
                return SessionEnd::OutgoingClosed;
            };
            if let Err(error) = send_client_outgoing(&mut send, outgoing).await {
                return SessionEnd::Drop(error);
            }
        }
    };
    let cancel = pin!(wait_for_cancel::<T::Timer>(transport.cancellation()));
    let reader = pin!(reader);
    let writer = pin!(writer);
    match select(cancel, select(reader, writer)).await {
        Either::Left(((), _)) => SessionEnd::OutgoingClosed,
        Either::Right((Either::Left((end, _)) | Either::Right((end, _)), _)) => end,
    }
}

// ─── In-session messages ─────────────────────────────────────────

/// Handle one host message received during a session.
pub(super) fn handle_client_wire_msg(
    incoming: &Sender<NetEvent>,
    message: NetMsg,
) -> Result<(), MultiplayerError> {
    let remote = |message: &'static str| MultiplayerError::RemoteProtocol(message.into());
    let Some(message) = super::client_gameplay::forward(message, incoming)? else {
        return Ok(());
    };
    match message {
        NetMsg::ModalProposal { .. } => Err(remote("server sent a client-only modal proposal")),
        NetMsg::ReconnectRequired { reason } => Err(MultiplayerError::ReconnectRequired { reason }),
        NetMsg::SnapshotTransitionReady { .. } => Err(remote(
            "server sent a client-only snapshot transition acknowledgement",
        )),
        NetMsg::Reject { reason } => Err(MultiplayerError::HostRejected {
            stage: "session",
            reason,
        }),
        other => Err(MultiplayerError::RemoteProtocol(
            format!("host sent invalid session message {other:?}").into(),
        )),
    }
}

/// Turn one session read into the next host message or the end of the
/// session.
///
/// A clean close at a frame boundary is a drop and reconnects, like any other
/// transport loss. The host never half-closes a live session: it ends one with
/// `Reject` (fatal) or `ReconnectRequired`, or closes the whole connection (a
/// transport error), so a bare FIN carries no authoritative decision.
/// Ending here instead would leave the game holding simulation after
/// `Disconnected` with neither a reconnect nor a fatal error.
fn reader_outcome(read: Result<Option<NetMsg>, MultiplayerError>) -> Result<NetMsg, SessionEnd> {
    match read {
        Ok(Some(message)) => Ok(message),
        Ok(None) => Err(SessionEnd::Drop(MultiplayerError::transport(
            "host closed the multiplayer stream",
            std::io::Error::from(std::io::ErrorKind::UnexpectedEof),
        ))),
        Err(error) => Err(SessionEnd::Drop(error)),
    }
}

async fn send_client_outgoing(
    send: &mut SendStream,
    outgoing: NetOutbound,
) -> Result<(), MultiplayerError> {
    // A refused publication stays a typed `MultiplayerError::Protocol` all the
    // way into `SessionEnd`.
    let message = super::client_outgoing::prepare(outgoing)?;
    write_frame(send, &message).await
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Run the shared session handler.
    pub(in crate::multiplayer) fn handle(
        incoming: &Sender<NetEvent>,
        message: NetMsg,
    ) -> Result<(), MultiplayerError> {
        handle_client_wire_msg(incoming, message)
    }

    pub(in crate::multiplayer) fn begin_sim() -> NetMsg {
        NetMsg::BeginSim {
            frame: 9,
            start_epoch_ms: 12,
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn clean_stream_close_is_a_reconnectable_drop() {
        assert!(matches!(
            reader_outcome(Ok(None)),
            Err(SessionEnd::Drop(MultiplayerError::Transport { .. }))
        ));
        assert!(matches!(
            reader_outcome(Err(MultiplayerError::Handshake("reset".into()))),
            Err(SessionEnd::Drop(_))
        ));
        assert!(matches!(
            reader_outcome(Ok(Some(NetMsg::Note("next".into())))),
            Ok(NetMsg::Note(_))
        ));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn begin_sim_is_released_immediately() {
        let (tx, rx) = std::sync::mpsc::channel();
        handle(&tx, begin_sim()).unwrap();
        assert!(matches!(
            rx.try_recv().unwrap(),
            NetEvent::BeginSim {
                frame: 9,
                start_epoch_ms: 12
            }
        ));
        assert!(rx.try_recv().is_err());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn begin_sim_requires_a_live_local_receiver() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(rx);
        assert!(
            handle(&tx, begin_sim())
                .unwrap_err()
                .to_string()
                .contains("channel is closed")
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn wrong_direction_messages_are_client_only_errors() {
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(
            handle(
                &tx,
                NetMsg::SnapshotTransitionReady {
                    id: robin_engine::multiplayer::SnapshotTransitionId {
                        session_id: MultiplayerSessionId([5; 32]),
                        sequence: 1,
                    },
                },
            )
            .unwrap_err()
            .to_string()
            .contains("client-only")
        );
        assert!(
            handle(&tx, NetMsg::Note("late".into())).is_ok(),
            "gameplay notes are forwarded"
        );
        assert!(matches!(rx.try_recv().unwrap(), NetEvent::Note(_)));
        assert!(
            handle(
                &tx,
                NetMsg::ContentOffer {
                    offer: DistributedModOffer {
                        schema_version: 1,
                        full_mod_sha256: [1; 32],
                        spellforge_package_sha256: None,
                        spellforge_vm_abi: None,
                        encoded_bytes: 1,
                        mission_basename: "Mission".into(),
                        mission_rhm_entry: "Data/Levels/Mission.rhm".into(),
                        map_filename: "Mission".into(),
                        title: "Mission".into(),
                        claimed_author: "Author".into(),
                        version: "1".into(),
                        source_url: "https://example.invalid".into(),
                        license: "CC0-1.0".into(),
                        host_endpoint_id: "endpoint-key".into(),
                    },
                },
            )
            .unwrap_err()
            .to_string()
            .contains("invalid session message")
        );
    }
}
