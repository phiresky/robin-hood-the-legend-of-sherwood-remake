//! Native client transport tests: server-to-client wire handling,
//! outbound gameplay validation, and reconnect state.
use super::super::test_support::{leaderboard_request, signed_response};
use super::{
    SharedClientLeaderboardCoSignState, client_gameplay_wire_msg, discard_session_outbound,
    handle_client_wire_msg, validate_reconnect_state,
};
use robin_engine::player_command::PlayerId;
use robin_run_protocol::LeaderboardCoSignPurposeV1;
use std::sync::Arc;

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
