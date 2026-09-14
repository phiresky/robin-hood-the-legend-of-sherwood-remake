//! Native client transport tests: server-to-client wire handling through the
//! shared session handler, outbound gameplay validation, and reconnect state.
use super::super::test_support::{leaderboard_request, signed_response};
use super::{
    ClientRankedAdmission as _, ClientTransport, NativeClientTransport, NativeRankedAdmission,
};
use crate::leaderboard_ranked_session::RankedSessionLifecycle;
use crate::multiplayer::client_protocol::{ReconnectIdentity, validate_reconnect_state};
use crate::multiplayer::client_session::tests::{
    assert_premature_begin_sim_downgrades, assert_premature_cosign_request_downgrades,
    assert_ranked_violation_downgrades, assert_reconnect_reset_policy, begin_sim, handle,
    invalid_ranked_messages,
};
use crate::multiplayer::{MultiplayerError, SharedClientLeaderboardCoSignState};
use robin_engine::multiplayer::{NetEvent, NetMsg, NetOutbound};
use robin_engine::player_command::PlayerId;
use robin_run_protocol::LeaderboardCoSignPurposeV1;
use std::sync::Arc;
use std::sync::mpsc::Sender;

/// 10/F1 case 1 on the native adapter: the same browse-only downgrade as the
/// browser, now also published as `RankedBrowseOnly` (it used to be a silent
/// lifecycle-only downgrade).
#[test]
fn native_invalid_ranked_messages_downgrade_to_browse_only() {
    for (message, reason) in invalid_ranked_messages() {
        let ranked = admission();
        assert_ranked_violation_downgrades(&ranked, message, reason);
        assert!(!ranked.simulation_release_unresolved().unwrap());
    }
}

/// 10/F1 case 3 on the native adapter.
#[test]
fn native_reconnect_reset_follows_the_shared_policy() {
    assert_reconnect_reset_policy(admission);
}

/// 10/F1 case 2 through the real native transport: a host that finishes its
/// stream cleanly at a frame boundary (no `Reject`) is a transport drop. The
/// client publishes `Disconnected` and reconnects; the old native policy ended
/// the connection there instead.
#[test]
fn clean_host_stream_close_reconnects() {
    use crate::multiplayer::InboundFramePolicy;
    use crate::multiplayer::framing::{read_frame, write_frame};
    use crate::multiplayer::identity::{GAME_ALPN, bind_endpoint};
    use std::time::{Duration, Instant};

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("fake host runtime");
    let endpoint = runtime
        .block_on(bind_endpoint(iroh::SecretKey::generate(), GAME_ALPN))
        .expect("bind fake host endpoint");
    runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(15), endpoint.online()).await })
        .expect("fake host endpoint online");
    let connect = serde_json::to_string(&endpoint.addr()).unwrap();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
    let host = runtime.spawn(async move {
        let welcome = || NetMsg::Welcome {
            your_seat: PlayerId(1),
            mission_id: "Dem_Lei_MP".into(),
            mission_seed: 7,
            sim_config: robin_engine::engine::SimConfig::default(),
            speech_timing_locale: None,
            host_nickname: "host".into(),
            session_id: robin_engine::multiplayer::MultiplayerSessionId([6; 32]),
        };
        let mut streams = Vec::new();
        for attempt in 0..2 {
            let incoming = endpoint.accept().await.expect("client connection");
            let conn = incoming.await.expect("client QUIC handshake");
            let (mut send, mut recv) = conn.accept_bi().await.expect("client game stream");
            let hello = read_frame(&mut recv, InboundFramePolicy::ClientHello)
                .await
                .expect("read Hello");
            assert!(
                matches!(hello, Some(NetMsg::Hello { .. })),
                "attempt {attempt}: {hello:?}"
            );
            write_frame(&mut send, &welcome())
                .await
                .expect("write Welcome");
            if attempt == 0 {
                // A clean FIN at a frame boundary while the connection stays open.
                send.finish().expect("finish host stream");
            }
            streams.push((conn, send, recv));
        }
        // Hold both connections until the client has observed the reconnect.
        let _ = done_rx.await;
        drop(streams);
        endpoint.close().await;
    });

    let (client_in_tx, client_in_rx) = std::sync::mpsc::channel();
    let (_client_out_tx, client_out_rx) = std::sync::mpsc::channel();
    let mut client = super::connect_client_with_key(
        iroh::SecretKey::generate(),
        connect,
        "alice".into(),
        client_in_tx,
        client_out_rx,
    )
    .expect("connect to fake host");
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut observed = Vec::new();
    let mut disconnected = false;
    loop {
        let event = client_in_rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| {
                panic!(
                    "no reconnect after a clean host stream close: {error}; observed {observed:?}"
                )
            });
        match &event {
            NetEvent::Disconnected => disconnected = true,
            NetEvent::Reconnected => {
                assert!(disconnected, "Reconnected before Disconnected");
                break;
            }
            NetEvent::Fatal(error) => panic!("clean host stream close ended the session: {error}"),
            _ => {}
        }
        observed.push(format!("{event:?}"));
    }
    client.shutdown();
    let _ = done_tx.send(());
    runtime.block_on(host).expect("fake host task");
}

/// A welcomed native admission with no prepared ranked setup yet.
fn admission() -> NativeRankedAdmission {
    let (_setup_tx, setup_rx) = crate::multiplayer::client_session::ranked_setup_channel();
    let ranked = NativeRankedAdmission::new(
        Arc::new(std::sync::Mutex::new(
            RankedSessionLifecycle::awaiting_prepared_inputs(),
        )),
        setup_rx,
        None,
        [3; 32],
        [4; 32],
    );
    ranked.welcomed(PlayerId(1));
    ranked
}

fn handle_native(
    incoming_tx: &Sender<NetEvent>,
    cosign_state: &SharedClientLeaderboardCoSignState,
    message: NetMsg,
) -> Result<(), MultiplayerError> {
    handle(&admission(), incoming_tx, cosign_state, message)
}

fn client_gameplay_wire_msg(outgoing: NetOutbound) -> Result<NetMsg, MultiplayerError> {
    let (incoming, _receiver) = std::sync::mpsc::channel();
    crate::multiplayer::client_outgoing::prepare(
        outgoing,
        &incoming,
        &Default::default(),
        crate::multiplayer::client_outgoing::ClientPublicationAuthority {
            co_sign_allowed: false,
            durable_public_key: None,
        },
    )?
    .ok_or_else(|| MultiplayerError::LocalState("outgoing publication has no wire frame".into()))
}

#[test]
fn native_premature_begin_sim_downgrades_to_browse_only() {
    assert_premature_begin_sim_downgrades(&admission());
}

#[test]
fn native_premature_cosign_request_downgrades_to_browse_only() {
    assert_premature_cosign_request_downgrades(&admission());
}

#[test]
fn begin_sim_requires_a_live_local_receiver() {
    let (tx, rx) = std::sync::mpsc::channel();
    let state = Arc::new(Default::default());
    drop(rx);
    assert!(
        handle_native(&tx, &state, begin_sim())
            .unwrap_err()
            .to_string()
            .contains("channel is closed")
    );
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
fn native_gameplay_rejects_late_content_and_opening_messages() {
    let (incoming_tx, incoming_rx) = std::sync::mpsc::channel();
    let cosign_state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
    assert!(
        handle_native(
            &incoming_tx,
            &cosign_state,
            NetMsg::ContentOffer { offer: offer() },
        )
        .unwrap_err()
        .to_string()
        .contains("invalid native session message")
    );
    assert!(
        handle_native(
            &incoming_tx,
            &cosign_state,
            NetMsg::ContentChunk(robin_engine::multiplayer::ContentChunk {
                full_mod_sha256: [1; 32],
                offset: 0,
                total_bytes: 1,
                bytes: vec![0],
            }),
        )
        .is_err()
    );
    assert!(
        handle_native(
            &incoming_tx,
            &cosign_state,
            NetMsg::Reject {
                reason: "session revoked".into(),
            },
        )
        .unwrap_err()
        .to_string()
        .contains("session revoked")
    );
    assert!(
        handle_native(
            &incoming_tx,
            &cosign_state,
            NetMsg::Welcome {
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
        .to_string()
        .contains("invalid native session message")
    );
    assert!(handle_native(&incoming_tx, &cosign_state, NetMsg::Note("legal".into())).is_ok());
    assert!(matches!(
        incoming_rx.recv().unwrap(),
        NetEvent::Note(note) if note == "legal"
    ));
}

#[test]
fn native_gameplay_rejects_host_only_and_late_content_outbound() {
    assert!(
        client_gameplay_wire_msg(NetOutbound::StateHash(
            robin_engine::multiplayer::StateHashReport {
                frame: 1,
                hash: Some(2),
                clock_frame: Some(3),
                ms_until_next_frame: Some(4),
            }
        ))
        .unwrap_err()
        .to_string()
        .contains("host-only")
    );
    assert!(
        client_gameplay_wire_msg(NetOutbound::ContentReady {
            full_mod_sha256: [1; 32],
        })
        .unwrap_err()
        .to_string()
        .contains("after gameplay began")
    );
    assert!(matches!(
        client_gameplay_wire_msg(NetOutbound::ReadyToSim { frame: 7 }).unwrap(),
        NetMsg::ReadyToSim { frame: 7 }
    ));
}

#[test]
fn client_wire_handler_never_exposes_unarmed_or_wrong_direction_cosign() {
    let request = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 73);
    let state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
    let (incoming_tx, incoming_rx) = std::sync::mpsc::channel();
    // Before ranked admission the host request is dropped, never exposed; only
    // the resulting browse-only downgrade is published.
    handle_native(
        &incoming_tx,
        &state,
        NetMsg::LeaderboardCoSignRequest(request),
    )
    .unwrap();
    assert!(matches!(
        incoming_rx.try_recv(),
        Ok(NetEvent::RankedBrowseOnly { .. })
    ));
    assert!(incoming_rx.try_recv().is_err());
    // The co-sign gate itself exposes only an exact locally armed request.
    assert_eq!(state.receive_wire_request(request).unwrap(), None);
    assert_eq!(state.arm_request(request).unwrap(), Some(request));

    let response = signed_response(&request, &iroh::SecretKey::generate());
    assert!(
        handle_native(
            &incoming_tx,
            &state,
            NetMsg::LeaderboardCoSignResponse(response),
        )
        .unwrap_err()
        .to_string()
        .contains("client-only")
    );
}

#[test]
fn reconnect_rejects_wrong_session_mission_config_or_speech_locale() {
    fn identity(
        seat: PlayerId,
        mission_id: &'static str,
        config: robin_engine::engine::SimConfig,
        speech_timing_locale: Option<&'static str>,
        session: u8,
    ) -> ReconnectIdentity<'static> {
        ReconnectIdentity {
            seat,
            mission_id,
            seed: 7,
            config,
            speech_timing_locale,
            session_id: robin_engine::multiplayer::MultiplayerSessionId([session; 32]),
        }
    }
    let expected = robin_engine::engine::SimConfig::default();
    assert!(
        validate_reconnect_state(
            identity(PlayerId(1), "MissionA", expected, Some("en-US"), 1),
            identity(PlayerId(1), "MissionB", expected, Some("en-US"), 1),
        )
        .is_err()
    );

    let mut changed = expected;
    changed.amount_of_speaking = 9;
    assert!(
        validate_reconnect_state(
            identity(PlayerId(1), "MissionA", expected, Some("en-US"), 1),
            identity(PlayerId(1), "MissionA", changed, Some("en-US"), 1),
        )
        .is_err()
    );
    assert!(
        validate_reconnect_state(
            identity(PlayerId(1), "MissionA", expected, Some("en-US"), 1),
            identity(PlayerId(2), "MissionA", expected, Some("en-US"), 1),
        )
        .is_err()
    );
    assert!(
        validate_reconnect_state(
            identity(PlayerId(1), "MissionA", expected, Some("en-US"), 1),
            identity(PlayerId(1), "MissionA", expected, Some("de-DE"), 1),
        )
        .is_err()
    );
    assert!(
        validate_reconnect_state(
            identity(PlayerId(1), "MissionA", expected, Some("en-US"), 1),
            identity(PlayerId(1), "MissionA", expected, Some("en-US"), 2),
        )
        .is_err()
    );
    assert!(
        validate_reconnect_state(
            identity(PlayerId(1), "MissionA", expected, Some("en-US"), 1),
            identity(PlayerId(1), "MissionA", expected, Some("en-US"), 1),
        )
        .is_ok()
    );
}

#[test]
fn host_reconnect_directive_ends_the_complete_client_session() {
    let (incoming_tx, _incoming_rx) = std::sync::mpsc::channel();
    let cosign_state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
    let error = handle_native(
        &incoming_tx,
        &cosign_state,
        NetMsg::ReconnectRequired {
            reason: "late input predates rollback horizon".to_string(),
        },
    )
    .expect_err("directive must unwind the session into the reconnect loop");
    assert!(matches!(error, MultiplayerError::ReconnectRequired { .. }));
    assert!(error.to_string().contains("full-snapshot reconnect"));
    assert!(error.to_string().contains("rollback horizon"));
}

#[test]
fn reconnect_discards_commands_queued_for_abandoned_session() {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    sender
        .send(NetOutbound::Input {
            origin_frame: 41,
            command: robin_engine::player_command::PlayerCommand::CrouchDown,
        })
        .expect("queue old-session command");
    sender
        .send(NetOutbound::ReadyToSim { frame: 40 })
        .expect("queue old-session readiness");

    assert_eq!(
        <NativeClientTransport<'_> as ClientTransport>::discard_outbound(&mut receiver),
        2
    );
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
    handle_native(
        &incoming_tx,
        &cosign_state,
        NetMsg::PrepareSnapshotTransition {
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
        NetEvent::PrepareSnapshotTransition {
            id: decoded_id,
            payload: robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                mission_id: 71,
                save_bytes: decoded_bytes,
            },
        } if decoded_id == id && decoded_bytes == save_bytes
    ));

    handle_native(
        &incoming_tx,
        &cosign_state,
        NetMsg::CommitSnapshotTransition { id },
    )
    .unwrap();
    assert!(matches!(
        incoming_rx.recv().unwrap(),
        NetEvent::CommitSnapshotTransition { id: decoded_id }
            if decoded_id == id
    ));
}
