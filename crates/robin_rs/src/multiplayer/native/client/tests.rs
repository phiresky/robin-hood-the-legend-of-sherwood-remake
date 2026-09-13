//! Native client transport tests: server-to-client wire handling through the
//! shared session handler, outbound gameplay validation, and reconnect state.
use super::super::test_support::{leaderboard_request, signed_response};
use super::{
    ClientRankedAdmission as _, ClientTransport, NativeClientTransport, NativeRankedAdmission,
};
use crate::leaderboard_ranked_session::RankedSessionLifecycle;
use crate::multiplayer::SharedClientLeaderboardCoSignState;
use crate::multiplayer::client_protocol::validate_reconnect_state;
use crate::multiplayer::client_session::tests::{
    assert_premature_begin_sim_downgrades, assert_premature_cosign_request_downgrades, begin_sim,
    handle,
};
use robin_engine::multiplayer::{NetEvent, NetMsg, NetOutbound};
use robin_engine::player_command::PlayerId;
use robin_run_protocol::LeaderboardCoSignPurposeV1;
use std::sync::Arc;
use std::sync::mpsc::Sender;

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
) -> Result<(), String> {
    handle(&admission(), incoming_tx, cosign_state, message)
}

fn client_gameplay_wire_msg(outgoing: NetOutbound) -> Result<NetMsg, String> {
    let (incoming, _receiver) = std::sync::mpsc::channel();
    crate::multiplayer::client_outgoing::prepare(
        outgoing,
        &incoming,
        &Default::default(),
        crate::multiplayer::client_outgoing::ClientPublicationAuthority {
            co_sign_allowed: false,
            durable_public_key: None,
        },
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "outgoing publication has no wire frame".to_owned())
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
        .contains("invalid native session message")
    );
    assert!(
        handle_native(
            &incoming_tx,
            &cosign_state,
            NetMsg::ContentChunk {
                full_mod_sha256: [1; 32],
                offset: 0,
                total_bytes: 1,
                bytes: vec![0],
            },
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
        client_gameplay_wire_msg(NetOutbound::StateHash {
            frame: 1,
            hash: Some(2),
            clock_frame: Some(3),
            ms_until_next_frame: Some(4),
        })
        .unwrap_err()
        .contains("host-only")
    );
    assert!(
        client_gameplay_wire_msg(NetOutbound::ContentReady {
            full_mod_sha256: [1; 32],
        })
        .unwrap_err()
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
    // Before ranked admission the host request is dropped, never exposed.
    handle_native(
        &incoming_tx,
        &state,
        NetMsg::LeaderboardCoSignRequest(request),
    )
    .unwrap();
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
    let error = handle_native(
        &incoming_tx,
        &cosign_state,
        NetMsg::ReconnectRequired {
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
