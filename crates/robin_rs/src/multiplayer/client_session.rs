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
//! - [`ClientRankedAdmission`]: the ranked trust model. Native signs the
//!   named-seat claim with the install's durable key in-process and receives
//!   its prepared setup through the session writer; the browser waits for the
//!   setup inside the challenge handler and signs through the isolated JS
//!   signer, gating `ReadyToSim` until ranking resolves. Only the challenge,
//!   setup and acknowledgement steps of these two admission protocols differ.
//!
//! Every ranked-session rule is decided here, once, for both platforms:
//!
//! - An invalid or out-of-phase ranked message (or simulation / co-signing
//!   released before admission resolved) irreversibly downgrades the lane to
//!   browse-only, publishes `RankedBrowseOnly`, and gameplay continues. Only a
//!   closed local event channel ends the session ([`contain_ranked_violation`]).
//! - A clean stream close is a transport drop and reconnects ([`reader_outcome`]).
//! - A dropped session resets ranked admission once, identically, for the
//!   authenticated reconnect ([`reset_ranked_admission_for_reconnect`]).
//! - Ranked submission acknowledgements are validated and published to the
//!   mission-end co-signer only for an admitted ranked client.
//! - A failed publication of one-shot ranked authority is fatal
//!   ([`publication_failure_is_fatal`]).

use super::client_gameplay::{deliver, deliver_lifecycle};
use super::client_outgoing::ClientPublicationAuthority;
use super::client_protocol::{
    ClientHandshake, ClientSessionMetadata, HandshakeAction, ReconnectIdentity, WelcomeData,
    validate_reconnect_content, validate_reconnect_state,
};
use super::content_transfer::{ContentDecision, accept_chunk};
use super::framing::{read_frame, write_frame};
use super::identity::GAME_ALPN;
use super::ranked_client::{
    ClientRankedJoinState, decode_ranked_participant_roster, ranked_lifecycle_lock,
};
use super::{
    InboundFramePolicy, MultiplayerError, NetEvent, NetMsg, NetOutbound, RankedBrowseOnlyReason,
    RankedJoinChallenge, RankedJoinResponse, RankedJoinUnavailableReason,
    SharedClientLeaderboardCoSignState,
};
use crate::leaderboard_ranked_session::{
    CampaignContinuationReceiptSelectionRequestV1, OfficialRankedSessionSetupV1,
    OfficialRankedSessionWireSetupV1, RankedCoSignContextV1, RankedSessionLifecycle,
    SharedRankedSessionLifecycle, decode_canonical_ranked_wire_document,
    decode_ranked_wire_document,
};
use futures::future::{Either, select};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr};
use robin_engine::multiplayer::{
    DistributedModOffer, MultiplayerSessionId, NetFatal, RankedCoSignContextDocument,
    RankedJoinAccepted, RankedParticipantRosterDocument, RankedSubmissionAcceptedDocument,
};
use robin_engine::player_command::PlayerId;
use robin_run_protocol::{
    CampaignContinuationPreflightRequestClaimV1, LeaderboardCoSignRequestV1, PublicKey32,
    SubmissionAcceptedV1, Validate as _,
};
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
    /// Network error, unexpected drop or clean stream close — caller retries.
    /// A clean close is not an authoritative end of the session: the host ends
    /// sessions with `Reject` or `ReconnectRequired`, so a bare FIN is treated
    /// like any other transport loss (see [`reader_outcome`]).
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
    /// A writer-side channel closed (native) or the shared ranked join state
    /// became unreadable (browser `ReadyToSim` gate) while the session is live.
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
///
/// Only the steps where the two trust models genuinely differ live here:
/// answering a challenge, receiving the locally prepared setup, and recording
/// the host acknowledgement. They return `Err` for any ranked failure and never
/// decide what that failure does; the shared handler applies the one policy
/// ([`contain_ranked_violation`]). Rosters, browse-only notices, co-sign
/// contexts, submission acknowledgements and reconnect resets are shared code.
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
    fn publication_authority(
        &self,
        requires_cosign: bool,
    ) -> Result<ClientPublicationAuthority, MultiplayerError>;

    /// Answer a host challenge (attestation or typed unavailability).
    async fn on_challenge<Tm: ClientTimer>(
        &self,
        links: &SessionLinks<'_, Self>,
        challenge: RankedJoinChallenge,
    ) -> Result<(), MultiplayerError>;
    /// Locally prepared setup delivered through the session writer.
    fn on_setup(&self, responses: &RankedResponses, setup: Option<OfficialRankedSessionSetupV1>);
    /// Validate the host acknowledgement, admit the ranked client lifecycle and
    /// publish `RankedJoinAccepted`.
    fn on_accepted(
        &self,
        links: &SessionLinks<'_, Self>,
        accepted: RankedJoinAccepted,
    ) -> Result<(), MultiplayerError>;
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

/// Ranked admission state is untouched by a failed attempt: no session ran, so
/// there is nothing to reset (the one reset happens when a session drops).
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
    ranked: &T::Ranked,
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
            SessionEnd::Drop(reason) => {
                if let Err(error) = reset_ranked_admission_for_reconnect(ranked, &incoming) {
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
            let message = match reader_outcome(
                read_frame(&mut recv, InboundFramePolicy::ServerToClient).await,
            ) {
                Ok(message) => message,
                Err(end) => return end,
            };
            if let Err(error) = handle_client_wire_msg::<T::Ranked, T::Timer>(&links, message).await
            {
                return if matches!(error, MultiplayerError::ReconnectRequired { .. }) {
                    SessionEnd::Drop(error)
                } else {
                    SessionEnd::Fatal(error)
                };
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
            let fatal = publication_failure_is_fatal(&outgoing);
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
        NetMsg::LeaderboardCoSignRequest(request) => contain_ranked_violation(
            links,
            RankedBrowseOnlyReason::RankedProtocolViolation,
            receive_cosign_request(links, request),
        )?,
        NetMsg::LeaderboardCoSignResponse(_) => {
            return Err(remote(
                "server sent a client-only leaderboard co-sign response",
            ));
        }
        NetMsg::RankedJoinChallenge(challenge) => {
            let answered = ranked.on_challenge::<Tm>(links, challenge).await;
            contain_ranked_violation(
                links,
                RankedBrowseOnlyReason::PeerRankedSessionMismatch,
                answered,
            )?;
        }
        NetMsg::RankedJoinAccepted(accepted) => contain_ranked_violation(
            links,
            RankedBrowseOnlyReason::PeerAttestationRejected,
            ranked.on_accepted(links, accepted),
        )?,
        NetMsg::RankedParticipantRoster(document) => contain_ranked_violation(
            links,
            RankedBrowseOnlyReason::PeerAttestationRejected,
            receive_roster_update(links, document),
        )?,
        NetMsg::RankedBrowseOnly { reason } => enter_browse_only(
            ranked,
            links.incoming,
            reason,
            format!("host downgraded ranked multiplayer: {reason:?}"),
        )?,
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
        NetMsg::RankedCoSignContext(context) => contain_ranked_violation(
            links,
            RankedBrowseOnlyReason::RankedProtocolViolation,
            receive_cosign_context(links, context),
        )?,
        NetMsg::RankedSubmissionAccepted(accepted) => contain_ranked_violation(
            links,
            RankedBrowseOnlyReason::RankedProtocolViolation,
            receive_submission_accepted(links, accepted),
        )?,
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

// ─── Ranked session policy ───────────────────────────────────────

/// Irreversibly resolve this connection's ranking lane to browse-only,
/// downgrade the ranked lifecycle, and publish `RankedBrowseOnly` the first
/// time only. Gameplay continues. Later notices (a local downgrade followed by
/// the host's own, or the host replaying its notice on reconnect) keep the
/// first reason and publish nothing further.
fn enter_browse_only<R: ClientRankedAdmission>(
    ranked: &R,
    incoming: &Sender<NetEvent>,
    reason: RankedBrowseOnlyReason,
    detail: String,
) -> Result<(), MultiplayerError> {
    let newly = ranked.join_state().mark_browse_only(reason)?;
    ranked_lifecycle_lock(ranked.lifecycle()).downgrade(detail.clone());
    if !newly {
        return Ok(());
    }
    tracing::warn!(?reason, %detail, "ranked multiplayer downgraded to browse-only; gameplay remains available");
    deliver(incoming, NetEvent::RankedBrowseOnly { reason })
}

/// The one policy for an invalid or out-of-phase ranked message (10/F1).
///
/// Ranking is an optional lane on top of gameplay. A host that sends a ranked
/// document the client cannot verify, or sends it in the wrong phase, can at
/// most make this run unranked: the lane becomes browse-only irreversibly, so
/// the client never co-signs and the run can never be submitted as ranked,
/// and `RankedBrowseOnly` is published so the downgrade is visible, not silent.
/// Ending the session instead would discard a playable game without protecting
/// ranking any further. Only a closed local event channel (the game loop is
/// gone) stays fatal.
///
/// While a challenge is still open the host is told ranking is unavailable, so
/// its admission does not wait for a response that will never come.
// TODO(ranked-client-downgrade-notice): after admission the wire has no
// client-to-host browse-only notice, so the host only learns of a local
// downgrade when its co-sign request goes unanswered.
fn contain_ranked_violation<R: ClientRankedAdmission>(
    links: &SessionLinks<'_, R>,
    reason: RankedBrowseOnlyReason,
    result: Result<(), MultiplayerError>,
) -> Result<(), MultiplayerError> {
    let error = match result {
        Ok(()) => return Ok(()),
        Err(error @ MultiplayerError::ChannelClosed(_)) => return Err(error),
        Err(error) => error,
    };
    if let Err(queue_error) = links.responses.queue(
        links.ranked.join_state(),
        RankedJoinResponse::Unavailable(RankedJoinUnavailableReason::LocalRankedSessionMismatch),
    ) {
        // Usually no challenge is open (e.g. after admission), so there is no
        // ranked response to send; the local downgrade below is still required.
        tracing::debug!(%queue_error, "no open ranked challenge to answer after a ranked violation");
    }
    enter_browse_only(links.ranked, links.incoming, reason, error.to_string())
}

/// A host that releases simulation before ranked admission resolved follows
/// the same policy as any other ranked violation.
fn downgrade_premature_simulation<R: ClientRankedAdmission>(
    links: &SessionLinks<'_, R>,
) -> Result<(), MultiplayerError> {
    contain_ranked_violation(
        links,
        RankedBrowseOnlyReason::RankedProtocolViolation,
        Err(MultiplayerError::Ranked(
            "host released simulation before ranked admission or browse-only resolution".into(),
        )),
    )
}

fn require_ranked_client<R: ClientRankedAdmission>(
    ranked: &R,
    violation: &'static str,
) -> Result<(), MultiplayerError> {
    if ranked_lifecycle_lock(ranked.lifecycle())
        .ranked_client()
        .is_none()
    {
        return Err(MultiplayerError::Ranked(violation.into()));
    }
    Ok(())
}

/// Stage a host co-sign request; it is exposed only once it equals the locally
/// armed request, and only for an admitted ranked client.
fn receive_cosign_request<R: ClientRankedAdmission>(
    links: &SessionLinks<'_, R>,
    request: LeaderboardCoSignRequestV1,
) -> Result<(), MultiplayerError> {
    require_ranked_client(
        links.ranked,
        "host requested leaderboard co-signing before ranked client admission",
    )?;
    if let Some(request) = links.cosign.receive_wire_request(request)? {
        deliver(links.incoming, NetEvent::LeaderboardCoSignRequest(request))?;
    }
    Ok(())
}

/// Install a complete, monotonic roster broadcast for the admitted client.
fn receive_roster_update<R: ClientRankedAdmission>(
    links: &SessionLinks<'_, R>,
    document: RankedParticipantRosterDocument,
) -> Result<(), MultiplayerError> {
    let document = links
        .ranked
        .join_state()
        .receive_wire_roster_update(document)?;
    {
        let mut lifecycle = ranked_lifecycle_lock(links.ranked.lifecycle());
        let genesis = lifecycle
            .ranked_client()
            .map(|client| client.session_genesis.clone())
            .ok_or_else(|| {
                MultiplayerError::Ranked(
                    "ranked roster update arrived before client lifecycle acceptance".into(),
                )
            })?;
        let participant_claims = decode_ranked_participant_roster(&document, &genesis)?;
        lifecycle
            .update_ranked_client_roster(&genesis, participant_claims)
            .map_err(|error| {
                MultiplayerError::ranked_document("update ranked client roster", error)
            })?;
    }
    deliver(links.incoming, NetEvent::RankedParticipantRoster(document))
}

/// Publish a decodable co-sign context to the mission-end co-signer of an
/// admitted ranked client.
fn receive_cosign_context<R: ClientRankedAdmission>(
    links: &SessionLinks<'_, R>,
    context: RankedCoSignContextDocument,
) -> Result<(), MultiplayerError> {
    decode_ranked_wire_document::<RankedCoSignContextV1>(context.as_bytes()).map_err(|error| {
        MultiplayerError::ranked_document("invalid ranked co-sign context", error)
    })?;
    require_ranked_client(
        links.ranked,
        "host published a ranked co-sign context before ranked client admission",
    )?;
    deliver(links.incoming, NetEvent::RankedCoSignContext(context))
}

/// Ranked submission acknowledgements (10/F1): validated and published to the
/// mission-end co-signer of an admitted ranked client on both platforms. The
/// host sends one to whichever admitted participant controls the submission,
/// and the browser's mission-end co-signer consumes it exactly like native, so
/// rejecting it ended a ranked browser session at the moment its run was
/// accepted.
fn receive_submission_accepted<R: ClientRankedAdmission>(
    links: &SessionLinks<'_, R>,
    accepted: RankedSubmissionAcceptedDocument,
) -> Result<(), MultiplayerError> {
    decode_ranked_wire_document::<SubmissionAcceptedV1>(accepted.as_bytes()).map_err(|error| {
        MultiplayerError::ranked_document("invalid ranked submission acknowledgement", error)
    })?;
    require_ranked_client(
        links.ranked,
        "host published a ranked submission acknowledgement before ranked client admission",
    )?;
    deliver(links.incoming, NetEvent::RankedSubmissionAccepted(accepted))
}

/// The one ranked reset when a session drops and the client reconnects (10/F1).
///
/// A lane that already gave up ranking (typed `Unavailable` answer or
/// browse-only) stays that way: downgrades are irreversible and the host
/// replays its browse-only notice on reconnect. Every other lane starts a
/// fresh authenticated admission cycle: the locally prepared setup stays
/// armed, an accepted challenge is retired against replay, and the lifecycle
/// keeps its admitted ranked client, so a ranked run survives transient network
/// loss once the host re-attests the replacement transport. Until then the
/// browser holds `ReadyToSim` again.
///
/// If the reset itself fails the lane cannot be re-admitted safely; it is
/// downgraded to browse-only and published, the same policy as an invalid
/// ranked message, instead of ending a playable session. Only a lost local
/// event channel or an unreadable join state is returned as an error.
pub(super) fn reset_ranked_admission_for_reconnect<R: ClientRankedAdmission>(
    ranked: &R,
    incoming: &Sender<NetEvent>,
) -> Result<(), MultiplayerError> {
    let join = ranked.join_state();
    let reset = match join.irreversibly_unranked() {
        Ok(true) => return Ok(()),
        Ok(false) => join.begin_reconnect(),
        Err(error) => Err(error),
    };
    let Err(error) = reset else {
        return Ok(());
    };
    enter_browse_only(
        ranked,
        incoming,
        RankedBrowseOnlyReason::RankedTransportInterrupted,
        format!("ranked reconnect trust state could not advance: {error}"),
    )
}

/// Turn one session read into the next host message or the end of the
/// session (10/F1).
///
/// A clean close at a frame boundary is a drop and reconnects, like any other
/// transport loss. The host never half-closes a live session: it ends one with
/// `Reject` (fatal) or `ReconnectRequired`, or closes the whole connection (a
/// transport error), so a bare FIN carries no authoritative decision.
/// Reconnecting keeps a ranked run alive through the authenticated reconnect;
/// ending here instead left the game holding simulation after `Disconnected`
/// with neither a reconnect nor a fatal error.
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

/// Whether a failed publication of `outgoing` ends the session instead of
/// reconnecting.
///
/// One-shot ranked authority is fatal: admission responses, campaign
/// continuation replies and every leaderboard co-sign step. The reconnect
/// discards the outbound queue and the co-sign gate has already consumed its
/// request, so nothing replays them; reconnecting would leave the ranked run
/// silently waiting for a signature or reply that was lost. Gameplay traffic
/// reconnects.
fn publication_failure_is_fatal(outgoing: &NetOutbound) -> bool {
    matches!(
        outgoing,
        NetOutbound::ArmRankedJoin { .. }
            | NetOutbound::RankedJoinResponse(_)
            | NetOutbound::RankedContinuationReceiptSelection(_)
            | NetOutbound::RankedContinuationPreflightSignature(_)
            | NetOutbound::LeaderboardCoSignRequest { .. }
            | NetOutbound::ArmLeaderboardCoSignRequest { .. }
            | NetOutbound::LeaderboardCoSignResponse(_)
    )
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

    /// Timer whose deadlines elapse immediately; the handler paths under test
    /// never wait.
    pub(in crate::multiplayer) struct ImmediateTimer;

    impl ClientTimer for ImmediateTimer {
        fn sleep(_duration: Duration) -> impl Future<Output = ()> {
            std::future::ready(())
        }
    }

    /// Minimal ranked admission: every trust-model step fails, so the shared
    /// policy is exercised on its own.
    struct MockRanked {
        lifecycle: SharedRankedSessionLifecycle,
        join: ClientRankedJoinState,
    }

    impl MockRanked {
        fn unresolved() -> Self {
            Self {
                lifecycle: Arc::new(std::sync::Mutex::new(
                    RankedSessionLifecycle::awaiting_prepared_inputs(),
                )),
                join: ClientRankedJoinState::default(),
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
        fn welcomed(&self, _seat: PlayerId) {}
        fn simulation_release_unresolved(&self) -> Result<bool, MultiplayerError> {
            Ok(!self.join.admission_resolved()?)
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
    /// neither staged nor exposed, the lane is downgraded and the downgrade is
    /// published, and the session continues.
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
        assert!(matches!(
            rx.try_recv().unwrap(),
            NetEvent::RankedBrowseOnly {
                reason: RankedBrowseOnlyReason::RankedProtocolViolation
            }
        ));
        assert!(
            rx.try_recv().is_err(),
            "the request itself is never exposed"
        );
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

    /// Bytes that fit a ranked wire document but decode as no ranked document.
    fn not_a_ranked_document() -> Vec<u8> {
        br#"{"not":"a ranked document"}"#.to_vec()
    }

    /// Every invalid or out-of-phase ranked message a host can send an
    /// unadmitted client, with the browse-only reason it must produce.
    pub(in crate::multiplayer) fn invalid_ranked_messages() -> Vec<(NetMsg, RankedBrowseOnlyReason)>
    {
        use robin_engine::multiplayer::{
            RankedJoinAttestationDocument, RankedJoinClaimDocument, RankedSessionGenesisDocument,
        };
        let genesis = || RankedSessionGenesisDocument::new(not_a_ranked_document()).unwrap();
        let well_formed_acknowledgement = RankedSubmissionAcceptedDocument::new(
            crate::leaderboard_ranked_session::encode_ranked_wire_document(&SubmissionAcceptedV1 {
                schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
                submission_id: robin_run_protocol::OpaqueId::new("shared-policy-submission")
                    .unwrap(),
                state: robin_run_protocol::SubmissionLifecycleV1::Queued,
                retry_after_ms: 250,
            })
            .unwrap(),
        )
        .unwrap();
        use RankedBrowseOnlyReason::{
            PeerAttestationRejected, PeerRankedSessionMismatch, RankedProtocolViolation,
        };
        vec![
            (
                NetMsg::RankedJoinChallenge(RankedJoinChallenge {
                    session_genesis: genesis(),
                    join_claim: RankedJoinClaimDocument::new(not_a_ranked_document()).unwrap(),
                }),
                PeerRankedSessionMismatch,
            ),
            (
                NetMsg::RankedJoinAccepted(RankedJoinAccepted {
                    session_genesis: genesis(),
                    join_attestation: RankedJoinAttestationDocument::new(not_a_ranked_document())
                        .unwrap(),
                    participant_roster: RankedParticipantRosterDocument::new(
                        not_a_ranked_document(),
                    )
                    .unwrap(),
                }),
                PeerAttestationRejected,
            ),
            (
                NetMsg::RankedParticipantRoster(
                    RankedParticipantRosterDocument::new(not_a_ranked_document()).unwrap(),
                ),
                PeerAttestationRejected,
            ),
            (
                NetMsg::RankedCoSignContext(
                    RankedCoSignContextDocument::new(not_a_ranked_document()).unwrap(),
                ),
                RankedProtocolViolation,
            ),
            (
                NetMsg::RankedSubmissionAccepted(
                    RankedSubmissionAcceptedDocument::new(not_a_ranked_document()).unwrap(),
                ),
                RankedProtocolViolation,
            ),
            // A well-formed acknowledgement is still out of phase before admission.
            (
                NetMsg::RankedSubmissionAccepted(well_formed_acknowledgement),
                RankedProtocolViolation,
            ),
            (
                NetMsg::LeaderboardCoSignRequest(leaderboard_request()),
                RankedProtocolViolation,
            ),
        ]
    }

    /// Assert the one invalid-ranked-message policy (10/F1 case 1) on any
    /// adapter: the session continues, the lane is irreversibly browse-only,
    /// the downgrade is published exactly once, and gameplay still flows.
    pub(in crate::multiplayer) fn assert_ranked_violation_downgrades<R: ClientRankedAdmission>(
        ranked: &R,
        message: NetMsg,
        reason: RankedBrowseOnlyReason,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let cosign = Arc::new(Default::default());
        let label = format!("{message:?}");
        handle(ranked, &tx, &cosign, message)
            .unwrap_or_else(|error| panic!("{label} ended the session: {error}"));
        match rx.try_recv() {
            Ok(NetEvent::RankedBrowseOnly { reason: published }) => {
                assert_eq!(published, reason, "{label}");
            }
            other => panic!("{label} published {other:?} instead of RankedBrowseOnly"),
        }
        assert!(
            rx.try_recv().is_err(),
            "{label} published more than the downgrade"
        );
        assert!(
            ranked_lifecycle_lock(ranked.lifecycle())
                .browse_only_reason()
                .is_some(),
            "{label} must downgrade the ranked lifecycle"
        );
        assert!(
            ranked.join_state().irreversibly_unranked().unwrap(),
            "{label} must make the lane browse-only"
        );
        handle(ranked, &tx, &cosign, NetMsg::Note("still playing".into())).unwrap();
        assert!(matches!(rx.try_recv(), Ok(NetEvent::Note(_))));
        handle(
            ranked,
            &tx,
            &cosign,
            NetMsg::RankedBrowseOnly {
                reason: RankedBrowseOnlyReason::HostRankedSessionUnavailable,
            },
        )
        .unwrap();
        assert!(
            rx.try_recv().is_err(),
            "a later browse-only notice is not published again"
        );
    }

    /// Drive `join` through the shared fixture to an accepted admission.
    #[cfg(not(target_arch = "wasm32"))]
    pub(in crate::multiplayer) fn admit_join_state(
        join: &ClientRankedJoinState,
    ) -> crate::multiplayer::tests::RankedJoinFixture {
        let fixture = crate::multiplayer::tests::ranked_join_fixture();
        assert_eq!(
            join.arm_expected_session(fixture.expected.clone()).unwrap(),
            None
        );
        assert!(
            join.receive_wire_challenge(fixture.challenge.clone())
                .unwrap()
                .is_some()
        );
        join.authorize_response(&fixture.response).unwrap();
        join.receive_wire_acceptance(fixture.accepted.clone())
            .unwrap();
        fixture
    }

    /// Assert the one reconnect reset (10/F1 case 3) on adapters built by
    /// `make`: unranked lanes stay unranked, an admitted lane needs a fresh
    /// admission that cannot replay the old challenge, and nothing is published.
    #[cfg(not(target_arch = "wasm32"))]
    pub(in crate::multiplayer) fn assert_reconnect_reset_policy<R: ClientRankedAdmission>(
        make: impl Fn() -> R,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();

        let fresh = make();
        reset_ranked_admission_for_reconnect(&fresh, &tx).unwrap();
        assert!(!fresh.join_state().irreversibly_unranked().unwrap());

        let admitted = make();
        let fixture = admit_join_state(admitted.join_state());
        reset_ranked_admission_for_reconnect(&admitted, &tx).unwrap();
        assert!(!admitted.join_state().is_accepted().unwrap());
        assert!(
            admitted
                .join_state()
                .receive_wire_challenge(fixture.challenge)
                .unwrap_err()
                .to_string()
                .contains("earlier stream"),
            "the accepted challenge is retired against replay"
        );
        assert!(
            ranked_lifecycle_lock(admitted.lifecycle())
                .browse_only_reason()
                .is_none(),
            "a transient drop does not downgrade an admitted run"
        );

        let unavailable = make();
        let staged = crate::multiplayer::tests::ranked_join_fixture();
        assert_eq!(
            unavailable
                .join_state()
                .receive_wire_challenge(staged.challenge)
                .unwrap(),
            None
        );
        unavailable
            .join_state()
            .authorize_response(&RankedJoinResponse::Unavailable(
                RankedJoinUnavailableReason::LocalRankedSessionUnavailable,
            ))
            .unwrap();
        reset_ranked_admission_for_reconnect(&unavailable, &tx).unwrap();
        assert!(unavailable.join_state().irreversibly_unranked().unwrap());

        let browse_only = make();
        assert!(
            browse_only
                .join_state()
                .mark_browse_only(RankedBrowseOnlyReason::HostRankedSessionUnavailable)
                .unwrap()
        );
        reset_ranked_admission_for_reconnect(&browse_only, &tx).unwrap();
        assert!(browse_only.join_state().irreversibly_unranked().unwrap());

        assert!(
            rx.try_recv().is_err(),
            "a reconnect reset publishes nothing"
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn mock_premature_begin_sim_downgrades_to_browse_only() {
        assert_premature_begin_sim_downgrades(&MockRanked::unresolved());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn mock_premature_cosign_request_downgrades_to_browse_only() {
        assert_premature_cosign_request_downgrades(&MockRanked::unresolved());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn shared_invalid_ranked_messages_downgrade_to_browse_only() {
        for (message, reason) in invalid_ranked_messages() {
            assert_ranked_violation_downgrades(&MockRanked::unresolved(), message, reason);
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn ranked_violation_with_a_closed_event_channel_is_fatal() {
        let ranked = MockRanked::unresolved();
        let (tx, rx) = std::sync::mpsc::channel();
        drop(rx);
        let cosign = Arc::new(Default::default());
        let error = handle(
            &ranked,
            &tx,
            &cosign,
            NetMsg::RankedSubmissionAccepted(
                RankedSubmissionAcceptedDocument::new(not_a_ranked_document()).unwrap(),
            ),
        )
        .expect_err("the game loop is gone");
        assert!(
            matches!(error, MultiplayerError::ChannelClosed(_)),
            "{error}"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn shared_reconnect_reset_policy() {
        assert_reconnect_reset_policy(MockRanked::unresolved);
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
    fn only_one_shot_ranked_publications_fail_the_session() {
        let request = leaderboard_request();
        for fatal in [
            NetOutbound::ArmLeaderboardCoSignRequest { request },
            NetOutbound::LeaderboardCoSignResponse(LeaderboardCoSignResponse {
                instance: request.instance,
                signer_public_key: [7; 32],
                signature: [8; 64],
            }),
        ] {
            assert!(publication_failure_is_fatal(&fatal), "{fatal:?}");
        }
        for reconnect in [
            NetOutbound::ReadyToSim { frame: 1 },
            NetOutbound::Input {
                origin_frame: 1,
                command: robin_engine::player_command::PlayerCommand::CrouchDown,
            },
        ] {
            assert!(!publication_failure_is_fatal(&reconnect), "{reconnect:?}");
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn begin_sim_requires_a_live_local_receiver() {
        let ranked = MockRanked::unresolved();
        ranked
            .join
            .mark_browse_only(RankedBrowseOnlyReason::HostRankedSessionUnavailable)
            .unwrap();
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
