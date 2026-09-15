//! Native client: the iroh endpoint on a dedicated tokio thread. The session
//! state machine itself is shared with the browser in `client_session`.
use super::*;
use crate::multiplayer::MultiplayerError;
use crate::multiplayer::client_protocol::ClientConfig;
use crate::multiplayer::client_session::{
    self, ClientHandle, ClientSlots, ClientTimer, ClientTimings, ClientTransport, InitialHandshake,
    StartupFailure,
};
use robin_engine::multiplayer::NetFatal;
use std::future::Future;

// ─── Connect ─────────────────────────────────────────────────────

/// Connect to a multiplayer server and run the I/O thread.  `addr` is
/// the host's endpoint id (or a full endpoint-address connect string,
/// see [`parse_connect_addr`]).  Returns once the handshake
/// completes; the assigned seat is reported through `incoming_tx` as
/// a [`NetEvent::AssignedLocalSeat`].
///
/// This standalone entry point binds a fresh ephemeral campaign transport
/// identity.
pub fn connect_client(
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    connect_client_in_campaign(
        &MultiplayerCampaignSession::default(),
        addr,
        nickname,
        incoming_tx,
        outgoing_rx,
    )
}

pub fn connect_client_in_campaign(
    campaign: &MultiplayerCampaignSession,
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    connect_client_inner(
        campaign.state().client_key.clone(),
        addr,
        nickname,
        incoming_tx,
        outgoing_rx,
    )
}

/// Explicit-key client entry used by transport tests.
#[cfg(test)]
pub fn connect_client_with_key(
    key: SecretKey,
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    connect_client_inner(key, addr, nickname, incoming_tx, outgoing_rx)
}

pub(super) fn connect_client_inner(
    transport_key: SecretKey,
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    robin_engine::multiplayer::validate_display_name(&nickname).map_err(std::io::Error::other)?;
    let server_addr = parse_connect_addr(addr.as_ref()).map_err(std::io::Error::other)?;
    let addr_display = addr.as_ref().to_string();
    let slots = ClientSlots::new();
    let slots_for_thread = slots.clone();
    let cancellation = Arc::clone(&slots.cancellation);
    let cancellation_for_thread = Arc::clone(&cancellation);
    let (handshake_tx, handshake_rx) = std::sync::mpsc::sync_channel(1);
    let (bridge_thread, mut outgoing_async_rx) = spawn_outgoing_bridge(
        "mp-client-outgoing-bridge",
        outgoing_rx,
        Arc::clone(&cancellation),
    )?;
    let io_thread = thread::Builder::new()
        .name("mp-client".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = handshake_tx
                        .send(Err(MultiplayerError::transport("build tokio runtime", e)));
                    return;
                }
            };
            rt.block_on(async move {
                run_client_io_async(
                    transport_key,
                    ClientConfig {
                        server_addr,
                        nickname,
                    },
                    ClientIo {
                        incoming_tx,
                        outgoing_rx: &mut outgoing_async_rx,
                        slots: slots_for_thread,
                        initial_handshake_tx: handshake_tx,
                    },
                )
                .await;
            });
            cancellation_for_thread.store(true, Ordering::Release);
            if bridge_thread.join().is_err() {
                tracing::error!("multiplayer client outgoing bridge panicked");
            }
        })?;

    let initial = match handshake_rx.recv() {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => {
            cancellation.store(true, Ordering::Release);
            let _ = io_thread.join();
            return Err(std::io::Error::other(err.context("initial handshake")));
        }
        Err(e) => {
            cancellation.store(true, Ordering::Release);
            let _ = io_thread.join();
            return Err(std::io::Error::other(MultiplayerError::transport(
                "initial handshake channel closed",
                e,
            )));
        }
    };
    match initial {
        InitialHandshake::Welcomed { seat, mission_seed } => tracing::info!(
            addr = %addr_display,
            ?seat,
            seed = mission_seed,
            "multiplayer client connected"
        ),
        InitialHandshake::ContentOffered { full_mod_sha256 } => tracing::info!(
            addr = %addr_display,
            full_mod_sha256 = %robin_engine::spellforge::hex_hash(&full_mod_sha256),
            "multiplayer client awaiting exact host-content admission"
        ),
    }

    Ok(ClientHandle::new(
        slots,
        Some(Box::new(move || {
            if io_thread.join().is_err() {
                tracing::error!("multiplayer client worker panicked during shutdown");
            }
        })),
    ))
}

/// Channel ends and shared slots the native client I/O task uses to talk to
/// the game loop and its [`ClientHandle`]. The outbound receiver is borrowed:
/// its owner outlives the task so the outgoing bridge closes only after the
/// task has finished.
///
/// Not serde: live channel ends and shared slots.
pub(super) struct ClientIo<'a> {
    pub(super) incoming_tx: Sender<NetEvent>,
    pub(super) outgoing_rx: &'a mut UnboundedReceiver<NetOutbound>,
    pub(super) slots: ClientSlots,
    pub(super) initial_handshake_tx:
        std::sync::mpsc::SyncSender<Result<InitialHandshake, MultiplayerError>>,
}

/// Bind the endpoint and drive the shared client session on it until the
/// connection ends.
pub(super) async fn run_client_io_async(
    transport_key: SecretKey,
    config: ClientConfig,
    io: ClientIo<'_>,
) {
    let ClientIo {
        incoming_tx,
        outgoing_rx,
        slots,
        initial_handshake_tx,
    } = io;
    let endpoint = match bind_endpoint(transport_key, GAME_ALPN).await {
        Ok(endpoint) => endpoint,
        Err(e) => {
            let _ = initial_handshake_tx.send(Err(e));
            return;
        }
    };

    let transport = NativeClientTransport {
        endpoint: &endpoint,
        config,
        initial_handshake_tx,
        cancellation: Arc::clone(&slots.cancellation),
    };
    client_session::run_client_io(&transport, outgoing_rx, incoming_tx, &slots).await;

    endpoint.close().await;
}

// ─── Transport ───────────────────────────────────────────────────

pub(super) struct NativeTimer;

impl ClientTimer for NativeTimer {
    fn sleep(duration: Duration) -> impl Future<Output = ()> {
        tokio::time::sleep(duration)
    }
}

pub(super) struct NativeClientTransport<'a> {
    endpoint: &'a Endpoint,
    config: ClientConfig,
    /// Unblocks [`connect_client_inner`] once the first handshake resolves.
    initial_handshake_tx: std::sync::mpsc::SyncSender<Result<InitialHandshake, MultiplayerError>>,
    cancellation: Arc<AtomicBool>,
}

impl ClientTransport for NativeClientTransport<'_> {
    type Timer = NativeTimer;
    type Outbound = UnboundedReceiver<NetOutbound>;

    const TIMINGS: ClientTimings = ClientTimings {
        handshake_frame: Some(HANDSHAKE_FRAME_TIMEOUT),
        initial_attempt: HANDSHAKE_FRAME_TIMEOUT,
        reconnect_attempt: Some(HANDSHAKE_FRAME_TIMEOUT),
        post_content_welcome: HANDSHAKE_FRAME_TIMEOUT,
        content_write: Some(CONTENT_TRANSFER_IDLE_TIMEOUT),
        content_chunk_idle: CONTENT_TRANSFER_IDLE_TIMEOUT,
        content_transfer: CONTENT_DECISION_TIMEOUT,
        content_decision: None,
        content_readiness: None,
    };
    const CANCELLED: &'static str = "transport cancelled";

    fn cancellation(&self) -> &AtomicBool {
        &self.cancellation
    }

    fn endpoint(&self) -> &Endpoint {
        self.endpoint
    }

    fn server_addr(&self) -> &EndpointAddr {
        &self.config.server_addr
    }

    fn hello(&self) -> NetMsg {
        NetMsg::Hello {
            protocol_version: NET_PROTOCOL_VERSION,
            nickname: self.config.nickname.clone(),
            browser_auth: None,
        }
    }

    fn expected_session(&self) -> Option<MultiplayerSessionId> {
        None
    }

    async fn recv_outbound(outbound: &mut Self::Outbound) -> Option<NetOutbound> {
        outbound.recv().await
    }

    /// Throw away commands queued for a transport session whose prediction
    /// future has been abandoned. Replaying them after the next handshake
    /// would apply pre-disconnect input on top of the authoritative
    /// replacement snapshot.
    fn discard_outbound(outbound: &mut Self::Outbound) -> usize {
        let mut discarded = 0;
        while outbound.try_recv().is_ok() {
            discarded += 1;
        }
        discarded
    }

    fn initial_handshake_exhausted(last_error: MultiplayerError) -> MultiplayerError {
        last_error
    }

    fn publish_initial_handshake(&self, progress: InitialHandshake) -> Result<(), ()> {
        self.initial_handshake_tx.send(Ok(progress)).map_err(|_| ())
    }

    fn startup_failed(
        &self,
        slots: &ClientSlots,
        incoming: &Sender<NetEvent>,
        failure: StartupFailure,
        error: MultiplayerError,
    ) {
        slots.set_startup_error(error.clone());
        // Before content admission the blocked connect call reports the error;
        // once the offer was published the game hears it as a fatal event.
        if failure != StartupFailure::Admission {
            let _ = self.initial_handshake_tx.send(Err(error.clone()));
        }
        if failure != StartupFailure::Connect {
            let _ = incoming.send(NetEvent::Fatal(NetFatal::new(error)));
        }
    }

    async fn after_welcome(&self) -> Result<(), MultiplayerError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
