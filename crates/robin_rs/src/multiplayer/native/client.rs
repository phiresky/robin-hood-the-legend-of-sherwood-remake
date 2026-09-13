//! Native client: the iroh endpoint on a dedicated tokio thread and the
//! in-process ranked admission signed with the install's durable key. The
//! session state machine itself is shared with the browser in
//! `client_session`.
use super::*;
use crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1;
use crate::multiplayer::client_outgoing::ClientPublicationAuthority;
use crate::multiplayer::client_protocol::ClientConfig;
use crate::multiplayer::client_session::{
    self, ClientHandle, ClientRankedAdmission, ClientSlots, ClientTimer, ClientTimings,
    ClientTransport, InitialHandshake, RankedResponses, SessionLinks, StartupFailure,
    WriterCommand, ranked_setup_channel,
};
use crate::multiplayer::ranked_client::ClientRankedJoinState;
use crate::multiplayer::{MessageError, MultiplayerError};
use robin_engine::multiplayer::NetFatal;
use std::cell::Cell;
use std::future::Future;

// ─── Connect ─────────────────────────────────────────────────────

/// Connect to a multiplayer server and run the I/O thread.  `addr` is
/// the host's endpoint id (or a full endpoint-address connect string,
/// see [`parse_connect_addr`]).  Returns once the handshake
/// completes; the assigned seat is reported through `incoming_tx` as
/// a [`NetEvent::AssignedLocalSeat`].
///
/// This standalone entry point binds a fresh ephemeral iroh identity and keeps
/// the install's durable game identity separate for ranked attestation. A
/// durable-key storage failure never prevents otherwise-compatible gameplay;
/// that client joins without ranked authority and the session is downgraded by
/// the authenticated admission protocol.
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
    let durable_ranked_key = match game_secret_key() {
        Ok(key) => Some(key),
        Err(error) => {
            tracing::warn!(%error, "durable ranked identity unavailable; multiplayer gameplay will remain available browse-only");
            None
        }
    };
    connect_client_inner(
        ClientKeys {
            transport_key: campaign.state().client_key.clone(),
            durable_ranked_key,
        },
        addr,
        nickname,
        incoming_tx,
        outgoing_rx,
    )
}

/// Explicit transport/ranking identity seam for real-iroh tests. Keeping the
/// two keys independently injectable prevents tests from accidentally blessing
/// the transport endpoint as durable ranking authority.
#[cfg(test)]
pub(super) fn connect_client_with_keys(
    keys: ClientKeys,
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    connect_client_inner(keys, addr, nickname, incoming_tx, outgoing_rx)
}

/// Explicit-key client entry used by transport tests.
/// The injected key owns both the test transport and its ranked identity;
/// production still derives only the durable ranked key from install state.
#[cfg(test)]
pub fn connect_client_with_key(
    key: SecretKey,
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    connect_client_inner(
        ClientKeys {
            transport_key: key.clone(),
            durable_ranked_key: Some(key),
        },
        addr,
        nickname,
        incoming_tx,
        outgoing_rx,
    )
}

/// The two identities of a native client. The transport key names the QUIC
/// endpoint; the optional durable key is the install's ranked identity. They
/// stay separate so a transport key is never taken as ranking authority.
///
/// Not serde: secret key material.
pub(super) struct ClientKeys {
    pub(super) transport_key: SecretKey,
    pub(super) durable_ranked_key: Option<SecretKey>,
}

pub(super) fn connect_client_inner(
    keys: ClientKeys,
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    let ClientKeys {
        transport_key,
        durable_ranked_key,
    } = keys;
    robin_engine::multiplayer::validate_display_name(&nickname).map_err(std::io::Error::other)?;
    let server_addr = parse_connect_addr(addr.as_ref()).map_err(std::io::Error::other)?;
    let ranked_authenticated_host_public_key = PublicKey32::from_bytes(*server_addr.id.as_bytes());
    let ranked_local_public_key = durable_ranked_key
        .as_ref()
        .map(|key| PublicKey32::from_bytes(*key.public().as_bytes()));
    let addr_display = addr.as_ref().to_string();
    let slots = ClientSlots::new();
    if let Some(key) = ranked_local_public_key {
        slots.set_ranked_local_public_key(key);
    }
    let slots_for_thread = slots.clone();
    let (ranked_setup_tx, ranked_setup_rx) = ranked_setup_channel();
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
                    ClientKeys {
                        transport_key,
                        durable_ranked_key,
                    },
                    ClientConfig {
                        server_addr,
                        nickname,
                    },
                    ClientIo {
                        incoming_tx,
                        outgoing_rx: &mut outgoing_async_rx,
                        slots: slots_for_thread,
                        ranked_setup_rx,
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
        ranked_setup_tx,
        ranked_authenticated_host_public_key,
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
    pub(super) ranked_setup_rx: async_channel::Receiver<Option<OfficialRankedSessionSetupV1>>,
    pub(super) initial_handshake_tx:
        std::sync::mpsc::SyncSender<Result<InitialHandshake, MultiplayerError>>,
}

/// Bind the endpoint and drive the shared client session on it until the
/// connection ends.
pub(super) async fn run_client_io_async(keys: ClientKeys, config: ClientConfig, io: ClientIo<'_>) {
    let ClientKeys {
        transport_key,
        durable_ranked_key,
    } = keys;
    let ClientIo {
        incoming_tx,
        outgoing_rx,
        slots,
        ranked_setup_rx,
        initial_handshake_tx,
    } = io;
    let endpoint = match bind_endpoint(transport_key, GAME_ALPN).await {
        Ok(endpoint) => endpoint,
        Err(e) => {
            let _ = initial_handshake_tx.send(Err(e));
            return;
        }
    };

    let ranked_public_key = durable_ranked_key
        .as_ref()
        .map(|key| *key.public().as_bytes());
    let ranked = NativeRankedAdmission::new(
        Arc::clone(&slots.ranked_lifecycle),
        ranked_setup_rx,
        durable_ranked_key,
        *endpoint.id().as_bytes(),
        *config.server_addr.id.as_bytes(),
    );
    let transport = NativeClientTransport {
        endpoint: &endpoint,
        config,
        ranked_public_key,
        initial_handshake_tx,
        cancellation: Arc::clone(&slots.cancellation),
    };
    client_session::run_client_io(&transport, &ranked, outgoing_rx, incoming_tx, &slots).await;

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
    ranked_public_key: Option<[u8; 32]>,
    /// Unblocks [`connect_client_inner`] once the first handshake resolves.
    initial_handshake_tx: std::sync::mpsc::SyncSender<Result<InitialHandshake, MultiplayerError>>,
    cancellation: Arc<AtomicBool>,
}

impl ClientTransport for NativeClientTransport<'_> {
    type Timer = NativeTimer;
    type Ranked = NativeRankedAdmission;
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
            ranked_public_key: self.ranked_public_key,
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

    async fn next_writer_command(
        &self,
        outbound: &mut Self::Outbound,
        responses: &async_channel::Receiver<RankedJoinResponse>,
        ranked: &Self::Ranked,
        _pending_ready: &mut Option<u32>,
    ) -> WriterCommand {
        tokio::select! {
            outgoing = outbound.recv() => match outgoing {
                Some(outgoing) => WriterCommand::Outbound(outgoing),
                None => WriterCommand::Closed,
            },
            response = responses.recv() => match response {
                Ok(response) => WriterCommand::RankedResponse(response),
                Err(_) => WriterCommand::Fatal(MultiplayerError::ChannelClosed(
                    "ranked response queue closed while the client session is live".into(),
                )),
            },
            setup = ranked.setup_rx.recv() => match setup {
                Ok(setup) => WriterCommand::RankedSetup(setup),
                Err(_) => WriterCommand::Fatal(MultiplayerError::ChannelClosed(
                    "ranked setup channel closed while the client session is live".into(),
                )),
            },
        }
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

// ─── Ranked admission ────────────────────────────────────────────

/// In-process ranked admission: the prepared setup arrives through the session
/// writer, challenges are staged until it matches, and every trust failure
/// downgrades to browse-only instead of failing the session.
pub(super) struct NativeRankedAdmission {
    lifecycle: SharedRankedSessionLifecycle,
    join_state: SharedClientRankedJoinState,
    setup_state: AtomicU8,
    setup_rx: async_channel::Receiver<Option<OfficialRankedSessionSetupV1>>,
    durable_ranked_key: Option<SecretKey>,
    local_transport_endpoint: [u8; 32],
    authenticated_host_endpoint: [u8; 32],
    local_seat: Cell<Option<PlayerId>>,
}

impl NativeRankedAdmission {
    pub(super) fn new(
        lifecycle: SharedRankedSessionLifecycle,
        setup_rx: async_channel::Receiver<Option<OfficialRankedSessionSetupV1>>,
        durable_ranked_key: Option<SecretKey>,
        local_transport_endpoint: [u8; 32],
        authenticated_host_endpoint: [u8; 32],
    ) -> Self {
        Self {
            lifecycle,
            join_state: Arc::new(Default::default()),
            setup_state: AtomicU8::new(RANKED_SETUP_AWAITING),
            setup_rx,
            durable_ranked_key,
            local_transport_endpoint,
            authenticated_host_endpoint,
            local_seat: Cell::new(None),
        }
    }

    fn local_seat(&self) -> PlayerId {
        self.local_seat.get().expect(
            "native ranked admission runs only after the authoritative Welcome assigned a seat",
        )
    }

    /// Downgrade the ranked lifecycle after a typed local unavailability was
    /// answered; the host's browse-only decision resolves the join state.
    fn downgrade(&self, reason: RankedBrowseOnlyReason, detail: String) {
        ranked_lifecycle_lock(&self.lifecycle).downgrade(detail.clone());
        tracing::warn!(?reason, %detail, "client ranked admission downgraded; gameplay remains available");
    }

    fn respond_to_ranked_challenge(
        &self,
        responses: &RankedResponses,
        challenge: RankedJoinChallenge,
    ) -> Result<(), MultiplayerError> {
        let Some(durable_key) = self.durable_ranked_key.as_ref() else {
            return responses.queue(
                &self.join_state,
                RankedJoinResponse::Unavailable(
                    crate::multiplayer::RankedJoinUnavailableReason::DurableIdentityUnavailable,
                ),
            );
        };
        let claim: robin_run_protocol::NamedSeatJoinClaimV1 =
            decode_ranked_wire_document(challenge.join_claim.as_bytes()).map_err(|error| {
                MultiplayerError::ranked_document("invalid ranked join claim", error)
            })?;
        let genesis: robin_run_protocol::ReplaySessionGenesisV1 =
            decode_ranked_wire_document(challenge.session_genesis.as_bytes()).map_err(|error| {
                MultiplayerError::ranked_document("invalid ranked session genesis", error)
            })?;
        let expected_setup = {
            let lifecycle = ranked_lifecycle_lock(&self.lifecycle);
            lifecycle
                .client_admission()
                .map(|admission| admission.expected_setup.clone())
                .or_else(|| {
                    lifecycle
                        .ranked_client()
                        .map(|client| client.admission.expected_setup.clone())
                })
        }
        .ok_or_else(|| {
            MultiplayerError::Ranked(
                "ranked challenge arrived without locally installed client inputs".into(),
            )
        })?;
        crate::leaderboard_ranked_session::validate_official_session_genesis(
            &genesis,
            self.authenticated_host_endpoint,
            &expected_setup,
        )
        .map_err(|error| {
            MultiplayerError::ranked_document("ranked challenge host/session mismatch", error)
        })?;
        if claim.public_key != PublicKey32::from_bytes(*durable_key.public().as_bytes())
            || claim.transport_endpoint_id != PublicKey32::from_bytes(self.local_transport_endpoint)
            || claim.host_endpoint_id != PublicKey32::from_bytes(self.authenticated_host_endpoint)
            || claim.seat != u16::from(self.local_seat().0)
        {
            return Err(MultiplayerError::Identity(
                "ranked join claim does not bind this durable client transport/seat".into(),
            ));
        }
        let durable_signing_key = ed25519_dalek::SigningKey::from_bytes(&durable_key.to_bytes());
        let attestation = sign_named_seat_join(&durable_signing_key, claim).map_err(|error| {
            MultiplayerError::ranked_document("sign ranked named-seat claim", error)
        })?;
        let bytes = encode_ranked_wire_document(&attestation).map_err(|error| {
            MultiplayerError::ranked_document("encode ranked named-seat attestation", error)
        })?;
        let document = RankedJoinAttestationDocument::new(bytes).map_err(|error| {
            MultiplayerError::ranked_document(
                "encode ranked named-seat attestation",
                MessageError(error.to_owned()),
            )
        })?;
        responses.queue(&self.join_state, RankedJoinResponse::Attestation(document))
    }

    fn handle_delivered_ranked_challenge(
        &self,
        responses: &RankedResponses,
        challenge: RankedJoinChallenge,
    ) {
        if let Err(error) = self.respond_to_ranked_challenge(responses, challenge) {
            let unavailable = RankedJoinResponse::Unavailable(
                crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionMismatch,
            );
            if let Err(queue_error) = responses.queue(&self.join_state, unavailable) {
                tracing::warn!(%queue_error, "could not send typed ranked admission unavailability");
            }
            self.downgrade(
                RankedBrowseOnlyReason::PeerRankedSessionMismatch,
                error.to_string(),
            );
        }
    }

    fn accept_client_ranked_join(
        &self,
        accepted: RankedJoinAccepted,
    ) -> Result<RankedJoinAccepted, MultiplayerError> {
        let accepted = self.join_state.receive_wire_acceptance(accepted)?;
        let genesis: robin_run_protocol::ReplaySessionGenesisV1 =
            decode_ranked_wire_document(accepted.session_genesis.as_bytes()).map_err(|error| {
                MultiplayerError::ranked_document("invalid accepted ranked genesis", error)
            })?;
        let attestation: robin_run_protocol::NamedSeatJoinAttestationV1 =
            decode_ranked_wire_document(accepted.join_attestation.as_bytes()).map_err(|error| {
                MultiplayerError::ranked_document("invalid accepted ranked attestation", error)
            })?;
        let participant_claims =
            crate::multiplayer::ranked_client::decode_ranked_participant_roster(
                &accepted.participant_roster,
                &genesis,
            )?;
        let local_seat = self.local_seat();
        let mut lifecycle = ranked_lifecycle_lock(&self.lifecycle);
        if let Some(client) = lifecycle.ranked_client() {
            if client.session_genesis != genesis
                || client.local_seat != u16::from(local_seat.0)
                || attestation.claim.public_key != client.admission.local_public_key
                || attestation.claim.participant_instance_id
                    != client
                        .participant_claims
                        .iter()
                        .find(|participant| participant.seat == client.local_seat)
                        .ok_or_else(|| {
                            MultiplayerError::LocalState(
                                "ranked client roster lost its local participant".into(),
                            )
                        })?
                        .participant_instance_id
            {
                return Err(MultiplayerError::Ranked(
                    "ranked reconnect acknowledgement changed admitted client identity".into(),
                ));
            }
            lifecycle
                .update_ranked_client_roster(&genesis, participant_claims)
                .map_err(|error| {
                    MultiplayerError::ranked_document(
                        "update ranked client roster after reconnect",
                        error,
                    )
                })?;
            return Ok(accepted);
        }
        lifecycle
            .accept_ranked_client(u16::from(local_seat.0), genesis, participant_claims)
            .map_err(|error| {
                MultiplayerError::ranked_document("accept ranked client lifecycle", error)
            })?;
        Ok(accepted)
    }
}

impl ClientRankedAdmission for NativeRankedAdmission {
    const LABEL: &'static str = "native";
    const RESPONSE_QUEUE_CAPACITY: Option<usize> = None;

    fn lifecycle(&self) -> &SharedRankedSessionLifecycle {
        &self.lifecycle
    }

    fn join_state(&self) -> &ClientRankedJoinState {
        &self.join_state
    }

    fn durable_public_key(&self) -> Option<PublicKey32> {
        self.durable_ranked_key
            .as_ref()
            .map(|key| PublicKey32::from_bytes(*key.public().as_bytes()))
    }

    fn authenticated_host_public_key(&self) -> PublicKey32 {
        PublicKey32::from_bytes(self.authenticated_host_endpoint)
    }

    fn welcomed(&self, seat: PlayerId) {
        self.local_seat.set(Some(seat));
    }

    fn simulation_release_unresolved(&self) -> Result<bool, MultiplayerError> {
        Ok(!self.join_state.is_accepted()?
            && ranked_lifecycle_lock(&self.lifecycle)
                .browse_only_reason()
                .is_none())
    }

    fn publication_authority(
        &self,
        requires_cosign: bool,
    ) -> Result<ClientPublicationAuthority, MultiplayerError> {
        Ok(ClientPublicationAuthority {
            co_sign_allowed: requires_cosign
                && ranked_lifecycle_lock(&self.lifecycle)
                    .ranked_client()
                    .is_some(),
            durable_public_key: self.durable_public_key(),
        })
    }

    async fn on_challenge<Tm: ClientTimer>(
        &self,
        links: &SessionLinks<'_, Self>,
        challenge: RankedJoinChallenge,
    ) -> Result<(), MultiplayerError> {
        let responses = links.responses;
        // An invalid, replayed or replacing challenge is returned to the
        // shared handler, which applies the one browse-only policy.
        match self.join_state.receive_wire_challenge(challenge)? {
            Some(challenge)
                if self.setup_state.load(Ordering::Acquire) == RANKED_SETUP_AVAILABLE =>
            {
                self.handle_delivered_ranked_challenge(responses, challenge);
            }
            _ if self.setup_state.load(Ordering::Acquire) == RANKED_SETUP_UNAVAILABLE => {
                if let Err(error) = responses.queue(
                    &self.join_state,
                    RankedJoinResponse::Unavailable(
                        crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionUnavailable,
                    ),
                ) {
                    tracing::warn!(%error, "could not answer ranked challenge with typed unavailability");
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn on_setup(&self, responses: &RankedResponses, setup: Option<OfficialRankedSessionSetupV1>) {
        let Some(setup) = setup else {
            self.setup_state
                .store(RANKED_SETUP_UNAVAILABLE, Ordering::Release);
            let _ = responses.queue(
                &self.join_state,
                RankedJoinResponse::Unavailable(
                    crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionUnavailable,
                ),
            );
            self.downgrade(
                RankedBrowseOnlyReason::PeerRankedSessionMismatch,
                "local prepared inputs explicitly selected browse-only multiplayer".to_string(),
            );
            return;
        };
        let expected = setup.ranked_session.clone();
        let official_subject =
            robin_run_protocol::official_content_subjects_v1(expected.content_edition)
                .contains(&expected.content_subject);
        let document = encode_ranked_wire_document(&expected)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                crate::multiplayer::RankedSessionConfigDocument::new(bytes).map_err(str::to_string)
            });
        let admission = self.durable_ranked_key.as_ref().map(|key| {
            RankedSessionClientAdmissionV1::new_official(
                setup,
                PublicKey32::from_bytes(self.authenticated_host_endpoint),
                PublicKey32::from_bytes(*key.public().as_bytes()),
                PublicKey32::from_bytes(self.local_transport_endpoint),
            )
        });
        let (document, admission) = match (official_subject, document, admission) {
            (true, Ok(document), Some(Ok(admission))) => (document, admission),
            (true, Ok(document), None) => {
                self.setup_state
                    .store(RANKED_SETUP_UNAVAILABLE, Ordering::Release);
                if let Ok(Some(challenge)) = self.join_state.arm_expected_session(document) {
                    self.handle_delivered_ranked_challenge(responses, challenge);
                }
                self.downgrade(
                    RankedBrowseOnlyReason::PeerIdentityUnavailable,
                    "durable ranked identity is unavailable".to_string(),
                );
                return;
            }
            (_, document, admission) => {
                self.setup_state
                    .store(RANKED_SETUP_UNAVAILABLE, Ordering::Release);
                let detail = format!(
                    "official ranked client setup failed: subject_official={official_subject}, document={:?}, admission={:?}",
                    document.err(),
                    admission.and_then(Result::err)
                );
                let _ = responses.queue(
                    &self.join_state,
                    RankedJoinResponse::Unavailable(
                        crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionMismatch,
                    ),
                );
                self.downgrade(RankedBrowseOnlyReason::PeerRankedSessionMismatch, detail);
                return;
            }
        };
        if let Err(error) =
            ranked_lifecycle_lock(&self.lifecycle).install_client_admission(admission)
        {
            self.setup_state
                .store(RANKED_SETUP_UNAVAILABLE, Ordering::Release);
            self.downgrade(
                RankedBrowseOnlyReason::RankedProtocolViolation,
                format!("could not install ranked client admission: {error}"),
            );
            return;
        }
        self.setup_state
            .store(RANKED_SETUP_AVAILABLE, Ordering::Release);
        match self.join_state.arm_expected_session(document) {
            Ok(Some(challenge)) => self.handle_delivered_ranked_challenge(responses, challenge),
            Ok(None) => {}
            Err(error) => self.downgrade(
                RankedBrowseOnlyReason::RankedProtocolViolation,
                error.to_string(),
            ),
        }
    }

    fn on_accepted(
        &self,
        links: &SessionLinks<'_, Self>,
        accepted: RankedJoinAccepted,
    ) -> Result<(), MultiplayerError> {
        let accepted = self.accept_client_ranked_join(accepted)?;
        links
            .incoming
            .send(NetEvent::RankedJoinAccepted(accepted))
            .map_err(|_| {
                MultiplayerError::ChannelClosed(
                    "client ranked acknowledgement channel is closed".into(),
                )
            })
    }
}

#[cfg(test)]
mod tests;
