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
    RankedBrowseOnlyReason, RankedJoinAttestationDocument, RankedJoinChallenge, RankedJoinResponse,
    RankedJoinUnavailableReason, RankedSessionConfigDocument, SharedClientLeaderboardCoSignState,
    SharedClientRankedJoinState, decode_msg, encode_msg, net_frame_class,
};
use crate::leaderboard_ranked_session::{
    CampaignContinuationReceiptSelectionRequestV1, CampaignContinuationReceiptSelectionResponseV1,
    OfficialRankedSessionSetupV1, OfficialRankedSessionWireSetupV1, RankedSessionClientAdmissionV1,
    RankedSessionLifecycle, SharedRankedSessionLifecycle,
};
use futures::future::{Either, select};
use futures::{FutureExt as _, pin_mut};
use gloo_timers::future::TimeoutFuture;
use iroh::endpoint::{Connection, ReadExactError, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use robin_engine::multiplayer::{BrowserPeerAuth, browser_seat_proof_message};
use robin_engine::player_command::PlayerId;
use robin_run_protocol::{
    CanonicalDocument as _, NamedSeatJoinClaimV1, PublicKey32, ReplaySessionGenesisV1,
    Validate as _,
};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use wasm_bindgen::JsCast as _;

const GAME_ALPN: &[u8] = b"robinhood/game/0";
const OUTGOING_POLL_MS: u32 = 4;
const INITIAL_CONNECT_TIMEOUT_MS: u32 = 15_000;
const RELAY_ONLINE_TIMEOUT_MS: u32 = 15_000;
// Mission authority is fetched and compared while level resources load. Slow
// browsers and cold HTTP caches routinely exceed a few seconds; retain a
// bounded failure path without racing normal bootstrap into browse-only.
const RANKED_SETUP_TIMEOUT_MS: u32 = 120_000;
const MAX_RECONNECT_BACKOFF_MS: u32 = 10_000;
const CONTENT_IDLE_TIMEOUT_MS: u32 = 30_000;
const CONTENT_DECISION_TIMEOUT_MS: u32 = 30 * 60 * 1_000;
const CONTENT_TRANSFER_TIMEOUT_MS: u32 = 15 * 60 * 1_000;
const CONTENT_READINESS_TIMEOUT_MS: u32 = 5 * 60 * 1_000;

/// Browser-only ranking state that must survive a dropped relay stream. The
/// shared gate owns the exact documents; the two cells retain transport facts
/// needed to reject a Welcome that races ahead of admission or changes seats
/// on reconnect.
struct BrowserRankedTransportState {
    join: SharedClientRankedJoinState,
    prepared_setup: RefCell<Option<OfficialRankedSessionSetupV1>>,
    pending_host_decision: Cell<bool>,
    host_admission_resolved: Cell<bool>,
    reconnect_eligible: Cell<bool>,
    welcomed_seat: Cell<Option<PlayerId>>,
    admitted_seat: Cell<Option<PlayerId>>,
    last_admitted_claim: RefCell<Option<NamedSeatJoinClaimV1>>,
    durable_public_key: Cell<Option<PublicKey32>>,
    authenticated_host_public_key: Cell<Option<PublicKey32>>,
}

impl Default for BrowserRankedTransportState {
    fn default() -> Self {
        Self {
            join: Arc::new(Default::default()),
            prepared_setup: RefCell::new(None),
            pending_host_decision: Cell::new(false),
            host_admission_resolved: Cell::new(false),
            reconnect_eligible: Cell::new(false),
            welcomed_seat: Cell::new(None),
            admitted_seat: Cell::new(None),
            last_admitted_claim: RefCell::new(None),
            durable_public_key: Cell::new(None),
            authenticated_host_public_key: Cell::new(None),
        }
    }
}

/// Browser-side handle to the live single-threaded iroh task.
pub struct ClientHandle {
    pub assigned_seat: Rc<RefCell<Option<PlayerId>>>,
    pub session_id: Rc<RefCell<Option<robin_engine::multiplayer::MultiplayerSessionId>>>,
    pub mission_seed: Rc<RefCell<Option<u64>>>,
    pub mission_sim_config: Rc<RefCell<Option<robin_engine::engine::SimConfig>>>,
    pub speech_timing_locale: Rc<RefCell<Option<Option<String>>>>,
    pub mission_id: Rc<RefCell<Option<String>>>,
    ranked_setup_tx: async_channel::Sender<Option<OfficialRankedSessionSetupV1>>,
    ranked_lifecycle: SharedRankedSessionLifecycle,
    ranked_local_public_key: Rc<Cell<Option<PublicKey32>>>,
    ranked_authenticated_host_public_key: PublicKey32,
    pub content_offer: Rc<RefCell<Option<robin_engine::multiplayer::DistributedModOffer>>>,
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

    pub fn content_offer(&self) -> Option<robin_engine::multiplayer::DistributedModOffer> {
        self.content_offer.borrow().clone()
    }

    pub fn startup_error(&self) -> Option<String> {
        self.startup_error.borrow().clone()
    }

    /// Install the exact locally prepared ranked-session authority before the
    /// host's split-phase admission challenge is answered. `None` explicitly
    /// declines ranking without aborting otherwise-compatible gameplay.
    pub fn install_ranked_session_setup(
        &self,
        setup: Option<OfficialRankedSessionSetupV1>,
    ) -> Result<(), String> {
        if let Some(setup) = &setup {
            setup
                .validate()
                .map_err(|error| format!("invalid local official ranked-session setup: {error}"))?;
        }
        self.ranked_setup_tx
            .try_send(setup)
            .map_err(|error| match error {
                async_channel::TrySendError::Full(_) => {
                    "browser ranked-session configuration was installed more than once".to_string()
                }
                async_channel::TrySendError::Closed(_) => {
                    "browser ranked-session transport is no longer running".to_string()
                }
            })
    }

    pub(crate) fn ranked_lifecycle(&self) -> SharedRankedSessionLifecycle {
        Arc::clone(&self.ranked_lifecycle)
    }

    pub(crate) fn ranked_local_seat(&self) -> Result<PlayerId, String> {
        self.assigned_seat()
            .ok_or_else(|| "browser multiplayer Welcome has not assigned a ranked seat".to_string())
    }

    pub(crate) fn ranked_local_public_key(&self) -> Option<PublicKey32> {
        self.ranked_local_public_key.get()
    }

    pub(crate) fn ranked_authenticated_host_public_key(&self) -> Option<PublicKey32> {
        Some(self.ranked_authenticated_host_public_key)
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
    let ranked_authenticated_host_public_key = PublicKey32::from_bytes(*server_addr.id.as_bytes());
    let ranked_local_public_key = Rc::new(Cell::new(None));
    let assigned_seat = Rc::new(RefCell::new(None));
    let session_id = Rc::new(RefCell::new(None));
    let mission_seed = Rc::new(RefCell::new(None));
    let mission_sim_config = Rc::new(RefCell::new(None));
    let speech_timing_locale = Rc::new(RefCell::new(None));
    let mission_id = Rc::new(RefCell::new(None));
    let content_offer = Rc::new(RefCell::new(None));
    let startup_error = Rc::new(RefCell::new(None));
    let cancellation = Rc::new(Cell::new(false));
    let (ranked_setup_tx, ranked_setup_rx) = async_channel::bounded(1);
    let ranked_lifecycle = Arc::new(std::sync::Mutex::new(
        RankedSessionLifecycle::awaiting_prepared_inputs(),
    ));
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
        Rc::clone(&ranked_local_public_key),
        ranked_setup_rx,
        Arc::clone(&ranked_lifecycle),
        Rc::clone(&content_offer),
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
        ranked_setup_tx,
        ranked_lifecycle,
        ranked_local_public_key,
        ranked_authenticated_host_public_key,
        content_offer,
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
    ranked_local_public_key_slot: Rc<Cell<Option<PublicKey32>>>,
    ranked_setup_rx: async_channel::Receiver<Option<OfficialRankedSessionSetupV1>>,
    ranked_lifecycle: SharedRankedSessionLifecycle,
    content_offer_slot: Rc<RefCell<Option<robin_engine::multiplayer::DistributedModOffer>>>,
    startup_error: Rc<RefCell<Option<String>>>,
    cancellation: Rc<Cell<bool>>,
) {
    let transport_key = SecretKey::generate();
    let transport_endpoint_id = transport_key.public();
    let browser_auth = match browser_peer_auth(&ticket, transport_endpoint_id).await {
        Ok(auth) => auth,
        Err(error) => {
            publish_startup_error(&startup_error, &incoming_tx, error);
            return;
        }
    };
    ranked_local_public_key_slot.set(Some(PublicKey32::from_bytes(
        browser_auth.durable_public_key,
    )));
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
    let invitation_session_id = match ticket.session_id() {
        Ok(session_id) => robin_engine::multiplayer::MultiplayerSessionId(session_id),
        Err(error) => {
            publish_startup_error(&startup_error, &incoming_tx, error);
            endpoint.close().await;
            return;
        }
    };

    let ranked_state = BrowserRankedTransportState::default();
    ranked_state
        .durable_public_key
        .set(Some(PublicKey32::from_bytes(
            browser_auth.durable_public_key,
        )));
    ranked_state
        .authenticated_host_public_key
        .set(Some(PublicKey32::from_bytes(*server_addr.id.as_bytes())));

    let first = initial_handshake(
        &endpoint,
        &server_addr,
        &nickname,
        &browser_auth,
        &ranked_state,
        &cancellation,
    )
    .await;
    let prelude = match first {
        Ok(result) => result,
        Err(error) => {
            publish_startup_error(&startup_error, &incoming_tx, error);
            endpoint.close().await;
            return;
        }
    };
    let (
        mut session,
        your_seat,
        mission_id,
        mission_seed,
        sim_config,
        speech_timing_locale,
        session_id,
        admitted_offer,
    ) = match prelude {
        HandshakePrelude::Welcome(handshake) => {
            let (session, seat, mission, seed, config, speech, session_id) = handshake;
            (
                session, seat, mission, seed, config, speech, session_id, None,
            )
        }
        HandshakePrelude::Content(session, offer) => {
            *content_offer_slot.borrow_mut() = Some(offer.clone());
            let _ = incoming_tx.send(NetEvent::ContentOffer(offer.clone()));
            match complete_content_admission(
                session,
                &offer,
                &incoming_tx,
                &mut outgoing_rx,
                &cancellation,
                invitation_session_id,
            )
            .await
            {
                Ok(ContentCompletion::Join(handshake)) => {
                    let (session, seat, mission, seed, config, speech, session_id) = handshake;
                    (
                        session,
                        seat,
                        mission,
                        seed,
                        config,
                        speech,
                        session_id,
                        Some(offer),
                    )
                }
                Ok(ContentCompletion::Prepared) => {
                    let _ = incoming_tx.send(NetEvent::Note(format!(
                        "verified and cached host content {} without joining a gameplay seat",
                        robin_engine::spellforge::hex_hash(&offer.full_mod_sha256)
                    )));
                    endpoint.close().await;
                    return;
                }
                Err(error) => {
                    publish_startup_error(
                        &startup_error,
                        &incoming_tx,
                        format!("distributed-mod admission failed: {error}"),
                    );
                    endpoint.close().await;
                    return;
                }
            }
        }
    };
    *content_offer_slot.borrow_mut() = admitted_offer.clone();
    if let Err(error) = mark_invitation_redeemed(ticket.payload().session_id.as_str()).await {
        publish_startup_error(&startup_error, &incoming_tx, error);
        endpoint.close().await;
        return;
    }

    ranked_state.welcomed_seat.set(Some(your_seat));
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

    let leaderboard_cosign_state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
    let mut backoff_ms = 500_u32;
    while !cancellation.get() {
        match run_session(
            session,
            &incoming_tx,
            &mut outgoing_rx,
            &leaderboard_cosign_state,
            &server_addr,
            transport_endpoint_id,
            &browser_auth,
            &ranked_setup_rx,
            &ranked_lifecycle,
            &ranked_state,
            &cancellation,
        )
        .await
        {
            SessionEnd::OutgoingClosed => break,
            SessionEnd::Drop(reason) => {
                if ranked_state.reconnect_eligible.get() {
                    if let Err(error) = ranked_state.join.begin_reconnect() {
                        let _ = incoming_tx.send(NetEvent::Fatal(format!(
                            "could not begin authenticated ranked reconnect: {error}"
                        )));
                        endpoint.close().await;
                        return;
                    }
                    ranked_state.pending_host_decision.set(false);
                    ranked_state.host_admission_resolved.set(false);
                    ranked_state.reconnect_eligible.set(false);
                }
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
            SessionEnd::Fatal(error) => {
                let _ = incoming_tx.send(NetEvent::Fatal(error));
                endpoint.close().await;
                return;
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
                Ok(prelude) => {
                    match resolve_reconnect(prelude, admitted_offer.as_ref(), session_id).await {
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
                            ranked_state.welcomed_seat.set(Some(next_seat));
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
                            if let Err(reset_error) =
                                reset_ranked_after_failed_handshake(&ranked_state)
                            {
                                let _ = incoming_tx.send(NetEvent::Fatal(format!(
                                    "could not reset ranked reconnect after content handshake failure: {reset_error}"
                                )));
                                endpoint.close().await;
                                return;
                            }
                            tracing::warn!(%error, backoff_ms, "browser iroh relay reconnect failed");
                            sleep_or_cancel(backoff_ms, &cancellation).await;
                            backoff_ms =
                                (backoff_ms.saturating_mul(2)).min(MAX_RECONNECT_BACKOFF_MS);
                        }
                    }
                }
                Err(error) => {
                    if let Err(reset_error) = reset_ranked_after_failed_handshake(&ranked_state) {
                        let _ = incoming_tx.send(NetEvent::Fatal(format!(
                            "could not reset ranked reconnect after transport failure: {reset_error}"
                        )));
                        endpoint.close().await;
                        return;
                    }
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
    ranked_state: &BrowserRankedTransportState,
    cancellation: &Cell<bool>,
) -> Result<HandshakePrelude, String> {
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
            Ok(Err(error)) => {
                reset_ranked_after_failed_handshake(ranked_state)?;
                last_error = error;
            }
            Err(()) => {
                reset_ranked_after_failed_handshake(ranked_state)?;
                last_error = "iroh relay connection attempt timed out".to_string();
            }
        }
        sleep_or_cancel(backoff_ms, cancellation).await;
        backoff_ms = (backoff_ms.saturating_mul(2)).min(500);
    }
    Err(format!(
        "could not reach the host through the iroh WebSocket relay within 15 seconds: {last_error}"
    ))
}

fn reset_ranked_after_failed_handshake(
    ranked_state: &BrowserRankedTransportState,
) -> Result<(), String> {
    if ranked_state.reconnect_eligible.get() {
        ranked_state.join.begin_reconnect()?;
        ranked_state.pending_host_decision.set(false);
        ranked_state.reconnect_eligible.set(false);
    }
    Ok(())
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

enum HandshakePrelude {
    Welcome(Handshake),
    Content(
        ClientSession,
        robin_engine::multiplayer::DistributedModOffer,
    ),
}

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
) -> Result<HandshakePrelude, String> {
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
            ranked_public_key: Some(browser_auth.durable_public_key),
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
                "browser received authoritative mission metadata through iroh WebSocket relay"
            );
            Ok(HandshakePrelude::Welcome((
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
            )))
        }
        Some(NetMsg::ContentOffer { offer }) => {
            offer
                .validate()
                .map_err(|error| format!("invalid distributed-mod offer: {error}"))?;
            let authenticated_host = server_addr.id.to_string();
            if offer.host_endpoint_id != authenticated_host {
                return Err(format!(
                    "distributed-mod offer claims host `{}`, but the authenticated iroh endpoint is `{authenticated_host}`",
                    offer.host_endpoint_id
                ));
            }
            Ok(HandshakePrelude::Content(
                ClientSession {
                    _connection: connection,
                    send,
                    recv,
                },
                offer,
            ))
        }
        Some(NetMsg::Reject { reason }) => Err(format!("host rejected connection: {reason}")),
        Some(other) => Err(format!("expected Welcome or ContentOffer, got {other:?}")),
        None => Err("host closed the stream before Welcome/content offer".to_string()),
    }
}

async fn read_welcome(
    mut session: ClientSession,
    expected_session_id: robin_engine::multiplayer::MultiplayerSessionId,
) -> Result<Handshake, String> {
    let message = with_timeout(CONTENT_IDLE_TIMEOUT_MS, read_frame(&mut session.recv))
        .await
        .map_err(|()| "post-content Welcome timed out".to_string())??;
    match message {
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
                return Err(
                    "post-content Welcome session does not match the signed invitation".to_string(),
                );
            }
            tracing::info!(
                ?your_seat,
                seed = mission_seed,
                host = %host_nickname,
                "browser welcomed after exact content admission"
            );
            Ok((
                session,
                your_seat,
                mission_id,
                mission_seed,
                sim_config,
                speech_timing_locale,
                session_id,
            ))
        }
        Some(NetMsg::Reject { reason }) => Err(format!("host rejected connection: {reason}")),
        Some(other) => Err(format!(
            "expected Welcome after content admission, got {other:?}"
        )),
        None => Err("host closed the stream before post-content Welcome".to_string()),
    }
}

enum ContentCompletion {
    Join(Handshake),
    Prepared,
}

async fn next_local_outbound(
    outgoing_rx: &mut Receiver<NetOutbound>,
    cancellation: &Cell<bool>,
    timeout_ms: u32,
    phase: &str,
) -> Result<NetOutbound, String> {
    let started = web_time::Instant::now();
    loop {
        if cancellation.get() {
            return Err(format!("{phase} cancelled"));
        }
        match outgoing_rx.try_recv() {
            Ok(message) => return Ok(message),
            Err(TryRecvError::Empty) if started.elapsed().as_millis() < u128::from(timeout_ms) => {
                TimeoutFuture::new(OUTGOING_POLL_MS).await;
            }
            Err(TryRecvError::Empty) => {
                return Err(format!("{phase} timed out after {timeout_ms} ms"));
            }
            Err(TryRecvError::Disconnected) => {
                return Err(format!("{phase} channel closed"));
            }
        }
    }
}

async fn complete_content_admission(
    mut session: ClientSession,
    offer: &robin_engine::multiplayer::DistributedModOffer,
    incoming_tx: &Sender<NetEvent>,
    outgoing_rx: &mut Receiver<NetOutbound>,
    cancellation: &Cell<bool>,
    expected_session_id: robin_engine::multiplayer::MultiplayerSessionId,
) -> Result<ContentCompletion, String> {
    let decision = next_local_outbound(
        outgoing_rx,
        cancellation,
        CONTENT_DECISION_TIMEOUT_MS,
        "content decision",
    )
    .await?;
    let mut received = match decision {
        NetOutbound::ContentRequest {
            full_mod_sha256,
            resume_offset,
        } if full_mod_sha256 == offer.full_mod_sha256 && resume_offset <= offer.encoded_bytes => {
            write_frame(
                &mut session.send,
                &NetMsg::ContentRequest {
                    full_mod_sha256,
                    resume_offset,
                },
            )
            .await?;
            resume_offset
        }
        NetOutbound::ContentRequest {
            full_mod_sha256,
            resume_offset,
        } => {
            return Err(format!(
                "invalid content request for {} at offset {resume_offset}; offered {} with {} bytes",
                robin_engine::spellforge::hex_hash(&full_mod_sha256),
                robin_engine::spellforge::hex_hash(&offer.full_mod_sha256),
                offer.encoded_bytes
            ));
        }
        NetOutbound::ContentReject {
            full_mod_sha256,
            reason,
        } if full_mod_sha256 == offer.full_mod_sha256 => {
            write_frame(
                &mut session.send,
                &NetMsg::ContentReject {
                    full_mod_sha256,
                    reason: reason.clone(),
                },
            )
            .await?;
            return Err(format!(
                "local player declined exact host content: {reason}"
            ));
        }
        other => {
            return Err(format!(
                "expected local ContentRequest/ContentReject, got {other:?}"
            ));
        }
    };

    let transfer_started = web_time::Instant::now();
    while received < offer.encoded_bytes {
        if cancellation.get() {
            return Err("content transfer cancelled".to_string());
        }
        if transfer_started.elapsed().as_millis() >= u128::from(CONTENT_TRANSFER_TIMEOUT_MS) {
            return Err(format!(
                "content transfer exceeded {CONTENT_TRANSFER_TIMEOUT_MS} ms"
            ));
        }
        let message = with_timeout(CONTENT_IDLE_TIMEOUT_MS, read_frame(&mut session.recv))
            .await
            .map_err(|()| "content chunk timed out".to_string())??;
        let Some(NetMsg::ContentChunk {
            full_mod_sha256,
            offset,
            total_bytes,
            bytes,
        }) = message
        else {
            return Err(format!(
                "expected sequential ContentChunk at offset {received}, got {message:?}"
            ));
        };
        if full_mod_sha256 != offer.full_mod_sha256
            || total_bytes != offer.encoded_bytes
            || offset != received
            || bytes.is_empty()
            || bytes.len() > robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT
        {
            return Err(format!(
                "invalid distributed-mod chunk: hash={} offset={offset} total={total_bytes} bytes={}; expected hash={} offset={received} total={} and 1..={} bytes",
                robin_engine::spellforge::hex_hash(&full_mod_sha256),
                bytes.len(),
                robin_engine::spellforge::hex_hash(&offer.full_mod_sha256),
                offer.encoded_bytes,
                robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT
            ));
        }
        let end = received
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| "distributed-mod chunk offset overflow".to_string())?;
        if end > offer.encoded_bytes {
            return Err(format!(
                "distributed-mod chunk ends at {end}, beyond offered {} bytes",
                offer.encoded_bytes
            ));
        }
        let _ = incoming_tx.send(NetEvent::ContentChunk {
            full_mod_sha256,
            offset,
            total_bytes,
            bytes,
        });
        received = end;
    }

    let ready = next_local_outbound(
        outgoing_rx,
        cancellation,
        CONTENT_READINESS_TIMEOUT_MS,
        "content readiness",
    )
    .await?;
    match ready {
        NetOutbound::ContentReady { full_mod_sha256 }
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            write_frame(&mut session.send, &NetMsg::ContentReady { full_mod_sha256 }).await?;
            Ok(ContentCompletion::Join(
                read_welcome(session, expected_session_id).await?,
            ))
        }
        NetOutbound::ContentPrepared { full_mod_sha256 }
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            write_frame(
                &mut session.send,
                &NetMsg::ContentPrepared { full_mod_sha256 },
            )
            .await?;
            Ok(ContentCompletion::Prepared)
        }
        NetOutbound::ContentReject {
            full_mod_sha256,
            reason,
        } if full_mod_sha256 == offer.full_mod_sha256 => {
            write_frame(
                &mut session.send,
                &NetMsg::ContentReject {
                    full_mod_sha256,
                    reason: reason.clone(),
                },
            )
            .await?;
            Err(format!(
                "downloaded host content failed local admission: {reason}"
            ))
        }
        other => Err(format!(
            "expected local ContentReady/ContentPrepared/ContentReject, got {other:?}"
        )),
    }
}

async fn resolve_reconnect(
    prelude: HandshakePrelude,
    admitted_offer: Option<&robin_engine::multiplayer::DistributedModOffer>,
    expected_session_id: robin_engine::multiplayer::MultiplayerSessionId,
) -> Result<Handshake, String> {
    match (prelude, admitted_offer) {
        (HandshakePrelude::Welcome(handshake), None) => Ok(handshake),
        (HandshakePrelude::Content(mut session, offer), Some(expected)) if &offer == expected => {
            write_frame(
                &mut session.send,
                &NetMsg::ContentRequest {
                    full_mod_sha256: offer.full_mod_sha256,
                    resume_offset: offer.encoded_bytes,
                },
            )
            .await?;
            write_frame(
                &mut session.send,
                &NetMsg::ContentReady {
                    full_mod_sha256: offer.full_mod_sha256,
                },
            )
            .await?;
            read_welcome(session, expected_session_id).await
        }
        (HandshakePrelude::Content(_, offer), Some(expected)) => Err(format!(
            "browser reconnect changed host content from {} to {}",
            robin_engine::spellforge::hex_hash(&expected.full_mod_sha256),
            robin_engine::spellforge::hex_hash(&offer.full_mod_sha256)
        )),
        (HandshakePrelude::Content(_, offer), None) => Err(format!(
            "browser reconnect unexpectedly introduced host content {}",
            robin_engine::spellforge::hex_hash(&offer.full_mod_sha256)
        )),
        (HandshakePrelude::Welcome(_), Some(expected)) => Err(format!(
            "browser reconnect omitted admitted host content {}",
            robin_engine::spellforge::hex_hash(&expected.full_mod_sha256)
        )),
    }
}

#[allow(clippy::too_many_arguments)]
async fn answer_ranked_join_challenge(
    challenge: RankedJoinChallenge,
    authenticated_host_endpoint: EndpointId,
    authenticated_transport_endpoint: EndpointId,
    browser_auth: &BrowserPeerAuth,
    ranked_setup_rx: &async_channel::Receiver<Option<OfficialRankedSessionSetupV1>>,
    ranked_lifecycle: &SharedRankedSessionLifecycle,
    ranked_state: &BrowserRankedTransportState,
    incoming_tx: &Sender<NetEvent>,
) -> Result<RankedJoinResponse, String> {
    if ranked_state.pending_host_decision.get() {
        return Err("host replayed or replaced a pending ranked join challenge".to_string());
    }

    // Stage the independently authenticated host value first. On the initial
    // stream this permits an exact local mismatch to be consumed by the closed
    // Unavailable response. Reconnects retain the expected configuration and
    // release only a fresh, matching challenge immediately.
    let already_released = ranked_state
        .join
        .receive_wire_challenge(challenge.clone())?;

    let local_setup = if let Some(setup) = ranked_state.prepared_setup.borrow().clone() {
        setup
    } else {
        let setup = match with_timeout(RANKED_SETUP_TIMEOUT_MS, ranked_setup_rx.recv()).await {
            Ok(Ok(setup)) => setup,
            Ok(Err(_)) | Err(()) => None,
        };
        let Some(setup) = setup else {
            downgrade_ranked_lifecycle(
                ranked_lifecycle,
                "browser ranked setup was explicitly unavailable",
            )?;
            return ranked_unavailable_response(
                ranked_state,
                RankedJoinUnavailableReason::LocalRankedSessionUnavailable,
            );
        };
        let admission = RankedSessionClientAdmissionV1::new_official(
            setup.clone(),
            PublicKey32::from_bytes(*authenticated_host_endpoint.as_bytes()),
            PublicKey32::from_bytes(browser_auth.durable_public_key),
            PublicKey32::from_bytes(*authenticated_transport_endpoint.as_bytes()),
        )
        .map_err(|error| format!("prepare authenticated browser ranked admission: {error}"))?;
        ranked_lifecycle
            .lock()
            .map_err(|_| "browser ranked lifecycle lock is poisoned".to_string())?
            .install_client_admission(admission)
            .map_err(|error| format!("install authenticated browser ranked admission: {error}"))?;
        *ranked_state.prepared_setup.borrow_mut() = Some(setup.clone());
        setup
    };
    let local_config = &local_setup.ranked_session;

    let released = if let Some(released) = already_released {
        released
    } else {
        let local_document =
            match crate::leaderboard_ranked_session::encode_ranked_wire_document(local_config)
                .map_err(|error| format!("encode browser ranked-session configuration: {error}"))
                .and_then(|bytes| {
                    RankedSessionConfigDocument::new(bytes).map_err(|error| {
                        format!("wrap browser ranked-session configuration: {error}")
                    })
                }) {
                Ok(document) => document,
                Err(error) => {
                    tracing::warn!(%error, "browser ranked setup could not be encoded");
                    downgrade_ranked_lifecycle(
                        ranked_lifecycle,
                        "browser ranked setup could not be canonically encoded",
                    )?;
                    return ranked_unavailable_response(
                        ranked_state,
                        RankedJoinUnavailableReason::LocalRankedSessionUnavailable,
                    );
                }
            };
        match ranked_state.join.arm_expected_session(local_document) {
            Ok(Some(released)) => released,
            Ok(None) => {
                return Err(
                    "ranked join setup did not release its already-staged challenge".to_string(),
                );
            }
            Err(error) => {
                tracing::warn!(%error, "browser ranked challenge did not match local setup");
                downgrade_ranked_lifecycle(
                    ranked_lifecycle,
                    "host ranked challenge did not match the prepared browser session",
                )?;
                return ranked_unavailable_response(
                    ranked_state,
                    RankedJoinUnavailableReason::LocalRankedSessionMismatch,
                );
            }
        }
    };

    let claim = match validate_browser_ranked_challenge(
        &released,
        &local_setup,
        authenticated_host_endpoint,
        authenticated_transport_endpoint,
        browser_auth,
        ranked_state.welcomed_seat.get(),
        ranked_state.last_admitted_claim.borrow().as_ref(),
    ) {
        Ok(claim) => claim,
        Err(error) => {
            tracing::warn!(%error, "browser ranked challenge failed endpoint binding");
            downgrade_ranked_lifecycle(
                ranked_lifecycle,
                "host ranked challenge failed browser identity binding",
            )?;
            return ranked_unavailable_response(
                ranked_state,
                RankedJoinUnavailableReason::LocalRankedSessionMismatch,
            );
        }
    };

    incoming_tx
        .send(NetEvent::RankedJoinChallenge(released))
        .map_err(|_| "browser ranked challenge channel is closed".to_string())?;
    let attestation =
        match crate::leaderboard_signing::browser_game_sign_named_seat_join(&claim).await {
            Ok(attestation) => attestation,
            Err(error) => {
                tracing::warn!(%error, "isolated browser ranked admission signer unavailable");
                downgrade_ranked_lifecycle(
                    ranked_lifecycle,
                    "isolated browser ranked admission signer was unavailable",
                )?;
                return ranked_unavailable_response(
                    ranked_state,
                    RankedJoinUnavailableReason::AttestationSigningFailed,
                );
            }
        };
    let attestation_bytes =
        crate::leaderboard_ranked_session::encode_ranked_wire_document(&attestation)
            .map_err(|error| format!("encode browser ranked join attestation: {error}"))?;
    let attestation_document = RankedJoinAttestationDocument::new(attestation_bytes)
        .map_err(|error| format!("wrap browser ranked join attestation: {error}"))?;
    let response = RankedJoinResponse::Attestation(attestation_document);
    ranked_state.join.authorize_response(&response)?;
    ranked_state.reconnect_eligible.set(true);
    ranked_state.pending_host_decision.set(true);
    Ok(response)
}

fn ranked_unavailable_response(
    ranked_state: &BrowserRankedTransportState,
    reason: RankedJoinUnavailableReason,
) -> Result<RankedJoinResponse, String> {
    let response = RankedJoinResponse::Unavailable(reason);
    ranked_state.join.authorize_response(&response)?;
    ranked_state.pending_host_decision.set(true);
    ranked_state.reconnect_eligible.set(false);
    Ok(response)
}

fn downgrade_ranked_lifecycle(
    lifecycle: &SharedRankedSessionLifecycle,
    reason: &'static str,
) -> Result<(), String> {
    lifecycle
        .lock()
        .map_err(|_| "browser ranked lifecycle lock is poisoned".to_string())?
        .downgrade(reason);
    Ok(())
}

fn ranked_browse_only_reason(reason: RankedBrowseOnlyReason) -> &'static str {
    match reason {
        RankedBrowseOnlyReason::HostRankedSessionUnavailable => {
            "host ranked session was unavailable"
        }
        RankedBrowseOnlyReason::PeerIdentityUnavailable => "a peer ranked identity was unavailable",
        RankedBrowseOnlyReason::PeerRankedSessionMismatch => "a peer ranked session did not match",
        RankedBrowseOnlyReason::PeerAttestationRejected => "a peer ranked attestation was rejected",
        RankedBrowseOnlyReason::RankedTransportInterrupted => {
            "ranked multiplayer transport was interrupted"
        }
        RankedBrowseOnlyReason::RankedProtocolViolation => "ranked multiplayer protocol violation",
    }
}

fn validate_browser_ranked_challenge(
    challenge: &RankedJoinChallenge,
    expected_setup: &OfficialRankedSessionSetupV1,
    authenticated_host_endpoint: EndpointId,
    authenticated_transport_endpoint: EndpointId,
    browser_auth: &BrowserPeerAuth,
    welcomed_seat: Option<PlayerId>,
    previous_claim: Option<&NamedSeatJoinClaimV1>,
) -> Result<NamedSeatJoinClaimV1, String> {
    let genesis: ReplaySessionGenesisV1 =
        crate::leaderboard_ranked_session::decode_ranked_wire_document(
            challenge.session_genesis.as_bytes(),
        )
        .map_err(|error| format!("decode browser ranked session genesis: {error}"))?;
    crate::leaderboard_ranked_session::validate_official_session_genesis(
        &genesis,
        *authenticated_host_endpoint.as_bytes(),
        expected_setup,
    )
    .map_err(|error| format!("validate browser ranked session genesis: {error}"))?;
    let claim: NamedSeatJoinClaimV1 =
        crate::leaderboard_ranked_session::decode_ranked_wire_document(
            challenge.join_claim.as_bytes(),
        )
        .map_err(|error| format!("decode browser ranked join claim: {error}"))?;
    let expected_genesis = genesis
        .canonical_digest()
        .map_err(|error| format!("digest browser ranked session genesis: {error}"))?;
    let expected_public_key = PublicKey32::from_bytes(browser_auth.durable_public_key);
    let expected_host_endpoint = PublicKey32::from_bytes(*authenticated_host_endpoint.as_bytes());
    let expected_transport_endpoint =
        PublicKey32::from_bytes(*authenticated_transport_endpoint.as_bytes());
    let expected_config = &expected_setup.ranked_session;
    let expected_seat = welcomed_seat
        .ok_or_else(|| "ranked challenge arrived before authoritative Welcome seat".to_string())?;
    if claim.session_genesis_sha256 != expected_genesis
        || claim.public_key != expected_public_key
        || claim.transport_endpoint_id != expected_transport_endpoint
        || claim.host_endpoint_id != expected_host_endpoint
        || claim.host_endpoint_id != genesis.claim.host_public_key
        || claim.replay_session_id != genesis.claim.replay_session_id
        || claim.host_nonce != genesis.claim.host_nonce
        || claim.mission_id != expected_config.mission_id
        || claim.content_manifest_sha256 != expected_config.content_manifest_sha256
        || claim.rules_config_sha256 != expected_config.rules_config_sha256
        || claim.ruleset_manifest_sha256 != expected_config.ruleset_manifest_sha256
        || claim.competition_manifest_sha256 != expected_config.competition_manifest_sha256
        || claim.seat == 0
        || u8::try_from(claim.seat).ok().map(PlayerId) != Some(expected_seat)
    {
        return Err(
            "ranked join claim does not match the durable browser identity, authenticated endpoints, or exact prepared session"
                .to_string(),
        );
    }
    match previous_claim {
        None if claim.connection_epoch != 0 => {
            return Err("initial ranked browser admission has a nonzero connection epoch".into());
        }
        Some(previous)
            if claim.seat != previous.seat
                || claim.public_key != previous.public_key
                || claim.participant_instance_id != previous.participant_instance_id
                || previous.connection_epoch.checked_add(1) != Some(claim.connection_epoch)
                || claim.join_event_ordinal <= previous.join_event_ordinal =>
        {
            return Err(
                "ranked browser reconnect changed its admitted participant or did not advance its lifecycle"
                    .into(),
            );
        }
        None | Some(_) => {}
    }
    Ok(claim)
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
    Fatal(String),
    OutgoingClosed,
}

async fn run_session(
    session: ClientSession,
    incoming_tx: &Sender<NetEvent>,
    outgoing_rx: &mut Receiver<NetOutbound>,
    leaderboard_cosign_state: &SharedClientLeaderboardCoSignState,
    server_addr: &EndpointAddr,
    transport_endpoint_id: EndpointId,
    browser_auth: &BrowserPeerAuth,
    ranked_setup_rx: &async_channel::Receiver<Option<OfficialRankedSessionSetupV1>>,
    ranked_lifecycle: &SharedRankedSessionLifecycle,
    ranked_state: &BrowserRankedTransportState,
    cancellation: &Cell<bool>,
) -> SessionEnd {
    let ClientSession {
        _connection,
        mut send,
        mut recv,
    } = session;
    let (ranked_response_tx, ranked_response_rx) = async_channel::bounded(1);
    let reader = async {
        loop {
            match read_frame(&mut recv).await {
                Ok(Some(NetMsg::RankedJoinChallenge(challenge))) => {
                    let response = match answer_ranked_join_challenge(
                        challenge,
                        server_addr.id,
                        transport_endpoint_id,
                        browser_auth,
                        ranked_setup_rx,
                        ranked_lifecycle,
                        ranked_state,
                        incoming_tx,
                    )
                    .await
                    {
                        Ok(response) => response,
                        Err(error) => return SessionEnd::Fatal(error),
                    };
                    if ranked_response_tx.try_send(response).is_err() {
                        return SessionEnd::Fatal(
                            "browser ranked admission response queue is occupied".to_string(),
                        );
                    }
                }
                Ok(Some(message)) => {
                    if let Err(error) = handle_client_wire_msg(
                        incoming_tx,
                        leaderboard_cosign_state,
                        ranked_lifecycle,
                        ranked_state,
                        message,
                    ) {
                        return if error.starts_with("host requires a full-snapshot reconnect:") {
                            SessionEnd::Drop(error)
                        } else {
                            SessionEnd::Fatal(error)
                        };
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
        let mut pending_ready_frame = None;
        loop {
            if cancellation.get() {
                return SessionEnd::OutgoingClosed;
            }
            if let Ok(response) = ranked_response_rx.try_recv() {
                if let Err(error) =
                    write_frame(&mut send, &NetMsg::RankedJoinResponse(response)).await
                {
                    return SessionEnd::Drop(error);
                }
                continue;
            }
            if let Some(frame) = pending_ready_frame {
                if ranked_state.host_admission_resolved.get() {
                    if let Err(error) = write_frame(&mut send, &NetMsg::ReadyToSim { frame }).await
                    {
                        return SessionEnd::Drop(error);
                    }
                    pending_ready_frame = None;
                } else {
                    TimeoutFuture::new(OUTGOING_POLL_MS).await;
                }
                continue;
            }
            match outgoing_rx.try_recv() {
                Ok(NetOutbound::ReadyToSim { frame })
                    if !ranked_state.host_admission_resolved.get() =>
                {
                    pending_ready_frame = Some(frame);
                }
                Ok(outgoing) => {
                    let leaderboard_control = matches!(
                        &outgoing,
                        NetOutbound::ArmRankedJoin { .. }
                            | NetOutbound::RankedJoinResponse(_)
                            | NetOutbound::RankedContinuationReceiptSelection(_)
                            | NetOutbound::RankedContinuationPreflightSignature(_)
                            | NetOutbound::LeaderboardCoSignRequest { .. }
                            | NetOutbound::ArmLeaderboardCoSignRequest { .. }
                            | NetOutbound::LeaderboardCoSignResponse(_)
                    );
                    if let Err(error) = send_client_outgoing(
                        &mut send,
                        outgoing,
                        incoming_tx,
                        leaderboard_cosign_state,
                        ranked_state,
                    )
                    .await
                    {
                        return if leaderboard_control {
                            SessionEnd::Fatal(error)
                        } else {
                            SessionEnd::Drop(error)
                        };
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

fn handle_client_wire_msg(
    incoming_tx: &Sender<NetEvent>,
    leaderboard_cosign_state: &SharedClientLeaderboardCoSignState,
    ranked_lifecycle: &SharedRankedSessionLifecycle,
    ranked_state: &BrowserRankedTransportState,
    message: NetMsg,
) -> Result<(), String> {
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
            if !ranked_state.host_admission_resolved.get() {
                return Err(
                    "host began simulation before ranked admission or browse-only resolution"
                        .to_string(),
                );
            }
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
        NetMsg::RankedBrowseOnly { reason } => {
            if ranked_state.join.mark_browse_only(reason)? {
                downgrade_ranked_lifecycle(ranked_lifecycle, ranked_browse_only_reason(reason))?;
                ranked_state.pending_host_decision.set(false);
                ranked_state.host_admission_resolved.set(true);
                ranked_state.reconnect_eligible.set(false);
                incoming_tx
                    .send(NetEvent::RankedBrowseOnly { reason })
                    .map_err(|_| "browser ranked browse-only channel is closed".to_string())?;
            }
        }
        NetMsg::RankedOfficialSessionSetup(document) => {
            crate::leaderboard_ranked_session::decode_canonical_ranked_wire_document::<
                OfficialRankedSessionWireSetupV1,
            >(document.as_bytes())
            .map_err(|error| format!("invalid official ranked wire setup: {error}"))?;
            incoming_tx
                .send(NetEvent::RankedOfficialSessionSetup(document))
                .map_err(|_| "browser official ranked setup channel is closed".to_string())?;
        }
        NetMsg::RankedContinuationReceiptSelectionRequest(document) => {
            let request = crate::leaderboard_ranked_session::decode_ranked_wire_document::<
                CampaignContinuationReceiptSelectionRequestV1,
            >(document.as_bytes())
            .map_err(|error| format!("invalid continuation receipt selection request: {error}"))?;
            let local_public_key = ranked_state.durable_public_key.get().ok_or_else(|| {
                "browser continuation receipt selection has no durable identity".to_string()
            })?;
            let host_public_key = ranked_state
                .authenticated_host_public_key
                .get()
                .ok_or_else(|| {
                    "browser continuation receipt selection has no authenticated host".to_string()
                })?;
            if request.lobby.host_public_key != host_public_key
                || request
                    .lobby
                    .participant_public_keys
                    .binary_search(&local_public_key)
                    .is_err()
            {
                return Err(
                    "continuation receipt selection request does not bind the authenticated browser session"
                        .to_string(),
                );
            }
            incoming_tx
                .send(NetEvent::RankedContinuationReceiptSelectionRequest(
                    document,
                ))
                .map_err(|_| {
                    "browser continuation receipt selection channel is closed".to_string()
                })?;
        }
        NetMsg::RankedContinuationReceiptSelection(_) => {
            return Err("host sent a client-only continuation receipt selection".to_string());
        }
        NetMsg::RankedContinuationPreflightClaim(document) => {
            let claim = crate::leaderboard_ranked_session::decode_ranked_wire_document::<
                robin_run_protocol::CampaignContinuationPreflightRequestClaimV1,
            >(document.as_bytes())
            .map_err(|error| format!("invalid continuation preflight claim: {error}"))?;
            let local_public_key = ranked_state.durable_public_key.get().ok_or_else(|| {
                "browser continuation preflight has no durable identity".to_string()
            })?;
            let host_public_key = ranked_state
                .authenticated_host_public_key
                .get()
                .ok_or_else(|| {
                    "browser continuation preflight has no authenticated host".to_string()
                })?;
            if claim.host_public_key != host_public_key
                || claim.campaign_controller_public_key != local_public_key
            {
                return Err(
                    "continuation preflight claim does not bind the authenticated browser controller"
                        .to_string(),
                );
            }
            incoming_tx
                .send(NetEvent::RankedContinuationPreflightClaim(document))
                .map_err(|_| "browser continuation preflight channel is closed".to_string())?;
        }
        NetMsg::RankedContinuationPreflightSignature(_) => {
            return Err("host sent a client-only continuation preflight signature".to_string());
        }
        NetMsg::RankedCoSignContext(context) => {
            if !ranked_state.join.is_accepted()? {
                return Err(
                    "host sent a ranked co-sign context outside an accepted ranked session"
                        .to_string(),
                );
            }
            incoming_tx
                .send(NetEvent::RankedCoSignContext(context))
                .map_err(|_| "browser ranked co-sign context channel is closed".to_string())?;
        }
        NetMsg::RankedJoinAccepted(accepted) => {
            let accepted = ranked_state.join.receive_wire_acceptance(accepted)?;
            let genesis: ReplaySessionGenesisV1 =
                crate::leaderboard_ranked_session::decode_ranked_wire_document(
                    accepted.session_genesis.as_bytes(),
                )
                .map_err(|error| format!("decode accepted browser ranked genesis: {error}"))?;
            let attestation: robin_run_protocol::NamedSeatJoinAttestationV1 =
                crate::leaderboard_ranked_session::decode_ranked_wire_document(
                    accepted.join_attestation.as_bytes(),
                )
                .map_err(|error| {
                    format!("decode accepted browser ranked join attestation: {error}")
                })?;
            let roster: Vec<robin_run_protocol::ParticipantClaimV1> =
                serde_json::from_slice(accepted.participant_roster.as_bytes())
                    .map_err(|error| format!("decode accepted browser ranked roster: {error}"))?;
            let admitted_seat = u8::try_from(attestation.claim.seat)
                .map(PlayerId)
                .map_err(|_| "ranked browser seat does not fit the gameplay seat".to_string())?;
            {
                let mut lifecycle = ranked_lifecycle
                    .lock()
                    .map_err(|_| "browser ranked lifecycle lock is poisoned".to_string())?;
                if lifecycle.client_admission().is_some() {
                    lifecycle
                        .accept_ranked_client(attestation.claim.seat, genesis, roster)
                        .map_err(|error| {
                            format!("accept authenticated browser ranked client: {error}")
                        })?;
                } else if lifecycle.ranked_client().is_some() {
                    lifecycle
                        .update_ranked_client_roster(&genesis, roster)
                        .map_err(|error| {
                            format!("accept browser ranked reconnect roster: {error}")
                        })?;
                } else {
                    return Err(
                        "ranked join acceptance has no pending or admitted browser lifecycle"
                            .to_string(),
                    );
                }
            }
            ranked_state.admitted_seat.set(Some(admitted_seat));
            *ranked_state.last_admitted_claim.borrow_mut() = Some(attestation.claim);
            ranked_state.pending_host_decision.set(false);
            ranked_state.host_admission_resolved.set(true);
            ranked_state.reconnect_eligible.set(true);
            incoming_tx
                .send(NetEvent::RankedJoinAccepted(accepted))
                .map_err(|_| "browser ranked admission channel is closed".to_string())?;
        }
        NetMsg::RankedParticipantRoster(document) => {
            let document = ranked_state.join.receive_wire_roster_update(document)?;
            let roster: Vec<robin_run_protocol::ParticipantClaimV1> =
                serde_json::from_slice(document.as_bytes())
                    .map_err(|error| format!("decode browser ranked roster update: {error}"))?;
            let mut lifecycle = ranked_lifecycle
                .lock()
                .map_err(|_| "browser ranked lifecycle lock is poisoned".to_string())?;
            let genesis = lifecycle
                .ranked_client()
                .map(|client| client.session_genesis.clone())
                .ok_or_else(|| {
                    "ranked roster update has no accepted browser client lifecycle".to_string()
                })?;
            lifecycle
                .update_ranked_client_roster(&genesis, roster)
                .map_err(|error| format!("install browser ranked roster update: {error}"))?;
            drop(lifecycle);
            incoming_tx
                .send(NetEvent::RankedParticipantRoster(document))
                .map_err(|_| "browser ranked roster channel is closed".to_string())?;
        }
        NetMsg::RankedJoinResponse(_) => {
            return Err("host sent a client-only ranked join response".to_string());
        }
        NetMsg::LeaderboardCoSignRequest(request) => {
            if let Some(request) = leaderboard_cosign_state.receive_wire_request(request)? {
                incoming_tx
                    .send(NetEvent::LeaderboardCoSignRequest(request))
                    .map_err(|_| {
                        "browser leaderboard co-sign request channel is closed".to_string()
                    })?;
            }
        }
        NetMsg::LeaderboardCoSignResponse(_) => {
            return Err("host sent a client-only leaderboard co-sign response".to_string());
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

async fn send_client_outgoing(
    send: &mut SendStream,
    outgoing: NetOutbound,
    incoming_tx: &Sender<NetEvent>,
    leaderboard_cosign_state: &SharedClientLeaderboardCoSignState,
    ranked_state: &BrowserRankedTransportState,
) -> Result<(), String> {
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
        | NetOutbound::BeginSnapshotTransition { .. }
        | NetOutbound::RankedJoinChallenge { .. }
        | NetOutbound::RankedJoinAccepted { .. }
        | NetOutbound::RankedParticipantRoster { .. }
        | NetOutbound::RankedOfficialSessionSetup(_)
        | NetOutbound::RankedBrowseOnly { .. }
        | NetOutbound::RankedContinuationReceiptSelectionRequest(_)
        | NetOutbound::RankedContinuationPreflightClaim { .. }
        | NetOutbound::RankedCoSignContext { .. }
        | NetOutbound::RankedSubmissionAccepted { .. } => {
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
        NetOutbound::ArmRankedJoin { .. } => {
            return Err(
                "browser ranked setup must be installed on ClientHandle before admission"
                    .to_string(),
            );
        }
        NetOutbound::RankedJoinResponse(_) => {
            return Err(
                "browser game attempted to bypass the isolated ranked admission signer".to_string(),
            );
        }
        NetOutbound::LeaderboardCoSignRequest { .. } => {
            return Err(
                "browser client attempted a server-only leaderboard co-sign request".to_string(),
            );
        }
        NetOutbound::ArmLeaderboardCoSignRequest { request } => {
            if !ranked_state.join.is_accepted()? {
                return Err(
                    "browser attempted to arm a leaderboard co-sign outside an accepted ranked session"
                        .to_string(),
                );
            }
            if let Some(request) = leaderboard_cosign_state.arm_request(request)? {
                incoming_tx
                    .send(NetEvent::LeaderboardCoSignRequest(request))
                    .map_err(|_| {
                        "browser leaderboard co-sign request channel is closed".to_string()
                    })?;
            }
        }
        NetOutbound::LeaderboardCoSignResponse(response) => {
            if !ranked_state.join.is_accepted()? {
                return Err(
                    "browser attempted a leaderboard co-sign outside an accepted ranked session"
                        .to_string(),
                );
            }
            leaderboard_cosign_state.authorize_response(&response)?;
            write_frame(send, &NetMsg::LeaderboardCoSignResponse(response)).await?;
        }
        NetOutbound::RankedContinuationReceiptSelection(selection) => {
            let decoded = crate::leaderboard_ranked_session::decode_ranked_wire_document::<
                CampaignContinuationReceiptSelectionResponseV1,
            >(selection.as_bytes())
            .map_err(|error| format!("invalid continuation receipt selection: {error}"))?;
            let local_public_key = ranked_state.durable_public_key.get().ok_or_else(|| {
                "browser continuation receipt selection has no durable identity".to_string()
            })?;
            if decoded.responder_public_key() != local_public_key {
                return Err(
                    "browser continuation receipt selection is controlled by another identity"
                        .to_string(),
                );
            }
            write_frame(send, &NetMsg::RankedContinuationReceiptSelection(selection)).await?;
        }
        NetOutbound::RankedContinuationPreflightSignature(signature) => {
            let decoded =
                crate::leaderboard_ranked_session::decode_canonical_ranked_wire_document::<
                    robin_run_protocol::ParticipantSignatureV1,
                >(signature.as_bytes())
                .map_err(|error| format!("invalid continuation preflight signature: {error}"))?;
            let local_public_key = ranked_state.durable_public_key.get().ok_or_else(|| {
                "browser continuation preflight has no durable identity".to_string()
            })?;
            if decoded.public_key != local_public_key || decoded.signature.is_zero() {
                return Err(
                    "browser continuation preflight signature uses the wrong identity".to_string(),
                );
            }
            write_frame(
                send,
                &NetMsg::RankedContinuationPreflightSignature(signature),
            )
            .await?;
        }
        NetOutbound::ContentRequest { .. }
        | NetOutbound::ContentReject { .. }
        | NetOutbound::ContentReady { .. }
        | NetOutbound::ContentPrepared { .. } => {
            return Err(
                "browser client queued a content-admission message after gameplay began"
                    .to_string(),
            );
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
    use super::{
        BrowserRankedTransportState, SharedClientLeaderboardCoSignState, handle_client_wire_msg,
        validate_reconnect_state,
    };
    use robin_engine::player_command::PlayerId;
    use robin_run_protocol::{
        Digest32, LeaderboardCoSignInstanceV1, LeaderboardCoSignPurposeV1,
        LeaderboardCoSignRequestV1,
    };
    use std::sync::Arc;

    fn leaderboard_request() -> LeaderboardCoSignRequestV1 {
        LeaderboardCoSignRequestV1 {
            instance: LeaderboardCoSignInstanceV1 {
                purpose: LeaderboardCoSignPurposeV1::Submission,
                replay_session_id: Digest32::from_bytes([41; 32]),
                submission_offer_sha256: Digest32::from_bytes([42; 32]),
            },
            run_digest: Digest32::from_bytes([43; 32]),
        }
    }

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

    #[test]
    fn browser_handler_exposes_only_an_exact_locally_armed_request() {
        let request = leaderboard_request();
        let state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
        let lifecycle = Arc::new(std::sync::Mutex::new(
            super::RankedSessionLifecycle::awaiting_prepared_inputs(),
        ));
        let (incoming_tx, incoming_rx) = std::sync::mpsc::channel();

        handle_client_wire_msg(
            &incoming_tx,
            &state,
            &lifecycle,
            &BrowserRankedTransportState::default(),
            robin_engine::multiplayer::NetMsg::LeaderboardCoSignRequest(request),
        )
        .unwrap();
        assert!(incoming_rx.try_recv().is_err());
        assert_eq!(state.arm_request(request).unwrap(), Some(request));

        let error = handle_client_wire_msg(
            &incoming_tx,
            &state,
            &lifecycle,
            &BrowserRankedTransportState::default(),
            robin_engine::multiplayer::NetMsg::LeaderboardCoSignResponse(
                robin_engine::multiplayer::LeaderboardCoSignResponse {
                    instance: request.instance,
                    signer_public_key: [7; 32],
                    signature: [8; 64],
                },
            ),
        )
        .unwrap_err();
        assert!(error.contains("client-only"));
    }
}
