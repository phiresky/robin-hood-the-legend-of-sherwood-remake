//! Native iroh (peer-to-peer QUIC) server / client for the
//! multiplayer transport.  Each external function spawns one OS
//! thread that owns a tokio runtime driving the iroh endpoint; the
//! game loop talks to it through [`super::NetChannels`].
//!
//! Peers are addressed by iroh endpoint id (a public key), not by
//! host:port.  Connectivity — hole punching, relay fallback, address
//! lookup — is handled entirely by iroh, so hosting needs no port
//! forwarding and no bind-address configuration.
//!
//! Each session runs over a single bidirectional QUIC stream per
//! peer, carrying length-prefixed [`NetMsg`] frames.  The joining
//! side opens the stream and sends `Hello`; the host answers
//! `Welcome` on the same stream.

mod peer_sessions;
use peer_sessions::PeerSessions;

mod server_dispatch;
#[cfg(test)]
use server_dispatch::validate_server_gameplay_outbound;
use server_dispatch::{announce_begin_sim, broadcast_input, run_server_outgoing_pump};
mod server_protocol;
use server_protocol::{
    AdmissionDeadline, CoSignTracker, PendingSnapshotTransition, RankedAdmissionTracker,
    ReadyBarrier, SnapshotTransitions,
};

use super::client_protocol::{ClientSessionMetadata, WelcomeData, validate_reconnect_state};
#[cfg(test)]
use super::encode_msg;
use super::identity::{
    GAME_ALPN, bind_endpoint, bind_endpoint_with_relay, game_secret_key, parse_connect_addr,
};
use super::{
    FrameCursor, INPUT_DELAY_FRAMES, InboundFramePolicy, InitialSnapshot,
    MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION, MultiplayerSessionId, NET_PROTOCOL_VERSION,
    NetEvent, NetMsg, NetOutbound, RankedBrowseOnlyReason, RankedJoinAccepted,
    RankedJoinAttestationDocument, RankedJoinChallenge, RankedJoinClaimDocument,
    RankedJoinResponse, RankedParticipantRosterDocument, RankedSessionGenesisDocument,
    SharedClientLeaderboardCoSignState, SharedClientRankedJoinState,
    verify_leaderboard_cosign_response,
};
use crate::distributed_mod::{
    DistributedModPackage, ValidatedDistributedMod, make_distributed_mod_offer,
};
use crate::leaderboard_ranked_session::{
    CampaignContinuationReceiptSelectionRequestV1, CampaignContinuationReceiptSelectionResponseV1,
    OfficialRankedSessionWireSetupV1, RankedSessionClientAdmissionV1, RankedSessionLifecycle,
    SharedRankedSessionLifecycle, decode_ranked_wire_document, encode_ranked_wire_document,
    sign_named_seat_join,
};
use iroh::endpoint::{Connection, ReadExactError, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
// Non-poisoning mutex: a panicking worker must not turn every later
// lock of the shared peer state into a second panic.
#[cfg(test)]
use super::clock::checked_epoch_ms;
use super::clock::try_current_epoch_ms as current_epoch_ms;
use parking_lot::Mutex;
use robin_engine::multiplayer::{
    BrowserPeerAuth, LeaderboardCoSignResponse, browser_seat_proof_message,
};
use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
use robin_run_protocol::{
    CampaignContinuationPreflightRequestClaimV1, LeaderboardCoSignInstanceV1,
    LeaderboardCoSignRequestV1, ParticipantPublicDisclosureV1, ParticipantSignatureV1, PublicKey32,
    Validate as _,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(20);
const RANKED_ADMISSION_TIMEOUT: Duration = Duration::from_secs(30);
const RANKED_SETUP_AWAITING: u8 = 0;
const RANKED_SETUP_AVAILABLE: u8 = 1;
const RANKED_SETUP_UNAVAILABLE: u8 = 2;

const HANDSHAKE_FRAME_TIMEOUT: Duration = Duration::from_secs(15);
/// Finish queued reconnect/commit frames after reader authority is detached.
const TERMINAL_WRITER_DRAIN_TIMEOUT: Duration = Duration::from_secs(15);
const CONTENT_TRANSFER_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const CONTENT_DECISION_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CONTENT_READINESS_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// QUIC close code used for orderly application shutdown.
const CLOSE_GRACEFUL: u32 = 0;

/// Canonical validated package a host distributes before Welcome/snapshot.
#[derive(Clone, Debug)]
pub struct HostedModContent {
    validated: ValidatedDistributedMod,
    encoded: Arc<[u8]>,
}

impl HostedModContent {
    pub fn from_encoded(encoded: Vec<u8>) -> Result<Self, String> {
        let validated = DistributedModPackage::decode(&encoded)
            .map_err(|error| format!("validate hosted distributed mod: {error}"))?;
        Ok(Self {
            validated,
            encoded: Arc::from(encoded),
        })
    }

    fn offer(
        &self,
        host_endpoint_id: String,
    ) -> Result<robin_engine::multiplayer::DistributedModOffer, String> {
        make_distributed_mod_offer(&self.validated, self.encoded.len() as u64, host_endpoint_id)
            .map_err(|error| format!("build distributed-mod offer: {error}"))
    }
}

/// Explicit campaign lifetime, independent of each mission's QUIC endpoint.
/// Transport credentials are deliberately neither persisted nor reconstructed
/// by deserialization. Durable ranked identity remains install-owned.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct MultiplayerCampaignSession {
    #[serde(skip)]
    state: Option<Arc<CampaignTransportState>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CampaignTransportState {
    #[serde(skip, default = "SecretKey::generate")]
    client_key: SecretKey,
    #[serde(skip)]
    continuation: Mutex<Option<HostSessionContinuation>>,
    #[serde(skip)]
    server_active: AtomicBool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CampaignServerLease {
    #[serde(skip)]
    state: Option<Arc<CampaignTransportState>>,
}

impl Drop for CampaignServerLease {
    fn drop(&mut self) {
        if let Some(state) = &self.state {
            state.server_active.store(false, Ordering::Release);
        }
    }
}

impl Default for MultiplayerCampaignSession {
    fn default() -> Self {
        Self {
            state: Some(Arc::new(CampaignTransportState {
                client_key: SecretKey::generate(),
                continuation: Mutex::new(None),
                server_active: AtomicBool::new(false),
            })),
        }
    }
}

impl MultiplayerCampaignSession {
    fn state(&self) -> &Arc<CampaignTransportState> {
        self.state
            .as_ref()
            .expect("decoded campaign has no live multiplayer authority")
    }

    pub(crate) fn discard_host_continuation(&self) -> Result<(), String> {
        let _lease = self.reserve_server().map_err(|error| error.to_string())?;
        *self.state().continuation.lock() = None;
        Ok(())
    }

    fn reserve_server(&self) -> std::io::Result<CampaignServerLease> {
        self.state()
            .server_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                std::io::Error::other("campaign already owns an active mission transport")
            })?;
        Ok(CampaignServerLease {
            state: Some(Arc::clone(self.state())),
        })
    }
}

/// One-shot campaign-local handoff between the old and replacement mission
/// transports. The outer campaign loop deliberately destroys each QUIC
/// endpoint at a load/restart boundary, but the authenticated session and its
/// seat ownership must survive that implementation detail.
#[derive(Clone)]
struct HostSessionContinuation {
    host_endpoint_id: EndpointId,
    session_id: MultiplayerSessionId,
    expected_players: u32,
    owner_seats: HashMap<PeerOwner, u8>,
    relay_url: Option<iroh::RelayUrl>,
}

fn publish_host_session_continuation(
    state: &CampaignTransportState,
    continuation: HostSessionContinuation,
) {
    let mut slot = state.continuation.lock();
    if let Some(existing) = slot.as_mut()
        && existing.host_endpoint_id == continuation.host_endpoint_id
        && existing.session_id == continuation.session_id
    {
        existing.owner_seats.extend(continuation.owner_seats);
        return;
    }
    assert!(
        slot.is_none(),
        "another multiplayer continuation is pending"
    );
    *slot = Some(continuation);
}

fn pending_host_session_continuation(
    state: &CampaignTransportState,
    host_endpoint_id: EndpointId,
    expected_players: u32,
) -> Result<Option<HostSessionContinuation>, String> {
    let slot = state.continuation.lock();
    let Some(continuation) = slot.as_ref() else {
        return Ok(None);
    };
    if continuation.host_endpoint_id != host_endpoint_id {
        return Err("pending multiplayer continuation belongs to another host identity".into());
    }
    if continuation.expected_players != expected_players {
        return Err(format!(
            "continued multiplayer session expects {} players, replacement requested {expected_players}",
            continuation.expected_players
        ));
    }
    Ok(slot.clone())
}

// ─── Framing ─────────────────────────────────────────────────────

async fn write_frame(send: &mut SendStream, msg: &NetMsg) -> Result<(), String> {
    let (header, bytes) = super::client_protocol::encode_frame(msg)?;
    send.write_all(&header)
        .await
        .map_err(|e| format!("write frame header: {e}"))?;
    send.write_all(&bytes)
        .await
        .map_err(|e| format!("write frame body: {e}"))?;
    Ok(())
}

/// Read one frame.  `Ok(None)` means the stream finished cleanly at a
/// frame boundary (graceful close).
async fn read_frame(
    recv: &mut RecvStream,
    policy: InboundFramePolicy,
) -> Result<Option<NetMsg>, String> {
    let mut header = [0u8; 5];
    match recv.read_exact(&mut header).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(format!("read frame header: {e}")),
    }
    let (class, len) = super::client_protocol::decode_header(header, policy)?;
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| format!("read frame body: {e}"))?;
    super::client_protocol::decode_body(class, &buf).map(Some)
}

async fn read_frame_bounded_with_timeout(
    recv: &mut RecvStream,
    policy: InboundFramePolicy,
    timeout: Duration,
    phase: &str,
) -> Result<Option<NetMsg>, String> {
    tokio::time::timeout(timeout, read_frame(recv, policy))
        .await
        .map_err(|_| format!("{phase} timed out after {timeout:?}"))?
}

async fn write_frame_with_timeout(
    send: &mut SendStream,
    msg: &NetMsg,
    timeout: Duration,
    phase: &str,
) -> Result<(), String> {
    tokio::time::timeout(timeout, write_frame(send, msg))
        .await
        .map_err(|_| format!("{phase} timed out after {timeout:?}"))?
}

/// Bridge a std mpsc receiver (game loop side) onto a tokio unbounded
/// channel so async code can `select!` on it.  The bridge thread
/// exits when cancellation flips or either channel closes.
fn spawn_outgoing_bridge(
    name: &str,
    outgoing_rx: Receiver<NetOutbound>,
    cancellation: Arc<AtomicBool>,
) -> std::io::Result<(JoinHandle<()>, UnboundedReceiver<NetOutbound>)> {
    let (tx, rx) = unbounded_channel::<NetOutbound>();
    let handle = thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            while !cancellation.load(Ordering::Acquire) {
                let msg = match outgoing_rx.recv_timeout(WORKER_POLL_INTERVAL) {
                    Ok(msg) => msg,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                if tx.send(msg).is_err() {
                    break;
                }
            }
        })?;
    Ok((handle, rx))
}

mod server;
pub use server::*;
mod client;
pub use client::*;

#[cfg(test)]
mod tests {
    #[test]
    fn begin_sim_requires_a_live_local_receiver() {
        let (tx, rx) = std::sync::mpsc::channel();
        let state = std::sync::Arc::new(Default::default());
        let begin = || super::NetMsg::BeginSim {
            frame: 9,
            start_epoch_ms: 12,
        };
        super::handle_client_wire_msg(&tx, &state, None, begin()).unwrap();
        assert!(matches!(
            rx.try_recv().unwrap(),
            super::NetEvent::BeginSim {
                frame: 9,
                start_epoch_ms: 12
            }
        ));
        drop(rx);
        assert!(
            super::handle_client_wire_msg(&tx, &state, None, begin())
                .unwrap_err()
                .contains("channel is closed")
        );
    }

    use super::{
        HostSessionContinuation, PeerOwner, PendingSnapshotTransition, SeatClaimKind, ServerPeers,
        SharedClientLeaderboardCoSignState, checked_epoch_ms, client_gameplay_wire_msg,
        connect_client_with_keys, discard_session_outbound, handle_client_wire_msg,
        retain_transition_peer_for_reconnect, start_server_with_key,
        take_committed_snapshot_transition, validate_peer_command_authority,
        validate_reconnect_state, validate_server_gameplay_outbound,
        validate_server_gameplay_wire_msg,
    };
    use crate::leaderboard_ranked_session::{
        OfficialRankedSessionSetupV1, RankedRunPreflightAdmissionV1,
    };
    use crate::multiplayer::{MAX_CONTENT_FRAME_BYTES, MAX_SERVER_CONTROL_FRAME_BYTES};
    use ed25519_dalek::{Signer, SigningKey};
    use robin_engine::multiplayer::LeaderboardCoSignResponse;
    use robin_engine::multiplayer::{NetEvent, NetOutbound};
    use robin_engine::player_command::PlayerId;
    use robin_run_protocol::{
        ArtifactRefV1, CanonicalDocument as _, ChallengeNonce32, Digest32,
        FreshRunPreflightGrantClaimV1, FreshRunPreflightGrantV1, FreshRunPreflightRequestClaimV1,
        FreshRunPreflightRequestV1, FreshRunScopeV1, LeaderboardCoSignInstanceV1,
        LeaderboardCoSignPurposeV1, LeaderboardCoSignRequestV1, OfficialContentEditionV1,
        OfficialContentSubjectV1, OpaqueId, PublicKey32, RANKED_CAMPAIGN_MEDIA_TYPE_V1,
        RankedSessionConfigV1, ResourceLocaleRootV1, SCHEMA_VERSION_V1, Signature64,
        SignatureAlgorithmV1, SimulationSeed64, SpeechTimingAuthorityV1, SubmissionAcceptedV1,
        SubmissionLifecycleV1,
    };
    use std::collections::HashSet;
    use std::sync::atomic::AtomicU32;
    use std::sync::mpsc::{Receiver, channel};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc::unbounded_channel;

    // No socket/runtime is needed: tests drive the exact synchronous dispatcher
    // called by run_server_peer_reader after bounded frame decoding.
    fn dispatch_test_context() -> (super::ServerContext, Receiver<NetEvent>) {
        let campaign = super::MultiplayerCampaignSession::default();
        let (incoming_tx, incoming_rx) = channel();
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        let context = super::ServerContext {
            _campaign_lease: campaign.reserve_server().unwrap(),
            campaign: Arc::clone(campaign.state()),
            session_dispatch: super::Mutex::new(()),
            peers: super::Mutex::new(ServerPeers::new(2)),
            incoming_tx,
            host_nickname: "host".into(),
            mission_id: "Dem_Lei_MP".into(),
            mission_seed: 7,
            sim_config: Default::default(),
            host_endpoint_id: iroh::SecretKey::from_bytes(&[9; 32]).public(),
            session_id: super::MultiplayerSessionId([4; 32]),
            ranked_lifecycle: Arc::new(StdMutex::new(
                super::RankedSessionLifecycle::awaiting_prepared_inputs(),
            )),
            ranked_browse_reason: super::Mutex::new(None),
            continued_session: false,
            relay_url: super::Mutex::new(None),
            speech_timing_locale: None,
            frame_cursor: Arc::new(AtomicU32::new(10)),
            initial_snapshot: Arc::new(StdMutex::new(None)),
            content: None,
            cancellation: Arc::new(super::AtomicBool::new(false)),
            shutdown_tx,
        };
        (context, incoming_rx)
    }

    #[test]
    fn fatal_server_failure_cancels_and_notifies_once() {
        let (context, events) = dispatch_test_context();
        let shutdown = context.shutdown_tx.subscribe();
        super::fail_server(&context, "first failure".into());
        super::fail_server(&context, "later failure".into());
        assert!(context.cancellation.load(super::Ordering::Acquire));
        assert!(*shutdown.borrow());
        assert!(
            matches!(events.try_recv(), Ok(NetEvent::Fatal(message)) if message == "first failure")
        );
        assert!(matches!(
            events.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
    }

    fn admission_challenge(
        seat: u8,
        generation: u64,
        deadline: Instant,
    ) -> super::PendingRankedAdmission {
        super::PendingRankedAdmission {
            seat,
            generation,
            kind: super::RankedAdmissionKind::Fresh,
            challenge: super::RankedJoinChallenge {
                session_genesis: super::RankedSessionGenesisDocument::new(vec![1]).unwrap(),
                join_claim: super::RankedJoinClaimDocument::new(vec![2]).unwrap(),
            },
            deadline,
        }
    }

    #[test]
    fn stale_peer_dispatch_rejects_every_effect_before_touching_successor_state() {
        use robin_engine::multiplayer::{
            ModalInstanceId, NetMsg, RankedContinuationPreflightSignatureDocument,
            RankedContinuationReceiptSelectionDocument, RankedJoinUnavailableReason,
            SnapshotTransitionId, SnapshotTransitionPayload,
        };
        use robin_engine::player_command::{DialogResult, ModalKind};
        for invalidation in ["replacement", "detachment", "release"] {
            let (context, events) = dispatch_test_context();
            let owner = PeerOwner::Native([1; 32]);
            let key = iroh::SecretKey::from_bytes(&[2; 32]);
            let mut identity = ranked_identity(1);
            identity.durable_public_key = Some(*key.public().as_bytes());
            let (sender, mut old_wire) = unbounded_channel();
            let first = context
                .peers
                .lock()
                .sessions
                .claim_seat(owner, "first", identity, sender)
                .unwrap();
            let (replacement_sender, mut replacement_wire) = unbounded_channel();
            let generation = {
                let mut peers = context.peers.lock();
                match invalidation {
                    "replacement" => {
                        peers
                            .sessions
                            .claim_seat(owner, "next", identity, replacement_sender)
                            .unwrap()
                            .generation
                    }
                    "detachment" => {
                        peers.sessions.detach_writer(&first.seat).unwrap();
                        first.generation
                    }
                    "release" => {
                        assert_eq!(
                            peers.sessions.release_seat_if_owner(
                                first.seat,
                                owner,
                                first.generation
                            ),
                            Some(false)
                        );
                        first.generation
                    }
                    _ => unreachable!(),
                }
            };
            let id = SnapshotTransitionId {
                session_id: context.session_id,
                sequence: 1,
            };
            let request = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 90);
            {
                let mut peers = context.peers.lock();
                peers.transitions.begin(PendingSnapshotTransition {
                    id,
                    payload: SnapshotTransitionPayload::Save {
                        mission_id: 7,
                        save_bytes: vec![1],
                    },
                    awaiting: HashSet::from([first.seat]),
                });
                // Pure tracker setup ensures even detached/released denial cannot
                // accidentally consume a still-pending session-level request.
                peers.cosigns.begin(PlayerId(first.seat), request).unwrap();
                peers.admission.begin(admission_challenge(
                    first.seat,
                    generation,
                    Instant::now() + Duration::from_secs(30),
                ));
            }
            let messages = [
                (
                    "input",
                    NetMsg::Input {
                        origin_frame: 10,
                        command: super::PlayerCommand::Noop,
                    },
                ),
                ("ready", NetMsg::ReadyToSim { frame: 99 }),
                ("snapshot ack", NetMsg::SnapshotTransitionReady { id }),
                (
                    "co-sign",
                    NetMsg::LeaderboardCoSignResponse(signed_response(&request, &key)),
                ),
                (
                    "ranked unavailable",
                    NetMsg::RankedJoinResponse(super::RankedJoinResponse::Unavailable(
                        RankedJoinUnavailableReason::DurableIdentityUnavailable,
                    )),
                ),
                (
                    "ranked attestation",
                    NetMsg::RankedJoinResponse(super::RankedJoinResponse::Attestation(
                        super::RankedJoinAttestationDocument::new(vec![1]).unwrap(),
                    )),
                ),
                (
                    "receipt selection",
                    NetMsg::RankedContinuationReceiptSelection(
                        RankedContinuationReceiptSelectionDocument::new(vec![1]).unwrap(),
                    ),
                ),
                (
                    "preflight signature",
                    NetMsg::RankedContinuationPreflightSignature(
                        RankedContinuationPreflightSignatureDocument::new(vec![1]).unwrap(),
                    ),
                ),
                (
                    "modal proposal",
                    NetMsg::ModalProposal {
                        instance: ModalInstanceId {
                            session_id: context.session_id,
                            opened_frame: 10,
                            occurrence: 1,
                        },
                        kind: ModalKind::Dialog { dialog_id: 44 },
                        result: DialogResult::Completed,
                        requested_frame: 10,
                    },
                ),
            ];
            for (label, message) in messages {
                let error = super::dispatch_server_peer_message(
                    &context,
                    PlayerId(first.seat),
                    first.generation,
                    identity,
                    message,
                )
                .unwrap_err();
                assert!(
                    error.to_string().contains("generation"),
                    "{invalidation} {label}: {error}"
                );
                assert!(
                    events.try_recv().is_err(),
                    "{invalidation} {label} published a host event"
                );
                assert!(
                    old_wire.try_recv().is_err(),
                    "{invalidation} {label} published to old writer"
                );
                assert!(
                    replacement_wire.try_recv().is_err(),
                    "{invalidation} {label} published to successor"
                );
                let peers = context.peers.lock();
                assert!(
                    peers
                        .transitions
                        .pending()
                        .unwrap()
                        .awaiting
                        .contains(&first.seat)
                );
                assert_eq!(peers.cosigns.pending_count(), 1);
                assert_eq!(peers.admission.pending().unwrap().generation, generation);
                assert!(
                    peers
                        .sessions
                        .readiness()
                        .all(|(_, _, ready_frame)| ready_frame.is_none())
                );
                assert!(peers.readiness.begun.is_none());
                drop(peers);
                assert!(
                    super::ranked_lifecycle_lock(&context.ranked_lifecycle)
                        .is_awaiting_prepared_inputs(),
                    "{invalidation} {label} downgraded ranked lifecycle"
                );
            }
        }
    }

    #[test]
    fn current_peer_dispatch_publishes_input_and_ready_but_old_generation_cannot() {
        let (context, events) = dispatch_test_context();
        // Explicit browse-only resolution is host policy, unrelated to freshness.
        super::ranked_lifecycle_lock(&context.ranked_lifecycle).downgrade("test browse session");
        let (sender, mut wire) = unbounded_channel();
        let claim = {
            let mut peers = context.peers.lock();
            let claim = peers
                .sessions
                .claim_seat(
                    PeerOwner::Native([1; 32]),
                    "peer",
                    ranked_identity(1),
                    sender,
                )
                .unwrap();
            peers.sessions.connect_sim_seat(claim.seat);
            peers.readiness.host_frame = Some(10);
            claim
        };
        super::dispatch_server_peer_message(
            &context,
            PlayerId(claim.seat),
            claim.generation,
            ranked_identity(1),
            super::NetMsg::Input {
                origin_frame: 10,
                command: super::PlayerCommand::Noop,
            },
        )
        .unwrap();
        assert!(
            matches!(events.try_recv().unwrap(), NetEvent::Input { input, .. } if input.player_id == PlayerId(1))
        );
        assert!(matches!(
            wire.try_recv().unwrap(),
            super::NetMsg::BroadcastInput { .. }
        ));
        super::dispatch_server_peer_message(
            &context,
            PlayerId(claim.seat),
            claim.generation,
            ranked_identity(1),
            super::NetMsg::ReadyToSim { frame: 12 },
        )
        .unwrap();
        assert!(matches!(
            events.try_recv().unwrap(),
            NetEvent::BeginSim { frame: 12, .. }
        ));
        assert!(matches!(
            wire.try_recv().unwrap(),
            super::NetMsg::BeginSim { frame: 12, .. }
        ));
        assert_eq!(
            context.peers.lock().sessions.ready_frame(claim.seat),
            Some(12)
        );
    }

    #[test]
    fn current_snapshot_ack_commits_once_and_detaches_authority() {
        let (context, events) = dispatch_test_context();
        let (sender, mut wire) = unbounded_channel();
        let claim = context
            .peers
            .lock()
            .sessions
            .claim_seat(
                PeerOwner::Native([1; 32]),
                "peer",
                ranked_identity(1),
                sender,
            )
            .unwrap();
        let id = robin_engine::multiplayer::SnapshotTransitionId {
            session_id: context.session_id,
            sequence: 1,
        };
        context
            .peers
            .lock()
            .transitions
            .begin(PendingSnapshotTransition {
                id,
                payload: robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                    mission_id: 7,
                    save_bytes: vec![1],
                },
                awaiting: HashSet::from([claim.seat]),
            });
        super::dispatch_server_peer_message(
            &context,
            PlayerId(claim.seat),
            claim.generation,
            ranked_identity(1),
            super::NetMsg::SnapshotTransitionReady { id },
        )
        .unwrap();
        assert!(
            matches!(wire.try_recv().unwrap(), super::NetMsg::CommitSnapshotTransition { id: actual } if actual == id)
        );
        assert!(
            matches!(events.try_recv().unwrap(), NetEvent::CommitSnapshotTransition { id: actual } if actual == id)
        );
        let error = super::dispatch_server_peer_message(
            &context,
            PlayerId(claim.seat),
            claim.generation,
            ranked_identity(1),
            super::NetMsg::SnapshotTransitionReady { id },
        )
        .unwrap_err();
        assert!(error.to_string().contains("detached writer"));
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn admission_tracker_deadlines_and_cancellation_are_generation_bound() {
        use super::server_protocol::{AdmissionDeadline, RankedAdmissionTracker};
        let now = Instant::now();
        let mut tracker = RankedAdmissionTracker::default();
        tracker.begin(admission_challenge(1, 7, now));
        assert_eq!(
            tracker.deadline(1, 7, Some(7), false, now),
            AdmissionDeadline::Expired
        );
        assert_eq!(
            tracker.deadline(1, 7, Some(8), false, now),
            AdmissionDeadline::Finished
        );
        assert_eq!(
            tracker.deadline(1, 7, None, false, now),
            AdmissionDeadline::Finished
        );
        assert_eq!(
            tracker.deadline(1, 7, Some(7), true, now),
            AdmissionDeadline::Finished
        );
        assert!(!tracker.cancel_for(1, 8));
        assert!(tracker.pending().is_some());
        assert!(tracker.cancel_for(1, 7));
        assert_eq!(
            tracker.deadline(1, 7, Some(7), false, now),
            AdmissionDeadline::Waiting
        );
    }

    #[test]
    fn ready_barrier_requires_attached_connected_quorum_and_preserves_provisional_frame_maximum() {
        let mut barrier = super::server_protocol::ReadyBarrier {
            host_frame: Some(10),
            ..Default::default()
        };
        assert_eq!(barrier.candidate(2, [(true, false, Some(20))]), None);
        assert_eq!(barrier.candidate(2, [(true, true, None)]), None);
        assert_eq!(barrier.candidate(3, [(true, true, Some(20))]), None);
        assert_eq!(
            barrier.candidate(2, [(true, true, Some(20)), (false, true, Some(25))]),
            Some(25)
        );
        barrier.commit(25, 100);
        assert_eq!(barrier.candidate(2, [(true, true, Some(20))]), None);
        barrier.reset();
        assert!(barrier.host_frame.is_none());
        assert!(barrier.begun.is_none());
    }

    #[test]
    fn superseded_reader_teardown_cannot_connect_or_admit_successor() {
        let (context, events) = dispatch_test_context();
        super::ranked_lifecycle_lock(&context.ranked_lifecycle).downgrade("test browse session");
        let owner = PeerOwner::Native([1; 32]);
        let (sender, _old_wire) = unbounded_channel();
        let first = context
            .peers
            .lock()
            .sessions
            .claim_seat(owner, "old", ranked_identity(1), sender)
            .unwrap();
        let (sender, mut next_wire) = unbounded_channel();
        let next = context
            .peers
            .lock()
            .sessions
            .claim_seat(owner, "next", ranked_identity(1), sender)
            .unwrap();
        super::release_peer_session(&context, PlayerId(first.seat), owner, first.generation);
        assert_eq!(
            context.peers.lock().sessions.generation(&next.seat),
            Some(&next.generation)
        );
        assert!(!context.peers.lock().sessions.is_sim_connected(&next.seat));
        assert!(events.try_recv().is_err());
        assert!(next_wire.try_recv().is_err());
    }

    #[tokio::test]
    async fn inactive_reader_drains_terminal_frames_after_buffered_input_without_restarting_writer()
    {
        use robin_engine::multiplayer::{NetMsg, SnapshotTransitionId, SnapshotTransitionPayload};
        for mode in ["commit", "reconnect", "replacement", "release"] {
            let (context, events) = dispatch_test_context();
            let owner = PeerOwner::Native([1; 32]);
            let identity = ranked_identity(1);
            let (sender, mut wire) = unbounded_channel();
            let claim = context
                .peers
                .lock()
                .sessions
                .claim_seat(owner, "old", identity, sender)
                .unwrap();
            let id = SnapshotTransitionId {
                session_id: context.session_id,
                sequence: 1,
            };
            if mode == "commit" {
                context
                    .peers
                    .lock()
                    .transitions
                    .begin(PendingSnapshotTransition {
                        id,
                        payload: SnapshotTransitionPayload::Save {
                            mission_id: 7,
                            save_bytes: vec![1],
                        },
                        awaiting: HashSet::from([claim.seat]),
                    });
            }
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let mut delivered = Vec::new();
            let writes_started = AtomicU32::new(0);
            let writer = async {
                writes_started.fetch_add(1, super::Ordering::Relaxed);
                started_tx.send(()).unwrap();
                while let Some(message) = wire.recv().await {
                    // Keep draining asynchronous. The start signal proves the
                    // original writer was already waiting on its queue before
                    // authority loss; this is not a partial-QUIC-write test.
                    tokio::task::yield_now().await;
                    delivered.push(message);
                }
                Ok(())
            };
            let reader = async {
                started_rx.await.unwrap();
                if mode == "commit" {
                    super::dispatch_server_peer_message(
                        &context,
                        PlayerId(claim.seat),
                        claim.generation,
                        identity,
                        NetMsg::SnapshotTransitionReady { id },
                    )
                    .unwrap();
                } else {
                    let _authority = context.session_dispatch.lock();
                    let mut peers = context.peers.lock();
                    peers
                        .sessions
                        .sender(&claim.seat)
                        .unwrap()
                        .send(NetMsg::ReconnectRequired {
                            reason: "test terminal reconnect".into(),
                        })
                        .unwrap();
                    match mode {
                        "reconnect" => {
                            peers.sessions.detach_writer(&claim.seat).unwrap();
                        }
                        "replacement" => {
                            let (sender, _replacement_wire) = unbounded_channel();
                            peers
                                .sessions
                                .claim_seat(owner, "successor", identity, sender)
                                .unwrap();
                        }
                        "release" => {
                            peers
                                .sessions
                                .release_seat_if_owner(claim.seat, owner, claim.generation)
                                .unwrap();
                        }
                        _ => unreachable!(),
                    }
                }
                // Models a second frame already buffered behind the final ack.
                // The production dispatcher denies it, and the production I/O
                // driver must not cancel the already-started writer as a result.
                let outcome = super::dispatch_server_peer_message(
                    &context,
                    PlayerId(claim.seat),
                    claim.generation,
                    identity,
                    NetMsg::Input {
                        origin_frame: 99,
                        command: super::PlayerCommand::Noop,
                    },
                );
                assert!(matches!(
                    outcome,
                    Err(super::PeerDispatchFailure::Inactive { .. })
                ));
                Ok(super::peer_reader_dispatch_result(outcome)?.expect("reader lost authority"))
            };
            super::drive_server_peer_io(reader, writer, Duration::from_secs(1))
                .await
                .unwrap();
            assert_eq!(writes_started.load(super::Ordering::Relaxed), 1);
            assert_eq!(delivered.len(), 1, "{mode}");
            if mode == "commit" {
                assert!(
                    matches!(delivered[0], NetMsg::CommitSnapshotTransition { id: actual } if actual == id)
                );
                assert!(
                    matches!(events.try_recv().unwrap(), NetEvent::CommitSnapshotTransition { id: actual } if actual == id)
                );
            } else {
                assert!(
                    matches!(delivered[0], NetMsg::ReconnectRequired { .. }),
                    "{mode}"
                );
            }
            assert!(
                events.try_recv().is_err(),
                "{mode}: unauthorized input reached host"
            );
            assert!(
                context
                    .peers
                    .lock()
                    .sessions
                    .readiness()
                    .all(|(_, _, ready_frame)| ready_frame.is_none())
            );
        }
    }

    #[tokio::test]
    async fn inactive_reader_bounds_a_stalled_terminal_writer() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let writer = async {
            started_tx.send(()).unwrap();
            std::future::pending::<Result<(), String>>().await
        };
        let reader = async {
            started_rx.await.unwrap();
            Ok(super::PeerReaderExit::Inactive)
        };
        let error = super::drive_server_peer_io(reader, writer, Duration::from_millis(10))
            .await
            .unwrap_err();
        assert!(error.contains("timed out draining terminal frames"));
    }

    #[tokio::test]
    async fn independent_writer_failure_releases_generation_and_allows_snapshot_reconnect() {
        let (context, _events) = dispatch_test_context();
        let owner = PeerOwner::Native([1; 32]);
        let (sender, mut receiver) = unbounded_channel();
        let claim = context
            .peers
            .lock()
            .sessions
            .claim_seat(owner, "peer", ranked_identity(1), sender.clone())
            .unwrap();
        let reader = std::future::pending::<Result<super::PeerReaderExit, String>>();
        let writer = async move {
            receiver.close();
            Err("independent stream write failure".to_string())
        };
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            super::drive_server_peer_io(reader, writer, Duration::from_secs(1)),
        )
        .await
        .unwrap();
        assert!(
            result
                .unwrap_err()
                .contains("independent stream write failure")
        );
        assert!(sender.is_closed());
        // This is the same unconditional teardown used by the socket adapter,
        // after either half ends. The reader has never produced an EOF.
        super::release_peer_session(&context, PlayerId(claim.seat), owner, claim.generation);
        assert!(context.peers.lock().sessions.sender(&claim.seat).is_none());
        let (replacement, _replacement_rx) = unbounded_channel();
        let reconnect = context
            .peers
            .lock()
            .sessions
            .claim_seat(owner, "peer", ranked_identity(1), replacement)
            .unwrap();
        assert_eq!(reconnect.seat, claim.seat);
        assert_eq!(reconnect.kind, SeatClaimKind::Reconnect);
        assert_ne!(reconnect.generation, claim.generation);
    }

    #[test]
    fn snapshot_tracker_rejects_wrong_and_duplicate_acknowledgements_without_consuming_barrier() {
        let id = robin_engine::multiplayer::SnapshotTransitionId {
            session_id: super::MultiplayerSessionId([4; 32]),
            sequence: 1,
        };
        let mut transitions = super::SnapshotTransitions::default();
        transitions.begin(PendingSnapshotTransition {
            id,
            payload: robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                mission_id: 7,
                save_bytes: vec![1],
            },
            awaiting: HashSet::from([1, 2]),
        });
        let wrong_id = robin_engine::multiplayer::SnapshotTransitionId { sequence: 2, ..id };
        assert!(transitions.acknowledge(PlayerId(1), wrong_id).is_err());
        assert!(transitions.acknowledge(PlayerId(3), id).is_err());
        assert_eq!(
            transitions.pending().unwrap().awaiting,
            HashSet::from([1, 2])
        );
        transitions.acknowledge(PlayerId(1), id).unwrap();
        assert!(transitions.acknowledge(PlayerId(1), id).is_err());
        assert!(transitions.take_completed().is_none());
        transitions.retain_for_reconnect(1);
        transitions.acknowledge(PlayerId(2), id).unwrap();
        assert!(transitions.take_completed().is_none());
        transitions.acknowledge(PlayerId(1), id).unwrap();
        assert_eq!(transitions.take_completed(), Some(id));
        assert_eq!(transitions.take_completed(), None);
    }

    fn ranked_identity(byte: u8) -> super::RankedPeerIdentity {
        super::RankedPeerIdentity {
            durable_public_key: Some([byte; 32]),
            transport_endpoint_id: [byte.wrapping_add(1); 32],
            public_disclosure: robin_run_protocol::ParticipantPublicDisclosureV1::NamedProfile,
        }
    }

    fn leaderboard_request(
        purpose: LeaderboardCoSignPurposeV1,
        byte: u8,
    ) -> LeaderboardCoSignRequestV1 {
        LeaderboardCoSignRequestV1 {
            instance: LeaderboardCoSignInstanceV1 {
                purpose,
                replay_session_id: Digest32::from_bytes([byte; 32]),
                submission_offer_sha256: Digest32::from_bytes([byte.wrapping_add(1); 32]),
            },
            run_digest: Digest32::from_bytes([byte.wrapping_add(2); 32]),
        }
    }

    fn official_ranked_setup(host_key: &iroh::SecretKey) -> OfficialRankedSessionSetupV1 {
        let ranked_session = RankedSessionConfigV1 {
            custom_rules_config: None,
            custom_canonical_campaign: None,
            schema_version: SCHEMA_VERSION_V1,
            mission_id: "Dem_Lei_MP".to_string(),
            content_edition: OfficialContentEditionV1::Demo,
            content_subject: OfficialContentSubjectV1::FieldMission {
                mission_id: "Dem_Lei_MP".to_string(),
            },
            simulation_seed: SimulationSeed64::new(7),
            starting_campaign_sha256: Digest32::from_bytes([1; 32]),
            starting_campaign_byte_length: 1,
            prepared_inputs_projection_sha256: Digest32::from_bytes([2; 32]),
            prepared_mission_inputs_seal_sha256: Digest32::from_bytes([3; 32]),
            build_manifest_sha256: Digest32::from_bytes([4; 32]),
            content_manifest_sha256: Digest32::from_bytes([5; 32]),
            campaign_content_manifest_sha256: None,
            rules_config_sha256: Digest32::from_bytes([6; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([7; 32]),
            competition_manifest_sha256: None,
            spellforge_content_sha256: None,
            resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
            speech_timing: SpeechTimingAuthorityV1::BaseInstallation,
        };
        let host_signing_key = SigningKey::from_bytes(&host_key.to_bytes());
        let authority_key = SigningKey::from_bytes(&[0x71; 32]);
        let request_claim = FreshRunPreflightRequestClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            request_nonce: ChallengeNonce32::from_bytes([0x72; 32]),
            host_public_key: PublicKey32::from_bytes(host_signing_key.verifying_key().to_bytes()),
            replay_session_id: Digest32::from_bytes([0x73; 32]),
            host_participant_instance_id: Digest32::from_bytes([0x74; 32]),
            host_nonce: ChallengeNonce32::from_bytes([0x75; 32]),
            scope: FreshRunScopeV1::IndividualLevel,
            starting_campaign: ArtifactRefV1 {
                sha256: ranked_session.starting_campaign_sha256,
                byte_length: ranked_session.starting_campaign_byte_length,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_string(),
            },
            ranked_session: ranked_session.clone(),
        };
        let request = FreshRunPreflightRequestV1 {
            host_signature: Signature64::from_bytes(
                host_signing_key
                    .sign(&request_claim.signing_bytes().unwrap())
                    .to_bytes(),
            ),
            claim: request_claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        let grant_claim = FreshRunPreflightGrantClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            grant_id: OpaqueId::new("native-ranked-test-grant").unwrap(),
            grant_nonce: ChallengeNonce32::from_bytes([0x76; 32]),
            grant_authority_public_key: PublicKey32::from_bytes(
                authority_key.verifying_key().to_bytes(),
            ),
            host_public_key: request.claim.host_public_key,
            grant_request_sha256: request.canonical_digest().unwrap(),
            ranked_session_sha256: ranked_session.canonical_digest().unwrap(),
            replay_session_id: request.claim.replay_session_id,
            host_participant_instance_id: request.claim.host_participant_instance_id,
            host_nonce: request.claim.host_nonce,
            scope: request.claim.scope,
            starting_campaign: request.claim.starting_campaign.clone(),
            admitted_at_unix_ms: 1_000,
            expires_at_unix_ms: 2_000,
        };
        let grant = FreshRunPreflightGrantV1 {
            authority_signature: Signature64::from_bytes(
                authority_key
                    .sign(&grant_claim.signing_bytes().unwrap())
                    .to_bytes(),
            ),
            claim: grant_claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        OfficialRankedSessionSetupV1 {
            ranked_session,
            custom_package_present: false,
            run_preflight: RankedRunPreflightAdmissionV1::Fresh { request, grant },
            run_preflight_grant_public_key: PublicKey32::from_bytes(
                authority_key.verifying_key().to_bytes(),
            ),
            trusted_now_unix_ms: 1_500,
        }
    }

    #[track_caller]
    fn recv_matching(
        receiver: &Receiver<NetEvent>,
        timeout: Duration,
        mut predicate: impl FnMut(&NetEvent) -> bool,
    ) -> NetEvent {
        let deadline = Instant::now() + timeout;
        let mut skipped = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = match receiver.recv_timeout(remaining) {
                Ok(event) => event,
                Err(error) => {
                    panic!("timed out waiting for multiplayer event: {error}; observed {skipped:?}")
                }
            };
            if predicate(&event) {
                return event;
            }
            skipped.push(event);
        }
    }

    fn signed_response(
        request: &LeaderboardCoSignRequestV1,
        key: &iroh::SecretKey,
    ) -> LeaderboardCoSignResponse {
        LeaderboardCoSignResponse {
            instance: request.instance,
            signer_public_key: *key.public().as_bytes(),
            signature: key.sign(&request.signing_bytes().unwrap()).to_bytes(),
        }
    }

    fn admit_ranked_test_identity(peers: &mut ServerPeers, seat: u8, key: &iroh::SecretKey) {
        peers.sessions.set_test_ranked_identity(
            seat,
            super::RankedPeerIdentity {
                durable_public_key: Some(*key.public().as_bytes()),
                transport_endpoint_id: *key.public().as_bytes(),
                public_disclosure: robin_run_protocol::ParticipantPublicDisclosureV1::NamedProfile,
            },
        );
    }

    fn claim_test_seat(
        peers: &mut ServerPeers,
        seat: u8,
        sender: tokio::sync::mpsc::UnboundedSender<robin_engine::multiplayer::NetMsg>,
    ) {
        let claim = peers
            .sessions
            .claim_seat(
                PeerOwner::Native([seat; 32]),
                "test peer",
                ranked_identity(seat),
                sender,
            )
            .unwrap();
        assert_eq!(claim.seat, seat);
    }

    fn offer() -> robin_engine::multiplayer::DistributedModOffer {
        robin_engine::multiplayer::DistributedModOffer {
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
        }
    }

    #[test]
    fn admission_phase_caps_fit_max_valid_offer_and_chunk() {
        let text = "x".repeat(robin_engine::multiplayer::DistributedModOffer::TEXT_BYTE_LIMIT);
        let host_id = "x".repeat(
            robin_engine::multiplayer::DistributedModOffer::AUTHENTICATED_HOST_ID_BYTE_LIMIT,
        );
        let offer = robin_engine::multiplayer::DistributedModOffer {
            schema_version: 1,
            full_mod_sha256: [1; 32],
            spellforge_package_sha256: None,
            spellforge_vm_abi: None,
            encoded_bytes: 1,
            mission_basename: text.clone(),
            mission_rhm_entry: text.clone(),
            map_filename: text.clone(),
            title: text.clone(),
            claimed_author: text.clone(),
            version: text.clone(),
            source_url: text.clone(),
            license: text.clone(),
            host_endpoint_id: host_id,
        };
        offer.validate().unwrap();
        let offer_bytes = super::encode_msg(&super::NetMsg::ContentOffer { offer });
        assert!(offer_bytes.len() <= MAX_SERVER_CONTROL_FRAME_BYTES);

        let chunk_bytes = super::encode_msg(&super::NetMsg::ContentChunk {
            full_mod_sha256: [1; 32],
            offset: 0,
            total_bytes: robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT as u64,
            bytes: vec![0; robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT],
        });
        assert!(chunk_bytes.len() <= MAX_CONTENT_FRAME_BYTES);
        assert!(chunk_bytes.len() > MAX_SERVER_CONTROL_FRAME_BYTES);
    }

    #[test]
    fn native_epoch_conversion_accepts_boundary_and_rejects_overflow() {
        assert_eq!(checked_epoch_ms(0), Ok(0));
        assert_eq!(checked_epoch_ms(u128::from(u64::MAX)), Ok(u64::MAX));
        assert!(checked_epoch_ms(u128::from(u64::MAX) + 1).is_err());
    }

    #[test]
    fn native_clock_returns_a_real_post_epoch_timestamp() {
        assert!(super::current_epoch_ms().expect("native system clock") > 0);
    }

    #[test]
    fn native_gameplay_rejects_late_content_and_opening_messages() {
        let (incoming_tx, incoming_rx) = std::sync::mpsc::channel();
        let cosign_state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &cosign_state,
                None,
                super::NetMsg::ContentOffer { offer: offer() },
            )
            .unwrap_err()
            .contains("invalid native session message")
        );
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &cosign_state,
                None,
                super::NetMsg::ContentChunk {
                    full_mod_sha256: [1; 32],
                    offset: 0,
                    total_bytes: 1,
                    bytes: vec![0],
                },
            )
            .is_err()
        );
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &cosign_state,
                None,
                super::NetMsg::Reject {
                    reason: "session revoked".into(),
                },
            )
            .unwrap_err()
            .contains("session revoked")
        );
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &cosign_state,
                None,
                super::NetMsg::Welcome {
                    your_seat: PlayerId(1),
                    mission_id: "late".into(),
                    mission_seed: 1,
                    sim_config: robin_engine::engine::SimConfig::default(),
                    speech_timing_locale: None,
                    host_nickname: "host".into(),
                    session_id: robin_engine::multiplayer::MultiplayerSessionId([2; 32]),
                },
            )
            .unwrap_err()
            .contains("invalid native session message")
        );
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &cosign_state,
                None,
                super::NetMsg::Note("legal".into()),
            )
            .is_ok()
        );
        assert!(matches!(
            incoming_rx.recv().unwrap(),
            super::NetEvent::Note(note) if note == "legal"
        ));
    }

    #[test]
    fn native_gameplay_rejects_host_only_and_late_content_outbound() {
        assert!(
            client_gameplay_wire_msg(super::NetOutbound::StateHash {
                frame: 1,
                hash: Some(2),
                clock_frame: Some(3),
                ms_until_next_frame: Some(4),
            })
            .unwrap_err()
            .contains("host-only")
        );
        assert!(
            client_gameplay_wire_msg(super::NetOutbound::ContentReady {
                full_mod_sha256: [1; 32],
            })
            .unwrap_err()
            .contains("after gameplay began")
        );
        assert!(matches!(
            client_gameplay_wire_msg(super::NetOutbound::ReadyToSim { frame: 7 }).unwrap(),
            super::NetMsg::ReadyToSim { frame: 7 }
        ));
    }

    #[test]
    fn native_server_gameplay_rejects_wrong_direction_messages() {
        assert!(validate_server_gameplay_wire_msg(&super::NetMsg::Note("legal".into())).is_ok());
        assert!(
            validate_server_gameplay_wire_msg(&super::NetMsg::ContentRequest {
                full_mod_sha256: [1; 32],
                resume_offset: 0,
            })
            .unwrap_err()
            .contains("ordinary peer session")
        );
        assert!(
            validate_server_gameplay_wire_msg(&super::NetMsg::StateHash {
                frame: 1,
                hash: Some(2),
                clock_frame: Some(3),
                ms_until_next_frame: Some(4),
            })
            .unwrap_err()
            .contains("invalid server-session message")
        );
        assert!(
            validate_server_gameplay_wire_msg(&super::NetMsg::Hello {
                protocol_version: robin_engine::multiplayer::NET_PROTOCOL_VERSION,
                nickname: "late".into(),
                browser_auth: None,
                ranked_public_key: None,
            })
            .unwrap_err()
            .contains("invalid server-session message")
        );

        assert!(
            validate_server_gameplay_outbound(&super::NetOutbound::StateHash {
                frame: 1,
                hash: Some(2),
                clock_frame: Some(3),
                ms_until_next_frame: Some(4),
            })
            .is_ok()
        );
        assert!(
            validate_server_gameplay_outbound(&super::NetOutbound::ContentPrepared {
                full_mod_sha256: [1; 32],
            })
            .unwrap_err()
            .contains("client-only")
        );
    }

    #[test]
    fn peer_inputs_reject_host_authoritative_commands_before_broadcast() {
        let error = validate_peer_command_authority(
            PlayerId(2),
            &robin_engine::player_command::PlayerCommand::ConnectSeat {
                player_id: PlayerId(7),
                nickname: "forged".to_string(),
            },
        )
        .expect_err("a peer must not author transport seat lifecycle");
        assert!(error.contains("host-authoritative"));

        validate_peer_command_authority(
            PlayerId(2),
            &robin_engine::player_command::PlayerCommand::CrouchDown,
        )
        .expect("ordinary seat input remains admissible");
    }

    #[test]
    fn campaign_owners_isolate_transport_identity_and_handoffs() {
        let a = super::MultiplayerCampaignSession::default();
        let b = super::MultiplayerCampaignSession::default();
        assert_eq!(a.state().client_key.public(), a.state().client_key.public());
        assert_ne!(a.state().client_key.public(), b.state().client_key.public());
        let continuation = HostSessionContinuation {
            host_endpoint_id: iroh::SecretKey::from_bytes(&[3; 32]).public(),
            session_id: robin_engine::multiplayer::MultiplayerSessionId([4; 32]),
            expected_players: 2,
            owner_seats: std::collections::HashMap::from([(PeerOwner::Native([8; 32]), 1)]),
            relay_url: None,
        };
        super::publish_host_session_continuation(a.state(), continuation.clone());
        super::publish_host_session_continuation(b.state(), continuation.clone());
        a.discard_host_continuation().unwrap();
        drop(a);
        assert!(
            super::pending_host_session_continuation(b.state(), continuation.host_endpoint_id, 3)
                .is_err()
        );
        assert!(
            super::pending_host_session_continuation(
                b.state(),
                iroh::SecretKey::generate().public(),
                2
            )
            .is_err()
        );
        let restored =
            super::pending_host_session_continuation(b.state(), continuation.host_endpoint_id, 2)
                .unwrap()
                .unwrap();
        assert_eq!(restored.owner_seats, continuation.owner_seats);
        assert_eq!(restored.session_id, continuation.session_id);
        // Reading for replacement preparation is transactional: failure before
        // successful endpoint publication leaves the handoff intact.
        assert!(b.state().continuation.lock().is_some());
    }

    #[test]
    fn failed_startup_cancellation_joins_bridge_with_sender_still_alive() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (bridge, _async_receiver) = super::spawn_outgoing_bridge(
            "test-campaign-failed-startup",
            receiver,
            cancellation.clone(),
        )
        .unwrap();
        cancellation.store(true, std::sync::atomic::Ordering::Release);
        bridge.join().unwrap();
        drop(sender);
    }

    #[test]
    fn failed_campaign_server_start_keeps_handoff_and_releases_lease() {
        let campaign = super::MultiplayerCampaignSession::default();
        let key = iroh::SecretKey::from_bytes(&[3; 32]);
        let continuation = HostSessionContinuation {
            host_endpoint_id: key.public(),
            session_id: robin_engine::multiplayer::MultiplayerSessionId([4; 32]),
            expected_players: 2,
            owner_seats: std::collections::HashMap::from([(PeerOwner::Native([8; 32]), 1)]),
            relay_url: None,
        };
        super::publish_host_session_continuation(campaign.state(), continuation.clone());
        let (_channels, incoming, outgoing, cursor, snapshot) =
            crate::multiplayer::NetChannels::new();
        let result = super::start_server_inner(
            &campaign,
            key,
            "host".into(),
            "Dem_Lei_MP".into(),
            42,
            robin_engine::engine::SimConfig::default(),
            None,
            incoming,
            outgoing,
            cursor,
            snapshot,
            3,
            None,
            false,
        );
        let Err(error) = result else {
            panic!("mismatched replacement unexpectedly started");
        };
        assert!(error.to_string().contains("expects 2 players"));
        let pending = super::pending_host_session_continuation(
            campaign.state(),
            continuation.host_endpoint_id,
            2,
        )
        .unwrap()
        .unwrap();
        assert_eq!(pending.owner_seats, continuation.owner_seats);
        assert!(campaign.reserve_server().is_ok());
    }

    #[test]
    fn campaign_server_lease_rejects_overlap_and_releases_failed_preparation() {
        let campaign = super::MultiplayerCampaignSession::default();
        let lease = campaign.reserve_server().unwrap();
        assert!(campaign.reserve_server().is_err());
        assert!(campaign.discard_host_continuation().is_err());
        let other = super::MultiplayerCampaignSession::default();
        let other_lease = other.reserve_server().unwrap();
        drop(lease);
        let replacement = campaign.reserve_server().unwrap();
        assert!(other.reserve_server().is_err());
        drop(other_lease);
        drop(replacement);
        assert!(campaign.reserve_server().is_ok());
    }

    #[test]
    fn campaign_serialization_cannot_restore_transport_authority() {
        let campaign = super::MultiplayerCampaignSession::default();
        let encoded = serde_json::to_string(&campaign).unwrap();
        assert_eq!(encoded, "{}");
        let decoded: super::MultiplayerCampaignSession = serde_json::from_str(&encoded).unwrap();
        assert!(decoded.state.is_none());
    }

    #[test]
    fn campaign_repeated_publication_merges_authenticated_seats() {
        let campaign = super::MultiplayerCampaignSession::default();
        let mut continuation = HostSessionContinuation {
            host_endpoint_id: iroh::SecretKey::from_bytes(&[3; 32]).public(),
            session_id: robin_engine::multiplayer::MultiplayerSessionId([4; 32]),
            expected_players: 3,
            owner_seats: std::collections::HashMap::from([(PeerOwner::Native([8; 32]), 1)]),
            relay_url: None,
        };
        super::publish_host_session_continuation(campaign.state(), continuation.clone());
        continuation.owner_seats =
            std::collections::HashMap::from([(PeerOwner::Browser([9; 32]), 2)]);
        super::publish_host_session_continuation(campaign.state(), continuation.clone());
        assert_eq!(
            super::pending_host_session_continuation(
                campaign.state(),
                continuation.host_endpoint_id,
                3
            )
            .unwrap()
            .unwrap()
            .owner_seats
            .len(),
            2
        );
    }

    #[test]
    fn replacement_transport_seeds_exact_authenticated_seats() {
        let browser = PeerOwner::Browser([7; 32]);
        let native = PeerOwner::Native([8; 32]);
        let continuation = HostSessionContinuation {
            host_endpoint_id: iroh::SecretKey::from_bytes(&[3; 32]).public(),
            session_id: robin_engine::multiplayer::MultiplayerSessionId([4; 32]),
            expected_players: 3,
            owner_seats: std::collections::HashMap::from([(browser, 1), (native, 2)]),
            relay_url: None,
        };
        let mut peers = ServerPeers::from_continuation(&continuation);

        let (browser_tx, _browser_rx) = unbounded_channel();
        let browser_claim = peers
            .sessions
            .claim_seat(browser, "renamed browser", ranked_identity(7), browser_tx)
            .unwrap();
        let (native_tx, _native_rx) = unbounded_channel();
        let native_claim = peers
            .sessions
            .claim_seat(native, "renamed native", ranked_identity(8), native_tx)
            .unwrap();

        assert_eq!(browser_claim.seat, 1);
        assert_eq!(browser_claim.kind, SeatClaimKind::Reconnect);
        assert_eq!(native_claim.seat, 2);
        assert_eq!(native_claim.kind, SeatClaimKind::Reconnect);
        let (intruder_tx, _intruder_rx) = unbounded_channel();
        assert!(
            peers
                .sessions
                .claim_seat(
                    PeerOwner::Browser([9; 32]),
                    "same nickname",
                    ranked_identity(9),
                    intruder_tx,
                )
                .is_err(),
            "a replacement session may not allocate beyond its retained roster"
        );
    }

    #[test]
    fn snapshot_transition_waits_for_every_current_peer() {
        let session_id = robin_engine::multiplayer::MultiplayerSessionId([4; 32]);
        let id = robin_engine::multiplayer::SnapshotTransitionId {
            session_id,
            sequence: 2,
        };
        let mut peers = ServerPeers::new(3);
        for seat in [1, 2] {
            let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
            claim_test_seat(&mut peers, seat, sender);
        }
        peers.transitions.begin(PendingSnapshotTransition {
            id,
            payload: robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                mission_id: 7,
                save_bytes: vec![1, 2, 3],
            },
            awaiting: HashSet::from([1, 2]),
        });

        assert!(take_committed_snapshot_transition(&mut peers).is_none());
        peers.transitions.acknowledge(PlayerId(1), id).unwrap();
        assert!(take_committed_snapshot_transition(&mut peers).is_none());
        peers.transitions.acknowledge(PlayerId(2), id).unwrap();
        let (committed_id, senders) =
            take_committed_snapshot_transition(&mut peers).expect("all peers acknowledged");
        assert_eq!(committed_id, id);
        assert_eq!(senders.len(), 2);
        assert_eq!(peers.sessions.senders().count(), 0);
        assert!(peers.transitions.pending().is_none());
    }

    #[test]
    fn disconnected_transition_peer_must_ack_again_after_reconnect() {
        let id = robin_engine::multiplayer::SnapshotTransitionId {
            session_id: robin_engine::multiplayer::MultiplayerSessionId([6; 32]),
            sequence: 1,
        };
        let mut peers = ServerPeers::new(1);
        peers.transitions.begin(PendingSnapshotTransition {
            id,
            payload: robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                mission_id: 3,
                save_bytes: vec![4, 5],
            },
            awaiting: HashSet::new(),
        });

        retain_transition_peer_for_reconnect(&mut peers, 1);
        assert!(peers.transitions.pending().unwrap().awaiting.contains(&1));
        assert!(take_committed_snapshot_transition(&mut peers).is_none());
    }

    #[test]
    fn server_cosign_state_targets_one_authenticated_seat_and_rejects_duplicates() {
        let request = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 70);
        let key = iroh::SecretKey::generate();
        let response = signed_response(&request, &key);
        let mut peers = ServerPeers::new(3);
        let (seat_one_tx, mut seat_one_rx) = unbounded_channel();
        let (seat_two_tx, mut seat_two_rx) = unbounded_channel();
        claim_test_seat(&mut peers, 1, seat_one_tx);
        claim_test_seat(&mut peers, 2, seat_two_tx);
        admit_ranked_test_identity(&mut peers, 1, &key);

        let target = peers
            .begin_leaderboard_cosign(PlayerId(1), request)
            .unwrap();
        target
            .send(robin_engine::multiplayer::NetMsg::LeaderboardCoSignRequest(
                request,
            ))
            .unwrap();
        assert!(matches!(
            seat_one_rx.try_recv().unwrap(),
            robin_engine::multiplayer::NetMsg::LeaderboardCoSignRequest(decoded)
                if decoded == request
        ));
        assert!(
            seat_two_rx.try_recv().is_err(),
            "request must not broadcast"
        );

        assert!(
            peers
                .complete_leaderboard_cosign(PlayerId(2), &response)
                .unwrap_err()
                .contains("wrong-target")
        );
        peers
            .complete_leaderboard_cosign(PlayerId(1), &response)
            .unwrap();
        assert!(
            peers
                .complete_leaderboard_cosign(PlayerId(1), &response)
                .unwrap_err()
                .contains("duplicate")
        );
        assert!(
            peers
                .begin_leaderboard_cosign(PlayerId(1), request)
                .unwrap_err()
                .contains("duplicate")
        );
    }

    #[test]
    fn server_cosign_state_allows_same_final_request_for_distinct_targets_only() {
        let request = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 71);
        let mut peers = ServerPeers::new(3);
        for seat in [1, 2] {
            let (sender, _receiver) = unbounded_channel();
            claim_test_seat(&mut peers, seat, sender);
            peers
                .begin_leaderboard_cosign(PlayerId(seat), request)
                .unwrap();
        }
        assert_eq!(peers.cosigns.pending_count(), 2);
        assert_eq!(peers.cosigns.seen_count(), 2);
    }

    #[test]
    fn invalid_cosign_signature_does_not_consume_the_pending_request() {
        let request = leaderboard_request(LeaderboardCoSignPurposeV1::CampaignContinuation, 72);
        let mut peers = ServerPeers::new(2);
        let admitted_key = iroh::SecretKey::generate();
        let (sender, _receiver) = unbounded_channel();
        claim_test_seat(&mut peers, 1, sender);
        admit_ranked_test_identity(&mut peers, 1, &admitted_key);
        peers
            .begin_leaderboard_cosign(PlayerId(1), request)
            .unwrap();

        let invalid = signed_response(&request, &iroh::SecretKey::generate());
        assert!(
            peers
                .complete_leaderboard_cosign(PlayerId(1), &invalid)
                .unwrap_err()
                .contains("other than its admitted durable identity")
        );
        assert_eq!(peers.cosigns.pending_count(), 1);
        let valid = signed_response(&request, &admitted_key);
        peers
            .complete_leaderboard_cosign(PlayerId(1), &valid)
            .unwrap();
        assert!(peers.cosigns.pending_count() == 0);
    }

    #[test]
    fn client_wire_handler_never_exposes_unarmed_or_wrong_direction_cosign() {
        let request = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 73);
        let state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
        let (incoming_tx, incoming_rx) = std::sync::mpsc::channel();
        handle_client_wire_msg(
            &incoming_tx,
            &state,
            None,
            robin_engine::multiplayer::NetMsg::LeaderboardCoSignRequest(request),
        )
        .unwrap();
        assert!(incoming_rx.try_recv().is_err());
        assert_eq!(state.arm_request(request).unwrap(), Some(request));

        let response = signed_response(&request, &iroh::SecretKey::generate());
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &state,
                None,
                robin_engine::multiplayer::NetMsg::LeaderboardCoSignResponse(response),
            )
            .unwrap_err()
            .contains("client-only")
        );
    }

    #[test]
    fn reconnect_rejects_wrong_session_mission_config_or_speech_locale() {
        let expected = robin_engine::engine::SimConfig::default();
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "MissionB",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_err()
        );

        let mut changed = expected;
        changed.amount_of_speaking = 9;
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "MissionA",
                7,
                changed,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_err()
        );
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(2),
                "MissionA",
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
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("de-DE"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_err()
        );
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "MissionA",
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
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_ok()
        );
    }

    #[test]
    fn host_reconnect_directive_ends_the_complete_client_session() {
        let (incoming_tx, _incoming_rx) = std::sync::mpsc::channel();
        let cosign_state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
        let error = handle_client_wire_msg(
            &incoming_tx,
            &cosign_state,
            None,
            robin_engine::multiplayer::NetMsg::ReconnectRequired {
                reason: "late input predates rollback horizon".to_string(),
            },
        )
        .expect_err("directive must unwind the session into the reconnect loop");
        assert!(error.contains("full-snapshot reconnect"));
        assert!(error.contains("rollback horizon"));
    }

    #[test]
    fn reconnect_discards_commands_queued_for_abandoned_session() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        sender
            .send(robin_engine::multiplayer::NetOutbound::Input {
                origin_frame: 41,
                command: robin_engine::player_command::PlayerCommand::CrouchDown,
            })
            .expect("queue old-session command");
        sender
            .send(robin_engine::multiplayer::NetOutbound::ReadyToSim { frame: 40 })
            .expect("queue old-session readiness");

        assert_eq!(discard_session_outbound(&mut receiver), 2);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn client_transition_events_preserve_exact_prepare_bytes_and_commit_id() {
        let (incoming_tx, incoming_rx) = std::sync::mpsc::channel();
        let cosign_state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
        let id = robin_engine::multiplayer::SnapshotTransitionId {
            session_id: robin_engine::multiplayer::MultiplayerSessionId([5; 32]),
            sequence: 9,
        };
        let save_bytes = vec![0, 17, 34, 255];
        handle_client_wire_msg(
            &incoming_tx,
            &cosign_state,
            None,
            robin_engine::multiplayer::NetMsg::PrepareSnapshotTransition {
                id,
                payload: robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                    mission_id: 71,
                    save_bytes: save_bytes.clone(),
                },
            },
        )
        .unwrap();
        assert!(matches!(
            incoming_rx.recv().unwrap(),
            robin_engine::multiplayer::NetEvent::PrepareSnapshotTransition {
                id: decoded_id,
                payload: robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                    mission_id: 71,
                    save_bytes: decoded_bytes,
                },
            } if decoded_id == id && decoded_bytes == save_bytes
        ));

        handle_client_wire_msg(
            &incoming_tx,
            &cosign_state,
            None,
            robin_engine::multiplayer::NetMsg::CommitSnapshotTransition { id },
        )
        .unwrap();
        assert!(matches!(
            incoming_rx.recv().unwrap(),
            robin_engine::multiplayer::NetEvent::CommitSnapshotTransition { id: decoded_id }
                if decoded_id == id
        ));
    }

    #[test]
    fn authenticated_owner_reclaims_and_replaces_only_its_original_seat() {
        let mut peers = ServerPeers::new(3);
        let owner = PeerOwner::Browser([7; 32]);
        let other = PeerOwner::Browser([8; 32]);
        let (first_tx, _first_rx) = unbounded_channel();
        let first = peers
            .sessions
            .claim_seat(owner, "Robin", ranked_identity(7), first_tx)
            .unwrap();
        let seat = first.seat;
        let generation = first.generation;
        assert_eq!(seat, 1);
        assert_eq!(first.kind, SeatClaimKind::Fresh);
        assert!(peers.sessions.connect_sim_seat(seat));

        let (replacement_tx, _replacement_rx) = unbounded_channel();
        let replacement = peers
            .sessions
            .claim_seat(owner, "Robin renamed", ranked_identity(7), replacement_tx)
            .unwrap();
        let replacement_seat = replacement.seat;
        let replacement_generation = replacement.generation;
        assert_eq!(replacement_seat, seat);
        assert_eq!(replacement.kind, SeatClaimKind::ActiveReplacement);
        assert!(peers.sessions.is_sim_connected(&seat));
        assert_ne!(replacement_generation, generation);
        assert_eq!(
            peers
                .sessions
                .release_seat_if_owner(seat, owner, generation),
            None
        );

        assert_eq!(
            peers
                .sessions
                .release_seat_if_owner(seat, owner, replacement_generation),
            Some(true)
        );
        let (other_tx, _other_rx) = unbounded_channel();
        let other_claim = peers
            .sessions
            .claim_seat(other, "Robin renamed", ranked_identity(8), other_tx)
            .unwrap();
        let other_seat = other_claim.seat;
        assert_eq!(
            other_seat, 2,
            "a matching nickname grants no seat authority"
        );

        let (rejoin_tx, _rejoin_rx) = unbounded_channel();
        let rejoined = peers
            .sessions
            .claim_seat(owner, "New name", ranked_identity(7), rejoin_tx)
            .unwrap();
        let rejoined_seat = rejoined.seat;
        assert_eq!(rejoined_seat, seat);
        assert_eq!(rejoined.kind, SeatClaimKind::Reconnect);
    }

    #[test]
    fn real_iroh_ranked_admission_uses_durable_key_and_gates_begin_and_reconnect() {
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_max_level(tracing::Level::INFO)
            .try_init();
        let host_key = iroh::SecretKey::generate();
        let setup = official_ranked_setup(&host_key);
        let transport_key = iroh::SecretKey::generate();
        let durable_key = iroh::SecretKey::generate();
        let transport_public = *transport_key.public().as_bytes();
        let durable_public = *durable_key.public().as_bytes();
        assert_ne!(transport_public, durable_public);

        let (server_in_tx, server_in_rx) = channel();
        let (server_out_tx, server_out_rx) = channel();
        let mut server = start_server_with_key(
            host_key,
            "host".into(),
            "Dem_Lei_MP".into(),
            7,
            robin_engine::engine::SimConfig::default(),
            Some("en-US".into()),
            server_in_tx,
            server_out_rx,
            Arc::new(AtomicU32::new(0)),
            Arc::new(StdMutex::new(None)),
            2,
        )
        .expect("start real iroh ranked host");
        server
            .install_ranked_session_setup(Some(setup.clone()))
            .expect("install ranked host setup");

        let (client_in_tx, client_in_rx) = channel();
        let (client_out_tx, client_out_rx) = channel();
        let mut client = connect_client_with_keys(
            transport_key.clone(),
            Some(durable_key.clone()),
            server.connect_string(),
            "alice".into(),
            client_in_tx,
            client_out_rx,
        )
        .expect("connect ranked client");
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::AssignedLocalSeat(PlayerId(1)))
        });

        client_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .unwrap();
        server_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .unwrap();
        let setup_deadline = Instant::now() + Duration::from_millis(200);
        loop {
            match client_in_rx
                .recv_timeout(setup_deadline.saturating_duration_since(Instant::now()))
            {
                Ok(NetEvent::BeginSim { .. }) => {
                    panic!("ranked admission must resolve before BeginSim")
                }
                Ok(NetEvent::RankedBrowseOnly { reason }) => {
                    panic!("ReadyToSim resolved pending ranked setup as browse-only: {reason:?}")
                }
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("ranked client event channel closed before setup")
                }
            }
        }
        assert!(
            super::ranked_lifecycle_lock(&client.ranked_lifecycle())
                .browse_only_reason()
                .is_none(),
            "ReadyToSim must leave explicit ranked setup unresolved"
        );

        client
            .install_ranked_session_setup(Some(setup.clone()))
            .expect("install exact ranked client setup");
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::RankedJoinAccepted(_))
        });
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::BeginSim { .. })
        });
        recv_matching(&server_in_rx, Duration::from_secs(10), |event| {
            matches!(
                event,
                NetEvent::Input { input, .. }
                    if matches!(
                        input.command,
                        robin_engine::player_command::PlayerCommand::ConnectSeat {
                            player_id: PlayerId(1),
                            ..
                        }
                    )
            )
        });

        let submission_accepted = SubmissionAcceptedV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission_id: OpaqueId::new("native-ranked-submission").unwrap(),
            state: SubmissionLifecycleV1::Queued,
            retry_after_ms: 250,
        };
        let accepted_document = robin_engine::multiplayer::RankedSubmissionAcceptedDocument::new(
            crate::leaderboard_ranked_session::encode_ranked_wire_document(&submission_accepted)
                .unwrap(),
        )
        .unwrap();
        server_out_tx
            .send(NetOutbound::RankedSubmissionAccepted {
                to: PlayerId(1),
                accepted: accepted_document,
            })
            .unwrap();
        let received = recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::RankedSubmissionAccepted(_))
        });
        let NetEvent::RankedSubmissionAccepted(received) = received else {
            unreachable!()
        };
        let decoded: SubmissionAcceptedV1 =
            crate::leaderboard_ranked_session::decode_ranked_wire_document(received.as_bytes())
                .unwrap();
        assert_eq!(decoded, submission_accepted);

        {
            let ranked_lifecycle = server.ranked_lifecycle();
            let lifecycle = super::ranked_lifecycle_lock(&ranked_lifecycle);
            let session = lifecycle
                .ranked_session()
                .expect("host remains ranked after admission");
            let guest = session
                .participant_claims()
                .into_iter()
                .find(|participant| participant.seat == 1)
                .expect("guest claim retained");
            assert_eq!(*guest.public_key.as_bytes(), durable_public);
            assert_eq!(
                *guest
                    .join_attestation
                    .expect("guest claim is attested")
                    .claim
                    .transport_endpoint_id
                    .as_bytes(),
                transport_public
            );
        }

        let disconnected_sender = server
            .context
            .peers
            .lock()
            .sessions
            .detach_writer(&1)
            .expect("ranked client has an active server writer");
        drop(disconnected_sender);
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::Disconnected)
        });
        recv_matching(&server_in_rx, Duration::from_secs(10), |event| {
            matches!(
                event,
                NetEvent::Input { input, .. }
                    if matches!(
                        input.command,
                        robin_engine::player_command::PlayerCommand::DisconnectSeat {
                            player_id: PlayerId(1)
                        }
                    )
            )
        });
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::Reconnected)
        });
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::AssignedLocalSeat(PlayerId(1)))
        });
        let reconnect_deadline = Instant::now() + Duration::from_secs(10);
        let mut accepted_before_begin = false;
        loop {
            let event = client_in_rx
                .recv_timeout(reconnect_deadline.saturating_duration_since(Instant::now()))
                .expect("ranked reconnect did not resume before timeout");
            match event {
                NetEvent::RankedJoinAccepted(_) => accepted_before_begin = true,
                NetEvent::BeginSim { .. } => {
                    assert!(
                        accepted_before_begin,
                        "cached BeginSim bypassed reconnect admission"
                    );
                    break;
                }
                NetEvent::Fatal(error) => panic!("ranked reconnect failed: {error}"),
                _ => {}
            }
        }
        client.shutdown();
        server.shutdown();
    }

    #[test]
    fn real_iroh_ready_before_browse_downgrade_still_begins_gameplay() {
        let (server_in_tx, _server_in_rx) = channel();
        let (server_out_tx, server_out_rx) = channel();
        let mut server = start_server_with_key(
            iroh::SecretKey::generate(),
            "host".into(),
            "Dem_Lei_MP".into(),
            7,
            robin_engine::engine::SimConfig::default(),
            Some("en-US".into()),
            server_in_tx,
            server_out_rx,
            Arc::new(AtomicU32::new(0)),
            Arc::new(StdMutex::new(None)),
            2,
        )
        .expect("start real iroh browse-only host");
        let (client_in_tx, client_in_rx) = channel();
        let (client_out_tx, client_out_rx) = channel();
        let mut client = connect_client_with_keys(
            iroh::SecretKey::generate(),
            None,
            server.connect_string(),
            "alice".into(),
            client_in_tx,
            client_out_rx,
        )
        .expect("connect browse-only client");
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::AssignedLocalSeat(PlayerId(1)))
        });
        client_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .unwrap();
        server_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .unwrap();
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::RankedBrowseOnly { .. })
        });
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::BeginSim { .. })
        });
        assert!(
            super::ranked_lifecycle_lock(&server.ranked_lifecycle())
                .browse_only_reason()
                .is_some()
        );
        assert!(
            super::ranked_lifecycle_lock(&client.ranked_lifecycle())
                .browse_only_reason()
                .is_some()
        );
        client.shutdown();
        server.shutdown();
    }
}
