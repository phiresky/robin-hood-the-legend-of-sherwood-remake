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
//!   channel vs handle slots), and a few timing and classification choices.
//! - [`ClientRankedAdmission`]: the ranked trust model. Native signs the
//!   named-seat claim with the install's durable key in-process and receives
//!   its prepared setup through the session writer; the browser waits for the
//!   setup inside the challenge handler and signs through the isolated JS
//!   signer, gating `ReadyToSim` on its admission phase. These are different
//!   admission protocols, not copies, so their per-message decisions stay behind
//!   trait methods (see the TODO on the trait for the remaining divergences).
//!
//! Both sides share one policy for simulation or co-signing released before
//! ranked admission resolved: downgrade to browse-only and keep playing.

use super::client_gameplay::{deliver, deliver_lifecycle};
use super::client_outgoing::ClientPublicationAuthority;
use super::client_protocol::{
    ClientHandshake, ClientSessionMetadata, HandshakeAction, WelcomeData,
    validate_reconnect_content, validate_reconnect_state,
};
use super::content_transfer::{ContentDecision, accept_chunk};
use super::framing::{read_frame, write_frame};
use super::identity::GAME_ALPN;
use super::ranked_client::{ClientRankedJoinState, ranked_lifecycle_lock};
use super::{
    InboundFramePolicy, MultiplayerError, NetEvent, NetMsg, NetOutbound, RankedBrowseOnlyReason,
    RankedJoinChallenge, RankedJoinResponse, RankedJoinUnavailableReason,
    SharedClientLeaderboardCoSignState,
};
use crate::leaderboard_ranked_session::{
    CampaignContinuationReceiptSelectionRequestV1, OfficialRankedSessionSetupV1,
    OfficialRankedSessionWireSetupV1, RankedSessionLifecycle, SharedRankedSessionLifecycle,
    decode_canonical_ranked_wire_document, decode_ranked_wire_document,
};
use futures::future::{Either, select};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr};
use robin_engine::multiplayer::{
    DistributedModOffer, MultiplayerSessionId, NetFatal, RankedCoSignContextDocument,
    RankedJoinAccepted, RankedParticipantRosterDocument, RankedSubmissionAcceptedDocument,
};
use robin_engine::player_command::PlayerId;
use robin_run_protocol::{CampaignContinuationPreflightRequestClaimV1, PublicKey32, Validate as _};
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
    pub(super) ranked_local_public_key: Arc<Mutex<Option<PublicKey32>>>,
    pub(super) startup_error: Arc<Mutex<Option<MultiplayerError>>>,
    pub(super) ranked_lifecycle: SharedRankedSessionLifecycle,
    pub(super) cancellation: Arc<AtomicBool>,
}

impl ClientSlots {
    pub(super) fn new() -> Self {
        Self {
            session_metadata: Arc::new(Mutex::new(None)),
            content_offer: Arc::new(Mutex::new(None)),
            ranked_local_public_key: Arc::new(Mutex::new(None)),
            startup_error: Arc::new(Mutex::new(None)),
            ranked_lifecycle: Arc::new(std::sync::Mutex::new(
                RankedSessionLifecycle::awaiting_prepared_inputs(),
            )),
            cancellation: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn set_startup_error(&self, error: MultiplayerError) {
        *slot(&self.startup_error) = Some(error);
    }

    pub(super) fn set_ranked_local_public_key(&self, key: PublicKey32) {
        *slot(&self.ranked_local_public_key) = Some(key);
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

/// One-shot channel carrying the locally prepared ranked setup (or the
/// explicit `None` decline) from the game to the client worker.
pub(super) fn ranked_setup_channel() -> (
    async_channel::Sender<Option<OfficialRankedSessionSetupV1>>,
    async_channel::Receiver<Option<OfficialRankedSessionSetupV1>>,
) {
    async_channel::bounded(1)
}

/// Handle to an active client connection.
pub struct ClientHandle {
    slots: ClientSlots,
    ranked_setup_tx: async_channel::Sender<Option<OfficialRankedSessionSetupV1>>,
    ranked_setup_sent: AtomicBool,
    ranked_authenticated_host_public_key: PublicKey32,
    worker: Option<ClientWorker>,
}

impl ClientHandle {
    pub(super) fn new(
        slots: ClientSlots,
        ranked_setup_tx: async_channel::Sender<Option<OfficialRankedSessionSetupV1>>,
        ranked_authenticated_host_public_key: PublicKey32,
        worker: Option<ClientWorker>,
    ) -> Self {
        Self {
            slots,
            ranked_setup_tx,
            ranked_setup_sent: AtomicBool::new(false),
            ranked_authenticated_host_public_key,
            worker,
        }
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

    /// Install the exact locally prepared ranked-session authority before the
    /// host's split-phase admission challenge is answered. `None` explicitly
    /// declines ranking without aborting otherwise-compatible gameplay.
    pub(crate) fn install_ranked_session_setup(
        &self,
        setup: Option<OfficialRankedSessionSetupV1>,
    ) -> Result<(), MultiplayerError> {
        const INSTALLED_TWICE: &str = "ranked client setup was installed more than once";
        if let Some(setup) = setup.as_ref() {
            setup.validate().map_err(|error| {
                MultiplayerError::ranked_document("invalid official ranked client setup", error)
            })?;
        }
        // Resolving setup is a one-shot decision, even if its channel send
        // fails. Queue capacity alone cannot enforce this once the first value
        // has been consumed by the worker.
        if self.ranked_setup_sent.swap(true, Ordering::AcqRel) {
            return Err(MultiplayerError::LocalState(INSTALLED_TWICE.into()));
        }
        self.ranked_setup_tx
            .try_send(setup)
            .map_err(|error| match error {
                async_channel::TrySendError::Full(_) => {
                    MultiplayerError::LocalState(INSTALLED_TWICE.into())
                }
                async_channel::TrySendError::Closed(_) => MultiplayerError::ChannelClosed(
                    "ranked client setup transport is no longer running".into(),
                ),
            })
    }

    pub(crate) fn ranked_lifecycle(&self) -> SharedRankedSessionLifecycle {
        Arc::clone(&self.slots.ranked_lifecycle)
    }

    pub(crate) fn ranked_local_seat(&self) -> Result<PlayerId, MultiplayerError> {
        self.session_metadata()
            .map(|session| session.seat)
            .ok_or_else(|| {
                MultiplayerError::Handshake(
                    "ranked client seat is unavailable before handshake".into(),
                )
            })
    }

    pub(crate) fn ranked_local_public_key(&self) -> Option<PublicKey32> {
        *slot(&self.slots.ranked_local_public_key)
    }

    pub(crate) fn ranked_authenticated_host_public_key(&self) -> Option<PublicKey32> {
        Some(self.ranked_authenticated_host_public_key)
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
    /// Server closed the stream cleanly and the platform treats that as the
    /// end of the connection.
    #[cfg_attr(
        target_arch = "wasm32",
        allow(dead_code, reason = "only the native adapter ends on a clean close")
    )]
    Graceful,
    /// Network error / unexpected drop — caller should retry.
    Drop(MultiplayerError),
    /// A direction, session, request, or signature invariant failed. Retrying
    /// the same authenticated session cannot repair this trust violation.
    Fatal(MultiplayerError),
    /// The game loop dropped the outgoing channel or shutdown began — stop
    /// the I/O task entirely (no retry).
    OutgoingClosed,
}

/// What the session writer does next.
pub(super) enum WriterCommand {
    Outbound(NetOutbound),
    RankedResponse(RankedJoinResponse),
    /// Native delivers the prepared ranked setup through the writer.
    #[cfg_attr(
        target_arch = "wasm32",
        allow(
            dead_code,
            reason = "the browser awaits setup in its challenge handler"
        )
    )]
    RankedSetup(Option<OfficialRankedSessionSetupV1>),
    Closed,
    /// A writer-side channel closed while the session is live (native).
    #[cfg_attr(
        target_arch = "wasm32",
        allow(dead_code, reason = "browser writer channels cannot close mid-session")
    )]
    Fatal(MultiplayerError),
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
    type Ranked: ClientRankedAdmission;
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
    /// Wait for the session writer's next command. `pending_ready` is
    /// per-session state the platform may use to hold back `ReadyToSim`.
    async fn next_writer_command(
        &self,
        outbound: &mut Self::Outbound,
        responses: &async_channel::Receiver<RankedJoinResponse>,
        ranked: &Self::Ranked,
        pending_ready: &mut Option<u32>,
    ) -> WriterCommand;

    /// How a clean stream close at a frame boundary ends the session.
    // TODO(10/F1): native ends the connection (and reports Disconnected);
    // the browser treats it as a drop and reconnects. Pick one policy.
    fn stream_closed() -> SessionEnd;
    /// Whether a failed publication of `outgoing` is fatal rather than a
    /// reconnectable drop.
    // TODO(10/F1): the browser also treats ranked-admission and continuation
    // publications as fatal; native only leaderboard co-sign traffic.
    fn fatal_outbound(outgoing: &NetOutbound) -> bool;
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

/// Per-session references the wire handler and ranked admission use.
pub(super) struct SessionLinks<'a, R> {
    pub(super) ranked: &'a R,
    pub(super) incoming: &'a Sender<NetEvent>,
    pub(super) cosign: &'a SharedClientLeaderboardCoSignState,
    pub(super) responses: &'a RankedResponses,
}

/// Writer queue for ranked join responses of one session.
pub(super) struct RankedResponses {
    tx: async_channel::Sender<RankedJoinResponse>,
}

impl RankedResponses {
    pub(super) fn channel(
        capacity: Option<usize>,
    ) -> (Self, async_channel::Receiver<RankedJoinResponse>) {
        let (tx, rx) = match capacity {
            Some(capacity) => async_channel::bounded(capacity),
            None => async_channel::unbounded(),
        };
        (Self { tx }, rx)
    }

    /// Consume the delivered challenge with `response` and queue it for the
    /// writer.
    pub(super) fn queue(
        &self,
        join_state: &ClientRankedJoinState,
        response: RankedJoinResponse,
    ) -> Result<(), MultiplayerError> {
        join_state.authorize_response(&response)?;
        self.tx.try_send(response).map_err(|error| match error {
            async_channel::TrySendError::Full(_) => {
                MultiplayerError::LocalState("ranked admission response queue is occupied".into())
            }
            async_channel::TrySendError::Closed(_) => {
                MultiplayerError::ChannelClosed("ranked response writer queue is closed".into())
            }
        })
    }
}

/// Client-side ranked admission of one connection; lives across sessions and
/// uses interior mutability so the session reader and writer can share it.
// TODO(10/F1): the per-message ranked decisions below still differ between
// the adapters: native downgrades to browse-only on invalid challenge,
// acknowledgement, roster, co-sign context and submission acknowledgement,
// while the browser fails the session on most of them and does not accept
// submission acknowledgements at all. Unifying them is a policy change.
pub(super) trait ClientRankedAdmission: Sized {
    /// Names the platform in "host sent invalid … session message".
    const LABEL: &'static str;
    /// Capacity of the per-session ranked response queue (`None`: unbounded).
    const RESPONSE_QUEUE_CAPACITY: Option<usize>;

    fn lifecycle(&self) -> &SharedRankedSessionLifecycle;
    fn join_state(&self) -> &ClientRankedJoinState;
    fn durable_public_key(&self) -> Option<PublicKey32>;
    fn authenticated_host_public_key(&self) -> PublicKey32;
    /// The authoritative Welcome (initial or reconnect) assigned `seat`.
    fn welcomed(&self, seat: PlayerId);

    /// Whether ranked admission is still unresolved (neither admitted nor
    /// browse-only), so the host may not release simulation yet.
    fn simulation_release_unresolved(&self) -> Result<bool, MultiplayerError>;
    /// Irreversibly record browse-only for this connection's ranking lane.
    fn enter_browse_only(&self, reason: RankedBrowseOnlyReason) -> Result<(), MultiplayerError>;
    /// Downgrade the ranked lifecycle; gameplay continues. `detail` is the
    /// human-readable lifecycle/log text.
    fn downgrade(&self, reason: RankedBrowseOnlyReason, detail: String);
    fn publication_authority(
        &self,
        requires_cosign: bool,
    ) -> Result<ClientPublicationAuthority, MultiplayerError>;

    async fn on_challenge<Tm: ClientTimer>(
        &self,
        links: &SessionLinks<'_, Self>,
        challenge: RankedJoinChallenge,
    ) -> Result<(), MultiplayerError>;
    /// Locally prepared setup delivered through the session writer.
    fn on_setup(&self, responses: &RankedResponses, setup: Option<OfficialRankedSessionSetupV1>);
    fn on_accepted(
        &self,
        links: &SessionLinks<'_, Self>,
        accepted: RankedJoinAccepted,
    ) -> Result<(), MultiplayerError>;
    fn on_roster(
        &self,
        links: &SessionLinks<'_, Self>,
        document: RankedParticipantRosterDocument,
    ) -> Result<(), MultiplayerError>;
    fn on_browse_only(
        &self,
        links: &SessionLinks<'_, Self>,
        reason: RankedBrowseOnlyReason,
    ) -> Result<(), MultiplayerError>;
    fn on_cosign_context(
        &self,
        links: &SessionLinks<'_, Self>,
        context: RankedCoSignContextDocument,
    ) -> Result<(), MultiplayerError>;
    fn on_submission_accepted(
        &self,
        links: &SessionLinks<'_, Self>,
        accepted: RankedSubmissionAcceptedDocument,
    ) -> Result<(), MultiplayerError>;

    /// A session dropped and the client will reconnect. `Err` is fatal.
    fn on_session_dropped(&self) -> Result<(), MultiplayerError>;
    /// A (re)connect attempt failed before a session started. `Err` is fatal.
    fn reset_after_failed_handshake(&self) -> Result<(), MultiplayerError>;
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
    ranked: &T::Ranked,
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
        ranked.reset_after_failed_handshake()?;
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
        NetOutbound::ContentReject {
            full_mod_sha256,
            reason,
        } if full_mod_sha256 == offer.full_mod_sha256 => {
            write_frame_within::<T::Timer>(
                &mut session.send,
                &NetMsg::ContentReject {
                    full_mod_sha256,
                    reason: reason.clone(),
                },
                timings.content_write,
                "content rejection",
            )
            .await?;
            Err(MultiplayerError::ContentDeclined(
                format!("downloaded host content failed local admission: {reason}").into(),
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
                &NetMsg::ContentRequest {
                    full_mod_sha256: offer.full_mod_sha256,
                    resume_offset: offer.encoded_bytes,
                },
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
    ranked: &T::Ranked,
    outbound: &mut T::Outbound,
    incoming: Sender<NetEvent>,
    slots: &ClientSlots,
) {
    let prelude = match initial_handshake(transport, ranked).await {
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

    ranked.welcomed(your_seat);
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

    let cosign: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
    let mut backoff = RECONNECT_BACKOFF;
    loop {
        match run_session(transport, session, ranked, outbound, &incoming, &cosign).await {
            SessionEnd::Graceful => break,
            SessionEnd::Drop(reason) => {
                if let Err(error) = ranked.on_session_dropped() {
                    let _ = incoming.send(NetEvent::Fatal(NetFatal::new(error)));
                    return;
                }
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
                                your_seat,
                                &mission_id,
                                mission_seed,
                                sim_config,
                                speech_timing_locale.as_deref(),
                                session_id,
                                new_seat,
                                &new_mission_id,
                                new_seed,
                                new_config,
                                new_speech_timing_locale.as_deref(),
                                new_session_id,
                            ) {
                                let _ = incoming.send(NetEvent::Fatal(NetFatal::new(error)));
                                return;
                            }
                            tracing::info!(?new_seat, seed = new_seed, "client reconnected");
                            ranked.welcomed(new_seat);
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
            if let Err(reset_error) = ranked.reset_after_failed_handshake() {
                let _ = incoming.send(NetEvent::Fatal(NetFatal::new(reset_error.context(
                    format!("could not reset ranked reconnect after {failure_kind} failure"),
                ))));
                return;
            }
            tracing::warn!("reconnect {failure_kind} failed: {error}; will retry in {backoff:?}");
            if sleep_or_cancel::<T::Timer>(transport.cancellation(), backoff).await {
                return;
            }
            backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
        };
    }

    let _ = incoming.send(NetEvent::Disconnected);
}

/// Run one client session by racing a whole-session reader loop against a
/// whole-session writer loop, so local inputs are sent as soon as the game
/// loop queues them. The reader and writer each own their stream half for the
/// session's lifetime — a select over individual `read_frame` calls would drop
/// partially-read frames when another branch fires first.
async fn run_session<T: ClientTransport>(
    transport: &T,
    session: ClientSession,
    ranked: &T::Ranked,
    outbound: &mut T::Outbound,
    incoming: &Sender<NetEvent>,
    cosign: &SharedClientLeaderboardCoSignState,
) -> SessionEnd {
    let ClientSession {
        _conn,
        mut send,
        mut recv,
        protocol: _,
    } = session;
    let (responses, response_rx) = RankedResponses::channel(T::Ranked::RESPONSE_QUEUE_CAPACITY);
    let links = SessionLinks {
        ranked,
        incoming,
        cosign,
        responses: &responses,
    };
    let reader = async {
        loop {
            match read_frame(&mut recv, InboundFramePolicy::ServerToClient).await {
                Ok(Some(message)) => {
                    if let Err(error) =
                        handle_client_wire_msg::<T::Ranked, T::Timer>(&links, message).await
                    {
                        return if matches!(error, MultiplayerError::ReconnectRequired { .. }) {
                            SessionEnd::Drop(error)
                        } else {
                            SessionEnd::Fatal(error)
                        };
                    }
                }
                Ok(None) => return T::stream_closed(),
                Err(error) => return SessionEnd::Drop(error),
            }
        }
    };
    let writer = async {
        let mut pending_ready = None;
        loop {
            let outgoing = match transport
                .next_writer_command(outbound, &response_rx, ranked, &mut pending_ready)
                .await
            {
                WriterCommand::Outbound(outgoing) => outgoing,
                WriterCommand::RankedResponse(response) => {
                    if let Err(error) =
                        write_frame(&mut send, &NetMsg::RankedJoinResponse(response)).await
                    {
                        return SessionEnd::Drop(error);
                    }
                    continue;
                }
                WriterCommand::RankedSetup(setup) => {
                    ranked.on_setup(links.responses, setup);
                    continue;
                }
                WriterCommand::Closed => return SessionEnd::OutgoingClosed,
                WriterCommand::Fatal(error) => return SessionEnd::Fatal(error),
            };
            let fatal = T::fatal_outbound(&outgoing);
            if let Err(error) = send_client_outgoing(&mut send, outgoing, &links).await {
                return if fatal {
                    SessionEnd::Fatal(error)
                } else {
                    SessionEnd::Drop(error)
                };
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
pub(super) async fn handle_client_wire_msg<R: ClientRankedAdmission, Tm: ClientTimer>(
    links: &SessionLinks<'_, R>,
    message: NetMsg,
) -> Result<(), MultiplayerError> {
    let remote = |message: &'static str| MultiplayerError::RemoteProtocol(message.into());
    let closed = |message: &'static str| MultiplayerError::ChannelClosed(message.into());
    let Some(message) = super::client_gameplay::forward(message, links.incoming)? else {
        return Ok(());
    };
    let ranked = links.ranked;
    match message {
        NetMsg::BeginSim {
            frame,
            start_epoch_ms,
        } => {
            if ranked.simulation_release_unresolved()? {
                downgrade_premature_simulation(links)?;
            }
            deliver(
                links.incoming,
                NetEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                },
            )?;
        }
        NetMsg::ModalProposal { .. } => {
            return Err(remote("server sent a client-only modal proposal"));
        }
        NetMsg::ReconnectRequired { reason } => {
            return Err(MultiplayerError::ReconnectRequired { reason });
        }
        NetMsg::SnapshotTransitionReady { .. } => {
            return Err(remote(
                "server sent a client-only snapshot transition acknowledgement",
            ));
        }
        NetMsg::LeaderboardCoSignRequest(request) => {
            if ranked_lifecycle_lock(ranked.lifecycle())
                .ranked_client()
                .is_none()
            {
                ranked.downgrade(
                    RankedBrowseOnlyReason::RankedProtocolViolation,
                    "host requested leaderboard co-signing before ranked client admission"
                        .to_string(),
                );
                return Ok(());
            }
            if let Some(request) = links.cosign.receive_wire_request(request)? {
                links
                    .incoming
                    .send(NetEvent::LeaderboardCoSignRequest(request))
                    .map_err(|_| closed("client leaderboard co-sign request channel is closed"))?;
            }
        }
        NetMsg::LeaderboardCoSignResponse(_) => {
            return Err(remote(
                "server sent a client-only leaderboard co-sign response",
            ));
        }
        NetMsg::RankedJoinChallenge(challenge) => {
            ranked.on_challenge::<Tm>(links, challenge).await?;
        }
        NetMsg::RankedJoinAccepted(accepted) => ranked.on_accepted(links, accepted)?,
        NetMsg::RankedParticipantRoster(document) => ranked.on_roster(links, document)?,
        NetMsg::RankedBrowseOnly { reason } => ranked.on_browse_only(links, reason)?,
        NetMsg::RankedOfficialSessionSetup(document) => {
            decode_canonical_ranked_wire_document::<OfficialRankedSessionWireSetupV1>(
                document.as_bytes(),
            )
            .map_err(|error| {
                MultiplayerError::ranked_document("invalid official ranked wire setup", error)
            })?;
            links
                .incoming
                .send(NetEvent::RankedOfficialSessionSetup(document))
                .map_err(|_| closed("client official ranked setup channel is closed"))?;
        }
        NetMsg::RankedContinuationReceiptSelectionRequest(document) => {
            let request = decode_ranked_wire_document::<
                CampaignContinuationReceiptSelectionRequestV1,
            >(document.as_bytes())
            .map_err(|error| {
                MultiplayerError::ranked_document(
                    "invalid continuation receipt selection request",
                    error,
                )
            })?;
            let local_public_key = ranked.durable_public_key().ok_or_else(|| {
                MultiplayerError::Identity(
                    "continuation receipt selection has no durable ranked identity".into(),
                )
            })?;
            if request.lobby.host_public_key != ranked.authenticated_host_public_key()
                || request
                    .lobby
                    .participant_public_keys
                    .binary_search(&local_public_key)
                    .is_err()
            {
                return Err(MultiplayerError::Identity(
                    "continuation receipt selection request does not bind the authenticated host and local peer"
                        .into(),
                ));
            }
            links
                .incoming
                .send(NetEvent::RankedContinuationReceiptSelectionRequest(
                    document,
                ))
                .map_err(|_| {
                    closed("client continuation receipt selection request channel is closed")
                })?;
        }
        NetMsg::RankedContinuationReceiptSelection(_) => {
            return Err(remote(
                "server sent a client-only continuation receipt selection",
            ));
        }
        NetMsg::RankedContinuationPreflightClaim(document) => {
            let claim = decode_ranked_wire_document::<CampaignContinuationPreflightRequestClaimV1>(
                document.as_bytes(),
            )
            .map_err(|error| {
                MultiplayerError::ranked_document("invalid continuation preflight claim", error)
            })?;
            let local_public_key = ranked.durable_public_key().ok_or_else(|| {
                MultiplayerError::Identity(
                    "continuation preflight controller has no durable ranked identity".into(),
                )
            })?;
            if claim.host_public_key != ranked.authenticated_host_public_key()
                || claim.campaign_controller_public_key != local_public_key
            {
                return Err(MultiplayerError::Identity(
                    "continuation preflight claim does not bind the authenticated host and local controller"
                        .into(),
                ));
            }
            links
                .incoming
                .send(NetEvent::RankedContinuationPreflightClaim(document))
                .map_err(|_| closed("client continuation preflight claim channel is closed"))?;
        }
        NetMsg::RankedContinuationPreflightSignature(_) => {
            return Err(remote(
                "server sent a client-only continuation preflight signature",
            ));
        }
        NetMsg::RankedCoSignContext(context) => ranked.on_cosign_context(links, context)?,
        NetMsg::RankedSubmissionAccepted(accepted) => {
            ranked.on_submission_accepted(links, accepted)?;
        }
        NetMsg::RankedJoinResponse(_) => {
            return Err(remote("server sent a client-only ranked join response"));
        }
        NetMsg::Reject { reason } => {
            return Err(MultiplayerError::HostRejected {
                stage: "session",
                reason,
            });
        }
        other => {
            return Err(MultiplayerError::RemoteProtocol(
                format!("host sent invalid {} session message {other:?}", R::LABEL).into(),
            ));
        }
    }
    Ok(())
}

/// The one policy for a host that releases simulation before ranked admission
/// resolved: tell the host ranking is unavailable (when a challenge is still
/// open), irreversibly downgrade to browse-only, publish that, and continue.
fn downgrade_premature_simulation<R: ClientRankedAdmission>(
    links: &SessionLinks<'_, R>,
) -> Result<(), MultiplayerError> {
    let reason = RankedBrowseOnlyReason::RankedProtocolViolation;
    if let Err(error) = links.responses.queue(
        links.ranked.join_state(),
        RankedJoinResponse::Unavailable(RankedJoinUnavailableReason::LocalRankedSessionMismatch),
    ) {
        // Browse-only play is permitted even if no ranked challenge was
        // issued, so an unavailable response may not be authorized. The local
        // downgrade below remains required; it cannot be lost along with this
        // response.
        tracing::warn!(%error, "could not notify host of premature ranked BeginSim");
    }
    links.ranked.enter_browse_only(reason)?;
    links.ranked.downgrade(
        reason,
        "host released simulation before ranked admission or browse-only resolution".to_string(),
    );
    deliver(links.incoming, NetEvent::RankedBrowseOnly { reason })
}

async fn send_client_outgoing<R: ClientRankedAdmission>(
    send: &mut SendStream,
    outgoing: NetOutbound,
    links: &SessionLinks<'_, R>,
) -> Result<(), MultiplayerError> {
    let requires_cosign = matches!(
        &outgoing,
        NetOutbound::ArmLeaderboardCoSignRequest { .. } | NetOutbound::LeaderboardCoSignResponse(_)
    );
    let authority = links.ranked.publication_authority(requires_cosign)?;
    // A refused publication stays a typed `MultiplayerError::Protocol` all the
    // way into `SessionEnd` and the `NetEvent::Fatal` payload.
    if let Some(message) =
        super::client_outgoing::prepare(outgoing, links.incoming, links.cosign, authority)?
    {
        write_frame(send, &message).await?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use robin_engine::multiplayer::LeaderboardCoSignResponse;
    use robin_run_protocol::{
        Digest32, LeaderboardCoSignInstanceV1, LeaderboardCoSignPurposeV1,
        LeaderboardCoSignRequestV1,
    };
    use std::cell::{Cell, RefCell};

    /// Timer whose deadlines elapse immediately; the handler paths under test
    /// never wait.
    pub(in crate::multiplayer) struct ImmediateTimer;

    impl ClientTimer for ImmediateTimer {
        fn sleep(_duration: Duration) -> impl Future<Output = ()> {
            std::future::ready(())
        }
    }

    /// Minimal ranked admission recording which shared-policy seams ran.
    struct MockRanked {
        lifecycle: SharedRankedSessionLifecycle,
        join: ClientRankedJoinState,
        resolved: Cell<bool>,
        calls: RefCell<Vec<String>>,
    }

    impl MockRanked {
        fn unresolved() -> Self {
            Self {
                lifecycle: Arc::new(std::sync::Mutex::new(
                    RankedSessionLifecycle::awaiting_prepared_inputs(),
                )),
                join: ClientRankedJoinState::default(),
                resolved: Cell::new(false),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl ClientRankedAdmission for MockRanked {
        const LABEL: &'static str = "mock";
        const RESPONSE_QUEUE_CAPACITY: Option<usize> = Some(1);

        fn lifecycle(&self) -> &SharedRankedSessionLifecycle {
            &self.lifecycle
        }
        fn join_state(&self) -> &ClientRankedJoinState {
            &self.join
        }
        fn durable_public_key(&self) -> Option<PublicKey32> {
            None
        }
        fn authenticated_host_public_key(&self) -> PublicKey32 {
            PublicKey32::from_bytes([1; 32])
        }
        fn welcomed(&self, seat: PlayerId) {
            self.calls.borrow_mut().push(format!("welcomed {seat:?}"));
        }
        fn simulation_release_unresolved(&self) -> Result<bool, MultiplayerError> {
            Ok(!self.resolved.get())
        }
        fn enter_browse_only(
            &self,
            reason: RankedBrowseOnlyReason,
        ) -> Result<(), MultiplayerError> {
            self.join.mark_browse_only(reason)?;
            self.resolved.set(true);
            self.calls
                .borrow_mut()
                .push(format!("browse-only {reason:?}"));
            Ok(())
        }
        fn downgrade(&self, reason: RankedBrowseOnlyReason, detail: String) {
            ranked_lifecycle_lock(&self.lifecycle).downgrade(detail.clone());
            self.calls
                .borrow_mut()
                .push(format!("downgrade {reason:?}: {detail}"));
        }
        fn publication_authority(
            &self,
            _requires_cosign: bool,
        ) -> Result<ClientPublicationAuthority, MultiplayerError> {
            Ok(ClientPublicationAuthority {
                co_sign_allowed: false,
                durable_public_key: None,
            })
        }
        async fn on_challenge<Tm: ClientTimer>(
            &self,
            _links: &SessionLinks<'_, Self>,
            _challenge: RankedJoinChallenge,
        ) -> Result<(), MultiplayerError> {
            Err(MultiplayerError::LocalState(
                "mock admission has no challenge handler".into(),
            ))
        }
        fn on_setup(
            &self,
            _responses: &RankedResponses,
            _setup: Option<OfficialRankedSessionSetupV1>,
        ) {
            unreachable!("mock admission receives no setup")
        }
        fn on_accepted(
            &self,
            _links: &SessionLinks<'_, Self>,
            _accepted: RankedJoinAccepted,
        ) -> Result<(), MultiplayerError> {
            Err(MultiplayerError::LocalState(
                "mock admission has no acceptance handler".into(),
            ))
        }
        fn on_roster(
            &self,
            _links: &SessionLinks<'_, Self>,
            _document: RankedParticipantRosterDocument,
        ) -> Result<(), MultiplayerError> {
            Err(MultiplayerError::LocalState(
                "mock admission has no roster handler".into(),
            ))
        }
        fn on_browse_only(
            &self,
            _links: &SessionLinks<'_, Self>,
            _reason: RankedBrowseOnlyReason,
        ) -> Result<(), MultiplayerError> {
            Err(MultiplayerError::LocalState(
                "mock admission has no browse-only handler".into(),
            ))
        }
        fn on_cosign_context(
            &self,
            _links: &SessionLinks<'_, Self>,
            _context: RankedCoSignContextDocument,
        ) -> Result<(), MultiplayerError> {
            Err(MultiplayerError::LocalState(
                "mock admission has no co-sign context handler".into(),
            ))
        }
        fn on_submission_accepted(
            &self,
            _links: &SessionLinks<'_, Self>,
            _accepted: RankedSubmissionAcceptedDocument,
        ) -> Result<(), MultiplayerError> {
            Err(MultiplayerError::LocalState(
                "mock admission has no submission handler".into(),
            ))
        }
        fn on_session_dropped(&self) -> Result<(), MultiplayerError> {
            Ok(())
        }
        fn reset_after_failed_handshake(&self) -> Result<(), MultiplayerError> {
            Ok(())
        }
    }

    pub(in crate::multiplayer) fn leaderboard_request() -> LeaderboardCoSignRequestV1 {
        LeaderboardCoSignRequestV1 {
            instance: LeaderboardCoSignInstanceV1 {
                purpose: LeaderboardCoSignPurposeV1::Submission,
                replay_session_id: Digest32::from_bytes([41; 32]),
                submission_offer_sha256: Digest32::from_bytes([42; 32]),
            },
            run_digest: Digest32::from_bytes([43; 32]),
        }
    }

    /// Run the shared handler for `ranked` with a fresh session queue.
    pub(in crate::multiplayer) fn handle<R: ClientRankedAdmission>(
        ranked: &R,
        incoming: &Sender<NetEvent>,
        cosign: &SharedClientLeaderboardCoSignState,
        message: NetMsg,
    ) -> Result<(), MultiplayerError> {
        let (responses, _response_rx) = RankedResponses::channel(R::RESPONSE_QUEUE_CAPACITY);
        let links = SessionLinks {
            ranked,
            incoming,
            cosign,
            responses: &responses,
        };
        futures::executor::block_on(handle_client_wire_msg::<R, ImmediateTimer>(&links, message))
    }

    pub(in crate::multiplayer) fn begin_sim() -> NetMsg {
        NetMsg::BeginSim {
            frame: 9,
            start_epoch_ms: 12,
        }
    }

    /// Assert the one premature-BeginSim policy on any adapter: browse-only is
    /// published first, then simulation is still released.
    pub(in crate::multiplayer) fn assert_premature_begin_sim_downgrades<
        R: ClientRankedAdmission,
    >(
        ranked: &R,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let cosign = Arc::new(Default::default());
        handle(ranked, &tx, &cosign, begin_sim()).expect("premature BeginSim is not fatal");
        assert!(matches!(
            rx.try_recv().unwrap(),
            NetEvent::RankedBrowseOnly {
                reason: RankedBrowseOnlyReason::RankedProtocolViolation
            }
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            NetEvent::BeginSim {
                frame: 9,
                start_epoch_ms: 12
            }
        ));
        assert!(rx.try_recv().is_err());
        assert!(
            ranked_lifecycle_lock(ranked.lifecycle())
                .browse_only_reason()
                .is_some(),
            "the ranked lifecycle must be downgraded"
        );
        assert!(!ranked.simulation_release_unresolved().unwrap());
        assert!(
            ranked.join_state().begin_reconnect().is_err(),
            "browse-only is irreversible"
        );

        // Once resolved, a later BeginSim is released without a new downgrade.
        handle(ranked, &tx, &cosign, begin_sim()).unwrap();
        assert!(matches!(rx.try_recv().unwrap(), NetEvent::BeginSim { .. }));
        assert!(rx.try_recv().is_err());
    }

    /// Assert the one premature co-sign policy on any adapter: the request is
    /// neither staged nor exposed, the lifecycle is downgraded, and the session
    /// continues.
    pub(in crate::multiplayer) fn assert_premature_cosign_request_downgrades<
        R: ClientRankedAdmission,
    >(
        ranked: &R,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let cosign: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
        let request = leaderboard_request();
        handle(
            ranked,
            &tx,
            &cosign,
            NetMsg::LeaderboardCoSignRequest(request),
        )
        .expect("premature co-sign request is not fatal");
        assert!(rx.try_recv().is_err());
        assert!(
            ranked_lifecycle_lock(ranked.lifecycle())
                .browse_only_reason()
                .is_some()
        );
        assert_eq!(
            cosign.arm_request(request).unwrap(),
            None,
            "the unadmitted host request must not have been staged"
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn mock_premature_begin_sim_downgrades_to_browse_only() {
        let ranked = MockRanked::unresolved();
        assert_premature_begin_sim_downgrades(&ranked);
        assert_eq!(
            ranked.calls.borrow().as_slice(),
            [
                "browse-only RankedProtocolViolation",
                "downgrade RankedProtocolViolation: host released simulation before ranked admission or browse-only resolution",
            ]
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn mock_premature_cosign_request_downgrades_to_browse_only() {
        let ranked = MockRanked::unresolved();
        assert_premature_cosign_request_downgrades(&ranked);
        assert_eq!(ranked.calls.borrow().len(), 1);
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn begin_sim_requires_a_live_local_receiver() {
        let ranked = MockRanked::unresolved();
        ranked.resolved.set(true);
        let (tx, rx) = std::sync::mpsc::channel();
        drop(rx);
        let cosign = Arc::new(Default::default());
        assert!(
            handle(&ranked, &tx, &cosign, begin_sim())
                .unwrap_err()
                .to_string()
                .contains("channel is closed")
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn wrong_direction_messages_are_client_only_errors() {
        let ranked = MockRanked::unresolved();
        let (tx, rx) = std::sync::mpsc::channel();
        let cosign = Arc::new(Default::default());
        let request = leaderboard_request();
        for message in [
            NetMsg::LeaderboardCoSignResponse(LeaderboardCoSignResponse {
                instance: request.instance,
                signer_public_key: [7; 32],
                signature: [8; 64],
            }),
            NetMsg::SnapshotTransitionReady {
                id: robin_engine::multiplayer::SnapshotTransitionId {
                    session_id: MultiplayerSessionId([5; 32]),
                    sequence: 1,
                },
            },
        ] {
            assert!(
                handle(&ranked, &tx, &cosign, message)
                    .unwrap_err()
                    .to_string()
                    .contains("client-only")
            );
        }
        assert!(
            handle(&ranked, &tx, &cosign, NetMsg::Note("late".into())).is_ok(),
            "gameplay notes are forwarded"
        );
        assert!(matches!(rx.try_recv().unwrap(), NetEvent::Note(_)));
        assert!(
            handle(
                &ranked,
                &tx,
                &cosign,
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
            .contains("invalid mock session message")
        );
    }

    fn setup_handle(
        ranked_setup_tx: async_channel::Sender<Option<OfficialRankedSessionSetupV1>>,
    ) -> ClientHandle {
        ClientHandle::new(
            ClientSlots::new(),
            ranked_setup_tx,
            PublicKey32::from_bytes([1; 32]),
            None,
        )
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn ranked_setup_is_one_shot_after_the_first_value_is_consumed() {
        let (tx, rx) = ranked_setup_channel();
        let handle = setup_handle(tx);
        handle.install_ranked_session_setup(None).unwrap();
        assert!(rx.try_recv().unwrap().is_none());
        assert!(
            handle
                .install_ranked_session_setup(None)
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );
        assert!(rx.try_recv().is_err());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn failed_ranked_setup_send_is_not_retryable() {
        let (tx, rx) = ranked_setup_channel();
        let handle = setup_handle(tx);
        drop(rx);
        assert!(
            handle
                .install_ranked_session_setup(None)
                .unwrap_err()
                .to_string()
                .contains("no longer running")
        );
        assert!(
            handle
                .install_ranked_session_setup(None)
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );
    }
}
