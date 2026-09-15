//! Native server transport tests: peer dispatch, transition trackers, seat
//! ownership, and real-iroh host sessions.
use super::{
    HostSessionContinuation, PeerOwner, PendingSnapshotTransition, SeatClaimKind, ServerPeers,
    connect_client_with_key, retain_transition_peer_for_reconnect, start_server_with_key,
    take_committed_snapshot_transition, validate_peer_command_authority,
    validate_server_gameplay_outbound, validate_server_gameplay_wire_msg,
};
use crate::multiplayer::{ServerChannels, ServerConfig};
use robin_engine::multiplayer::{NetEvent, NetOutbound};
use robin_engine::player_command::PlayerId;
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
    super::fail_server(
        &context,
        super::MultiplayerError::LocalState("first failure".into()),
    );
    super::fail_server(
        &context,
        super::MultiplayerError::LocalState("later failure".into()),
    );
    assert!(context.cancellation.load(super::Ordering::Acquire));
    assert!(*shutdown.borrow());
    assert!(
        matches!(events.try_recv(), Ok(NetEvent::Fatal(message)) if message.to_string() == "first failure")
    );
    assert!(matches!(
        events.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn stale_peer_dispatch_rejects_every_effect_before_touching_successor_state() {
    use robin_engine::multiplayer::{
        ModalInstanceId, NetMsg, SnapshotTransitionId, SnapshotTransitionPayload,
    };
    use robin_engine::player_command::{DialogResult, ModalKind};
    for invalidation in ["replacement", "detachment", "release"] {
        let (context, events) = dispatch_test_context();
        let owner = PeerOwner::Native([1; 32]);
        let (sender, mut old_wire) = unbounded_channel();
        let first = context
            .peers
            .lock()
            .sessions
            .claim_seat(owner, "first", sender)
            .unwrap();
        let (replacement_sender, mut replacement_wire) = unbounded_channel();
        {
            let mut peers = context.peers.lock();
            match invalidation {
                "replacement" => {
                    peers
                        .sessions
                        .claim_seat(owner, "next", replacement_sender)
                        .unwrap();
                }
                "detachment" => {
                    peers.sessions.detach_writer(&first.seat).unwrap();
                }
                "release" => {
                    assert_eq!(
                        peers
                            .sessions
                            .release_seat_if_owner(first.seat, owner, first.generation),
                        Some(false)
                    );
                }
                _ => unreachable!(),
            }
        }
        let id = SnapshotTransitionId {
            session_id: context.session_id,
            sequence: 1,
        };
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
                awaiting: HashSet::from([first.seat]),
            });
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
                "modal proposal",
                NetMsg::ModalProposal(robin_engine::multiplayer::ModalProposal {
                    instance: ModalInstanceId {
                        session_id: context.session_id,
                        opened_frame: 10,
                        occurrence: 1,
                    },
                    kind: ModalKind::Dialog { dialog_id: 44 },
                    result: DialogResult::Completed,
                    requested_frame: 10,
                }),
            ),
        ];
        for (label, message) in messages {
            let error = super::dispatch_server_peer_message(
                &context,
                PlayerId(first.seat),
                first.generation,
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
            assert!(
                peers
                    .sessions
                    .readiness()
                    .all(|(_, _, ready_frame)| ready_frame.is_none())
            );
            assert!(peers.readiness.begun.is_none());
        }
    }
}

#[test]
fn current_peer_dispatch_publishes_input_and_ready_but_old_generation_cannot() {
    let (context, events) = dispatch_test_context();
    let (sender, mut wire) = unbounded_channel();
    let claim = {
        let mut peers = context.peers.lock();
        let claim = peers
            .sessions
            .claim_seat(PeerOwner::Native([1; 32]), "peer", sender)
            .unwrap();
        peers.sessions.connect_sim_seat(claim.seat);
        peers.readiness.host_frame = Some(10);
        claim
    };
    super::dispatch_server_peer_message(
        &context,
        PlayerId(claim.seat),
        claim.generation,
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
        .claim_seat(PeerOwner::Native([1; 32]), "peer", sender)
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
        super::NetMsg::SnapshotTransitionReady { id },
    )
    .unwrap_err();
    assert!(error.to_string().contains("detached writer"));
    assert!(events.try_recv().is_err());
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
    let owner = PeerOwner::Native([1; 32]);
    let (sender, _old_wire) = unbounded_channel();
    let first = context
        .peers
        .lock()
        .sessions
        .claim_seat(owner, "old", sender)
        .unwrap();
    let (sender, mut next_wire) = unbounded_channel();
    let next = context
        .peers
        .lock()
        .sessions
        .claim_seat(owner, "next", sender)
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
async fn inactive_reader_drains_terminal_frames_after_buffered_input_without_restarting_writer() {
    use robin_engine::multiplayer::{NetMsg, SnapshotTransitionId, SnapshotTransitionPayload};
    for mode in ["commit", "reconnect", "replacement", "release"] {
        let (context, events) = dispatch_test_context();
        let owner = PeerOwner::Native([1; 32]);
        let (sender, mut wire) = unbounded_channel();
        let claim = context
            .peers
            .lock()
            .sessions
            .claim_seat(owner, "old", sender)
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
                            .claim_seat(owner, "successor", sender)
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
        std::future::pending::<Result<(), super::MultiplayerError>>().await
    };
    let reader = async {
        started_rx.await.unwrap();
        Ok(super::PeerReaderExit::Inactive)
    };
    let error = super::drive_server_peer_io(reader, writer, Duration::from_millis(10))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("timed out draining terminal frames")
    );
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
        .claim_seat(owner, "peer", sender.clone())
        .unwrap();
    let reader = std::future::pending::<Result<super::PeerReaderExit, super::MultiplayerError>>();
    let writer = async move {
        receiver.close();
        Err(super::MultiplayerError::LocalState(
            "independent stream write failure".into(),
        ))
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
            .to_string()
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
        .claim_seat(owner, "peer", replacement)
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

fn claim_test_seat(
    peers: &mut ServerPeers,
    seat: u8,
    sender: tokio::sync::mpsc::UnboundedSender<robin_engine::multiplayer::NetMsg>,
) {
    let claim = peers
        .sessions
        .claim_seat(PeerOwner::Native([seat; 32]), "test peer", sender)
        .unwrap();
    assert_eq!(claim.seat, seat);
}

#[test]
fn native_server_gameplay_rejects_wrong_direction_messages() {
    assert!(validate_server_gameplay_wire_msg(&super::NetMsg::Note("legal".into())).is_ok());
    assert!(
        validate_server_gameplay_wire_msg(&super::NetMsg::ContentRequest(
            robin_engine::multiplayer::ContentRequest {
                full_mod_sha256: [1; 32],
                resume_offset: 0,
            }
        ))
        .unwrap_err()
        .to_string()
        .contains("ordinary peer session")
    );
    assert!(
        validate_server_gameplay_wire_msg(&super::NetMsg::StateHash(
            robin_engine::multiplayer::StateHashReport {
                frame: 1,
                hash: Some(2),
                clock_frame: Some(3),
                ms_until_next_frame: Some(4),
            }
        ))
        .unwrap_err()
        .to_string()
        .contains("invalid server-session message")
    );
    assert!(
        validate_server_gameplay_wire_msg(&super::NetMsg::Hello {
            protocol_version: robin_engine::multiplayer::NET_PROTOCOL_VERSION,
            nickname: "late".into(),
            browser_auth: None,
        })
        .unwrap_err()
        .to_string()
        .contains("invalid server-session message")
    );

    assert!(
        validate_server_gameplay_outbound(&super::NetOutbound::StateHash(
            robin_engine::multiplayer::StateHashReport {
                frame: 1,
                hash: Some(2),
                clock_frame: Some(3),
                ms_until_next_frame: Some(4),
            }
        ))
        .is_ok()
    );
    assert!(
        validate_server_gameplay_outbound(&super::NetOutbound::ContentPrepared {
            full_mod_sha256: [1; 32],
        })
        .unwrap_err()
        .to_string()
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
    assert!(error.to_string().contains("host-authoritative"));

    validate_peer_command_authority(
        PlayerId(2),
        &robin_engine::player_command::PlayerCommand::CrouchDown,
    )
    .expect("ordinary seat input remains admissible");
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
    let (_channels, server_channels) = crate::multiplayer::NetChannels::new_server();
    let result = super::start_server_inner(
        &campaign,
        key,
        ServerConfig {
            host_nickname: "host".into(),
            mission_id: "Dem_Lei_MP".into(),
            mission_seed: 42,
            sim_config: robin_engine::engine::SimConfig::default(),
            speech_timing_locale: None,
            expected_players: 3,
            browser_join_enabled: false,
        },
        server_channels,
        None,
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
        .claim_seat(browser, "renamed browser", browser_tx)
        .unwrap();
    let (native_tx, _native_rx) = unbounded_channel();
    let native_claim = peers
        .sessions
        .claim_seat(native, "renamed native", native_tx)
        .unwrap();

    assert_eq!(browser_claim.seat, 1);
    assert_eq!(browser_claim.kind, SeatClaimKind::Reconnect);
    assert_eq!(native_claim.seat, 2);
    assert_eq!(native_claim.kind, SeatClaimKind::Reconnect);
    let (intruder_tx, _intruder_rx) = unbounded_channel();
    assert!(
        peers
            .sessions
            .claim_seat(PeerOwner::Browser([9; 32]), "same nickname", intruder_tx)
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
fn authenticated_owner_reclaims_and_replaces_only_its_original_seat() {
    let mut peers = ServerPeers::new(3);
    let owner = PeerOwner::Browser([7; 32]);
    let other = PeerOwner::Browser([8; 32]);
    let (first_tx, _first_rx) = unbounded_channel();
    let first = peers.sessions.claim_seat(owner, "Robin", first_tx).unwrap();
    let seat = first.seat;
    let generation = first.generation;
    assert_eq!(seat, 1);
    assert_eq!(first.kind, SeatClaimKind::Fresh);
    assert!(peers.sessions.connect_sim_seat(seat));

    let (replacement_tx, _replacement_rx) = unbounded_channel();
    let replacement = peers
        .sessions
        .claim_seat(owner, "Robin renamed", replacement_tx)
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
        .claim_seat(other, "Robin renamed", other_tx)
        .unwrap();
    let other_seat = other_claim.seat;
    assert_eq!(
        other_seat, 2,
        "a matching nickname grants no seat authority"
    );

    let (rejoin_tx, _rejoin_rx) = unbounded_channel();
    let rejoined = peers
        .sessions
        .claim_seat(owner, "New name", rejoin_tx)
        .unwrap();
    let rejoined_seat = rejoined.seat;
    assert_eq!(rejoined_seat, seat);
    assert_eq!(rejoined.kind, SeatClaimKind::Reconnect);
}

#[test]
fn real_iroh_seat_connects_and_ready_begins_gameplay() {
    let (server_in_tx, server_in_rx) = channel();
    let (server_out_tx, server_out_rx) = channel();
    let mut server = start_server_with_key(
        iroh::SecretKey::generate(),
        ServerConfig {
            host_nickname: "host".into(),
            mission_id: "Dem_Lei_MP".into(),
            mission_seed: 7,
            sim_config: robin_engine::engine::SimConfig::default(),
            speech_timing_locale: Some("en-US".into()),
            expected_players: 2,
            browser_join_enabled: false,
        },
        ServerChannels {
            incoming_tx: server_in_tx,
            outgoing_rx: server_out_rx,
            frame_cursor: Arc::new(AtomicU32::new(0)),
            initial_snapshot: Arc::new(StdMutex::new(None)),
        },
        None,
    )
    .expect("start real iroh host");
    let (client_in_tx, client_in_rx) = channel();
    let (client_out_tx, client_out_rx) = channel();
    let mut client = connect_client_with_key(
        iroh::SecretKey::generate(),
        server.connect_string(),
        "alice".into(),
        client_in_tx,
        client_out_rx,
    )
    .expect("connect client");
    recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
        matches!(event, NetEvent::AssignedLocalSeat(PlayerId(1)))
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
    client_out_tx
        .send(NetOutbound::ReadyToSim { frame: 0 })
        .unwrap();
    server_out_tx
        .send(NetOutbound::ReadyToSim { frame: 0 })
        .unwrap();
    recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
        matches!(event, NetEvent::BeginSim { .. })
    });
    client.shutdown();
    server.shutdown();
}
