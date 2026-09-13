//! Browser iroh multiplayer client.
//!
//! Browsers cannot use iroh's UDP discovery paths. The endpoint therefore
//! dials the one host-signed HTTPS relay route from a browser join ticket and
//! carries the native game's unchanged ALPN, bidirectional stream, framing,
//! admission, rollback, and snapshot protocol over the relay WebSocket.
//!
//! The session state machine is shared with native in `client_session`; this
//! module supplies the browser endpoint, timers, polled outbound queue and the
//! stable shell's identity/invitation glue. Ranked admission lives in
//! `browser_ranked`.
//!
//! Browser hosting and DHT discovery remain deliberately unsupported.
//! TODO(browser-webrtc): if iroh gains a production WebRTC path, add it below
//! this endpoint abstraction instead of inventing a second game protocol.

use super::browser_ranked::BrowserRankedAdmission;
use super::client_protocol::ClientConfig;
use super::client_session::{
    self, ClientHandle, ClientSlots, ClientTimer, ClientTimings, ClientTransport, InitialHandshake,
    SessionEnd, StartupFailure, WriterCommand, ranked_setup_channel,
};
use super::join_ticket::BrowserJoinTicket;
use super::{NET_PROTOCOL_VERSION, NetEvent, NetMsg, NetOutbound, RankedJoinResponse};
use crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use robin_engine::multiplayer::{
    BrowserPeerAuth, MultiplayerSessionId, browser_seat_proof_message,
};
use robin_run_protocol::PublicKey32;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;
use wasm_bindgen::JsCast as _;

const OUTGOING_POLL: Duration = Duration::from_millis(4);
const RELAY_ONLINE_TIMEOUT: Duration = Duration::from_secs(15);
const HANDSHAKE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
const CONTENT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const CONTENT_DECISION_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CONTENT_TRANSFER_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const CONTENT_READINESS_TIMEOUT: Duration = Duration::from_secs(5 * 60);

mod native_only;
pub use native_only::{MultiplayerCampaignSession, ServerHandle, connect_client_in_campaign};

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
    let slots = ClientSlots::new();
    let (ranked_setup_tx, ranked_setup_rx) = ranked_setup_channel();
    wasm_bindgen_futures::spawn_local(run_client_io(
        ticket,
        ClientConfig {
            server_addr,
            nickname,
        },
        incoming_tx,
        outgoing_rx,
        slots.clone(),
        ranked_setup_rx,
    ));

    Ok(ClientHandle::new(
        slots,
        ranked_setup_tx,
        ranked_authenticated_host_public_key,
        None,
    ))
}

async fn run_client_io(
    ticket: BrowserJoinTicket,
    config: ClientConfig,
    incoming_tx: Sender<NetEvent>,
    mut outgoing_rx: Receiver<NetOutbound>,
    slots: ClientSlots,
    ranked_setup_rx: async_channel::Receiver<Option<OfficialRankedSessionSetupV1>>,
) {
    let transport_key = SecretKey::generate();
    let transport_endpoint_id = transport_key.public();
    let browser_auth = match browser_peer_auth(&ticket, transport_endpoint_id).await {
        Ok(auth) => auth,
        Err(error) => {
            publish_startup_error(&slots, &incoming_tx, error);
            return;
        }
    };
    slots.set_ranked_local_public_key(PublicKey32::from_bytes(browser_auth.durable_public_key));
    let endpoint = match Endpoint::builder(presets::N0)
        .secret_key(transport_key)
        .bind()
        .await
    {
        Ok(endpoint) => endpoint,
        Err(error) => {
            publish_startup_error(
                &slots,
                &incoming_tx,
                format!("start browser iroh endpoint: {error}"),
            );
            return;
        }
    };

    if client_session::with_timeout::<BrowserTimer, _>(RELAY_ONLINE_TIMEOUT, endpoint.online())
        .await
        .is_err()
    {
        publish_startup_error(
            &slots,
            &incoming_tx,
            "iroh relay did not become reachable within 15 seconds; browser multiplayer requires WebSocket relay access"
                .to_string(),
        );
        endpoint.close().await;
        return;
    }
    if let Err(error) = ticket.session_id() {
        publish_startup_error(&slots, &incoming_tx, error);
        endpoint.close().await;
        return;
    }
    let expected_session = match signed_invitation_session(&browser_auth) {
        Ok(session) => session,
        Err(error) => {
            publish_startup_error(&slots, &incoming_tx, error);
            endpoint.close().await;
            return;
        }
    };

    let ranked = BrowserRankedAdmission::new(
        Arc::clone(&slots.ranked_lifecycle),
        ranked_setup_rx,
        browser_auth.clone(),
        transport_endpoint_id,
        config.server_addr.id,
    );
    let transport = BrowserClientTransport {
        endpoint,
        config,
        browser_auth,
        expected_session,
        invitation_session_id: ticket.payload().session_id.as_str().to_owned(),
        cancellation: Arc::clone(&slots.cancellation),
    };
    client_session::run_client_io(&transport, &ranked, &mut outgoing_rx, incoming_tx, &slots).await;

    transport.endpoint.close().await;
}

fn signed_invitation_session(
    browser_auth: &BrowserPeerAuth,
) -> Result<MultiplayerSessionId, String> {
    Ok(MultiplayerSessionId(
        BrowserJoinTicket::decode_authenticated(&browser_auth.join_code)?.session_id()?,
    ))
}

fn publish_startup_error(slots: &ClientSlots, incoming_tx: &Sender<NetEvent>, error: String) {
    slots.set_startup_error(error.clone());
    let _ = incoming_tx.send(NetEvent::Fatal(error));
}

// ─── Transport ───────────────────────────────────────────────────

pub(super) struct BrowserTimer;

impl ClientTimer for BrowserTimer {
    fn sleep(duration: Duration) -> impl Future<Output = ()> {
        gloo_timers::future::sleep(duration)
    }
}

struct BrowserClientTransport {
    endpoint: Endpoint,
    config: ClientConfig,
    browser_auth: BrowserPeerAuth,
    /// Session of the signed invitation; every Welcome must belong to it.
    expected_session: MultiplayerSessionId,
    invitation_session_id: String,
    cancellation: Arc<AtomicBool>,
}

impl ClientTransport for BrowserClientTransport {
    type Timer = BrowserTimer;
    type Ranked = BrowserRankedAdmission;
    type Outbound = Receiver<NetOutbound>;

    const TIMINGS: ClientTimings = ClientTimings {
        handshake_frame: None,
        initial_attempt: HANDSHAKE_ATTEMPT_TIMEOUT,
        reconnect_attempt: None,
        post_content_welcome: CONTENT_IDLE_TIMEOUT,
        content_write: None,
        content_chunk_idle: CONTENT_IDLE_TIMEOUT,
        content_transfer: CONTENT_TRANSFER_TIMEOUT,
        content_decision: Some(CONTENT_DECISION_TIMEOUT),
        content_readiness: Some(CONTENT_READINESS_TIMEOUT),
    };
    const CANCELLED: &'static str = "browser multiplayer connection cancelled";

    fn cancellation(&self) -> &AtomicBool {
        &self.cancellation
    }

    fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    fn server_addr(&self) -> &EndpointAddr {
        &self.config.server_addr
    }

    fn hello(&self) -> NetMsg {
        NetMsg::Hello {
            protocol_version: NET_PROTOCOL_VERSION,
            nickname: self.config.nickname.clone(),
            browser_auth: Some(self.browser_auth.clone()),
            ranked_public_key: Some(self.browser_auth.durable_public_key),
        }
    }

    fn expected_session(&self) -> Option<MultiplayerSessionId> {
        Some(self.expected_session)
    }

    async fn recv_outbound(outbound: &mut Self::Outbound) -> Option<NetOutbound> {
        loop {
            match outbound.try_recv() {
                Ok(message) => return Some(message),
                Err(TryRecvError::Empty) => BrowserTimer::sleep(OUTGOING_POLL).await,
                Err(TryRecvError::Disconnected) => return None,
            }
        }
    }

    fn discard_outbound(outbound: &mut Self::Outbound) -> usize {
        let mut discarded = 0;
        while outbound.try_recv().is_ok() {
            discarded += 1;
        }
        discarded
    }

    /// Ranked responses go first; `ReadyToSim` is held until ranked admission
    /// resolves (admitted or browse-only).
    async fn next_writer_command(
        &self,
        outbound: &mut Self::Outbound,
        responses: &async_channel::Receiver<RankedJoinResponse>,
        ranked: &Self::Ranked,
        pending_ready: &mut Option<u32>,
    ) -> WriterCommand {
        loop {
            if self.cancellation.load(std::sync::atomic::Ordering::Acquire) {
                return WriterCommand::Closed;
            }
            if let Ok(response) = responses.try_recv() {
                return WriterCommand::RankedResponse(response);
            }
            if let Some(frame) = *pending_ready {
                if ranked.admission_resolved() {
                    *pending_ready = None;
                    return WriterCommand::Outbound(NetOutbound::ReadyToSim { frame });
                }
                BrowserTimer::sleep(OUTGOING_POLL).await;
                continue;
            }
            match outbound.try_recv() {
                Ok(NetOutbound::ReadyToSim { frame }) if !ranked.admission_resolved() => {
                    *pending_ready = Some(frame);
                }
                Ok(outgoing) => return WriterCommand::Outbound(outgoing),
                Err(TryRecvError::Empty) => BrowserTimer::sleep(OUTGOING_POLL).await,
                Err(TryRecvError::Disconnected) => return WriterCommand::Closed,
            }
        }
    }

    fn stream_closed() -> SessionEnd {
        SessionEnd::Drop("host closed the multiplayer stream".to_string())
    }

    fn fatal_outbound(outgoing: &NetOutbound) -> bool {
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

    fn initial_handshake_exhausted(last_error: String) -> String {
        format!(
            "could not reach the host through the iroh WebSocket relay within 15 seconds: {last_error}"
        )
    }

    /// The browser handle is returned before connecting; mission setup polls
    /// its slots instead of blocking on this progress.
    fn publish_initial_handshake(&self, progress: InitialHandshake) -> Result<(), ()> {
        match progress {
            InitialHandshake::Welcomed { seat, mission_seed } => {
                tracing::debug!(
                    ?seat,
                    seed = mission_seed,
                    "browser multiplayer client welcomed"
                );
            }
            InitialHandshake::ContentOffered { full_mod_sha256 } => tracing::debug!(
                full_mod_sha256 = %robin_engine::spellforge::hex_hash(&full_mod_sha256),
                "browser multiplayer client awaiting exact host-content admission"
            ),
        }
        Ok(())
    }

    fn startup_failed(
        &self,
        slots: &ClientSlots,
        incoming: &Sender<NetEvent>,
        _failure: StartupFailure,
        error: String,
    ) {
        publish_startup_error(slots, incoming, error);
    }

    async fn after_welcome(&self) -> Result<(), String> {
        mark_invitation_redeemed(&self.invitation_session_id).await
    }
}

// ─── Stable shell glue ───────────────────────────────────────────

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
