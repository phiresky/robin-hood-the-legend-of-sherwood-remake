//! Multiplayer transport — iroh (peer-to-peer QUIC) server / client.
//!
//! The wire-format types ([`robin_engine::multiplayer::NetMsg`], [`NetEvent`], [`NetOutbound`])
//! and protocol constants live in
//! [`robin_engine::multiplayer`] so [`robin_engine::engine_manager::EngineManager`]
//! can route mutations through the rollback-safe path. This module wraps
//! the engine channel bundle in [`NetChannels`] so the channels and their
//! platform-specific [`MultiplayerRuntime`] have one owner and one lifetime.

mod clock;
mod error;

pub use clock::{ClockError, current_epoch_ms};
pub use error::{MessageError, MultiplayerError, SharedError};
use robin_engine::multiplayer::NetChannels as EngineNetChannels;
pub(crate) use robin_engine::multiplayer::{
    FrameCursor, InitialSnapshot, NetEvent, NetOutbound, STATE_HASH_INTERVAL,
};
use std::ops::Deref;
use std::sync::mpsc::{Receiver, Sender};

pub mod content_identity;

/// Most players one multiplayer session admits, host included. Browser join
/// tickets and the native host both bound `expected_players` by it.
pub const MAX_MULTIPLAYER_PLAYERS: u32 = 4;

// The one feature gate of the transport: `enabled.rs` mounts the iroh
// server/client (native) or relay client (browser); `disabled.rs` provides the
// same names as inert stand-ins.
#[cfg(feature = "multiplayer")]
#[path = "multiplayer/enabled.rs"]
mod transport;
#[cfg(not(feature = "multiplayer"))]
#[path = "multiplayer/disabled.rs"]
mod transport;
pub use transport::*;

/// Game-loop channels coupled to the runtime that services them.
///
/// Field order is intentional: the engine channel senders are dropped before
/// the runtime, then runtime shutdown joins workers after their channel ends
/// have closed.
pub struct NetChannels {
    channels: EngineNetChannels,
    runtime: Option<MultiplayerRuntime>,
}

impl NetChannels {
    /// Build an unattached channel bundle. The caller must attach the runtime
    /// returned by [`start_server_in_campaign`] or [`connect_client`] before publishing the
    /// bundle to the game loop.
    pub fn new() -> (
        Self,
        Sender<NetEvent>,
        Receiver<NetOutbound>,
        FrameCursor,
        InitialSnapshot,
    ) {
        let (channels, incoming_tx, outgoing_rx, frame_cursor, initial_snapshot) =
            EngineNetChannels::new();
        (
            Self {
                channels,
                runtime: None,
            },
            incoming_tx,
            outgoing_rx,
            frame_cursor,
            initial_snapshot,
        )
    }

    /// Couple the channel bundle to its transport owner.
    pub fn attach_runtime(&mut self, runtime: impl Into<MultiplayerRuntime>) {
        assert!(
            self.runtime.is_none(),
            "multiplayer channels already have an attached runtime"
        );
        // Convert inside `map`: without the feature `MultiplayerRuntime` is
        // uninhabited, and a direct `Some(runtime.into())` is linted unreachable.
        self.runtime = Some(runtime).map(Into::into);
    }

    /// Explicitly stop and detach the transport. Drop performs the same work.
    pub fn shutdown(&mut self) {
        if let Some(mut runtime) = self.runtime.take() {
            runtime.shutdown();
        }
    }

    /// Retain the authenticated host session/seat roster when this mission's
    /// transport shuts down. Clients require no explicit flag: their durable
    /// browser owner or process-held native key reclaims the retained seat.
    pub(crate) fn preserve_session_for_next_mission(&mut self) {
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.preserve_session_for_next_mission();
        }
    }
}

impl Deref for NetChannels {
    type Target = EngineNetChannels;

    fn deref(&self) -> &Self::Target {
        &self.channels
    }
}

#[cfg(all(test, feature = "multiplayer", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::distributed_mod::DistributedModPackage;
    use crate::multiplayer::native::{
        HostedModContent, connect_client, connect_client_with_key, start_server_with_key,
    };
    use robin_engine::multiplayer::new_frame_cursor;
    use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn start_owned_server() -> (NetChannels, String) {
        let (mut channels, server_channels) = NetChannels::new_server();
        let handle = start_server_with_key(
            iroh::SecretKey::generate(),
            ServerConfig {
                host_nickname: "host".into(),
                mission_id: "Dem_Lei_MP".into(),
                mission_seed: 42,
                sim_config: robin_engine::engine::SimConfig::default(),
                speech_timing_locale: Some("en-US".into()),
                expected_players: 1,
                browser_join_enabled: false,
            },
            server_channels,
            None,
        )
        .expect("start server on an ephemeral iroh identity");
        let connect_string = handle.connect_string();
        channels.attach_runtime(handle);
        (channels, connect_string)
    }

    #[test]
    fn dropping_owned_runtime_joins_workers() {
        let (channels, _connect_string) = start_owned_server();
        let (done_tx, done_rx) = channel();

        let shutdown_thread = std::thread::spawn(move || {
            drop(channels);
            done_tx.send(()).expect("report completed shutdown");
        });

        done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("runtime drop must close the endpoint and join its workers");
        shutdown_thread
            .join()
            .expect("runtime shutdown test worker must not panic");
    }

    #[test]
    fn server_client_input_roundtrip() {
        // Server side.
        let (server_in_tx, server_in_rx) = channel::<NetEvent>();
        let (_server_out_tx, server_out_rx) = channel::<NetOutbound>();
        let server_cursor = new_frame_cursor();
        let server_snapshot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let expected_config = robin_engine::engine::SimConfig {
            amount_of_speaking: 9,
            ..Default::default()
        };
        let _server = start_server_with_key(
            iroh::SecretKey::generate(),
            ServerConfig {
                host_nickname: "host".into(),
                mission_id: "Dem_Lei_MP".into(),
                mission_seed: 42,
                sim_config: expected_config,
                speech_timing_locale: Some("en-US".into()),
                expected_players: 2,
                browser_join_enabled: false,
            },
            ServerChannels {
                incoming_tx: server_in_tx,
                outgoing_rx: server_out_rx,
                frame_cursor: server_cursor,
                initial_snapshot: server_snapshot,
            },
            None,
        )
        .expect("start_server");
        let addr = _server.connect_string();

        // Client side.
        let (client_in_tx, client_in_rx) = channel::<NetEvent>();
        let (client_out_tx, client_out_rx) = channel::<NetOutbound>();
        let _client = connect_client(&addr, "alice".into(), client_in_tx, client_out_rx)
            .expect("connect_client");
        let session = _client
            .session_metadata()
            .expect("complete Welcome publication");
        assert_eq!(session.seat, PlayerId(1));
        assert_eq!(session.mission_id, "Dem_Lei_MP");
        assert_eq!(session.mission_seed, 42);
        assert_eq!(session.sim_config, expected_config);
        assert_eq!(session.speech_timing_locale.as_deref(), Some("en-US"));
        assert_eq!(session.session_id, _server.session_id());
        assert!(session.admitted_content.is_none());

        let assigned = loop {
            match client_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::AssignedLocalSeat(p)) => break p,
                Ok(NetEvent::Note(_)) => continue,
                Ok(other) => panic!("unexpected pre-handshake event {other:?}"),
                Err(e) => panic!("timeout waiting for AssignedLocalSeat: {e}"),
            }
        };
        assert_eq!(assigned, PlayerId(1));

        let mut saw_join = false;
        for _ in 0..16 {
            match server_in_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(NetEvent::Input { input, .. }) => {
                    if let PlayerCommand::ConnectSeat {
                        player_id,
                        ref nickname,
                        ..
                    } = input.command
                        && player_id == PlayerId(1)
                        && nickname == "alice"
                    {
                        saw_join = true;
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert!(
            saw_join,
            "server should have folded a ConnectSeat for the new client"
        );

        client_out_tx
            .send(NetOutbound::Input {
                origin_frame: 0,
                command: PlayerCommand::CrouchDown,
            })
            .unwrap();

        let (server_input, server_target) = loop {
            match server_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::Input {
                    input,
                    target_frame,
                    ..
                }) if matches!(input.command, PlayerCommand::CrouchDown) => {
                    break (input, target_frame);
                }
                Ok(_) => continue,
                Err(e) => panic!("timeout waiting for server-side input echo: {e}"),
            }
        };
        assert_eq!(server_input.player_id, PlayerId(1));
        assert_eq!(server_target, INPUT_DELAY_FRAMES);

        let client_seen = loop {
            match client_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::Input { input, .. })
                    if matches!(input.command, PlayerCommand::CrouchDown) =>
                {
                    break input;
                }
                Ok(_) => continue,
                Err(e) => panic!("timeout waiting for client-side input echo: {e}"),
            }
        };
        assert_eq!(client_seen.player_id, PlayerId(1));

        let _ = (PlayerInput::new(PlayerId(0), PlayerCommand::CrouchDown),);
    }

    #[test]
    fn host_content_preflight_uses_no_seat_then_exact_resume_reconnects() {
        let package = test_distributed_mod();
        let encoded = package.package.encode().expect("encode test full mod");
        let expected_hash = package.package.manifest.full_mod_sha256;
        let hosted = HostedModContent::from_encoded(encoded.clone()).expect("hosted content");

        let (server_in_tx, server_in_rx) = channel::<NetEvent>();
        let (_server_out_tx, server_out_rx) = channel::<NetOutbound>();
        let server_key = iroh::SecretKey::generate();
        let server_cursor = new_frame_cursor();
        let server_snapshot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let _server = start_server_with_key(
            server_key,
            ServerConfig {
                host_nickname: "host".into(),
                mission_id: "TestMission".into(),
                mission_seed: 42,
                sim_config: robin_engine::engine::SimConfig::default(),
                speech_timing_locale: None,
                expected_players: 2,
                browser_join_enabled: false,
            },
            ServerChannels {
                incoming_tx: server_in_tx,
                outgoing_rx: server_out_rx,
                frame_cursor: server_cursor,
                initial_snapshot: server_snapshot,
            },
            Some(hosted),
        )
        .expect("start content server");

        let client_key = iroh::SecretKey::generate();
        let (client_in_tx, client_in_rx) = channel::<NetEvent>();
        let (client_out_tx, client_out_rx) = channel::<NetOutbound>();
        let mut client = connect_client_with_key(
            client_key.clone(),
            _server.connect_string(),
            "alice".into(),
            client_in_tx,
            client_out_rx,
        )
        .expect("connect through content prelude");
        let offer = client
            .content_offer()
            .expect("content offer before Welcome");
        assert_eq!(offer.full_mod_sha256, expected_hash);
        assert!(client.session_metadata().is_none());
        assert!(matches!(
            client_in_rx.recv_timeout(Duration::from_secs(2)),
            Ok(NetEvent::ContentOffer(seen)) if seen == offer
        ));

        client_out_tx
            .send(NetOutbound::ContentRequest(
                robin_engine::multiplayer::ContentRequest {
                    full_mod_sha256: expected_hash,
                    resume_offset: 0,
                },
            ))
            .expect("accept exact content");
        let mut downloaded = Vec::new();
        while downloaded.len() < encoded.len() {
            match client_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::ContentChunk(robin_engine::multiplayer::ContentChunk {
                    full_mod_sha256,
                    offset,
                    total_bytes,
                    bytes,
                })) => {
                    assert_eq!(full_mod_sha256, expected_hash);
                    assert_eq!(offset as usize, downloaded.len());
                    assert_eq!(total_bytes as usize, encoded.len());
                    downloaded.extend_from_slice(&bytes);
                }
                Ok(other) => panic!("unexpected pre-admission event {other:?}"),
                Err(error) => panic!("content transfer timed out: {error}"),
            }
        }
        assert_eq!(downloaded, encoded);
        DistributedModPackage::decode(&downloaded).expect("downloaded exact package validates");
        assert!(
            client.session_metadata().is_none(),
            "Welcome must still be gated"
        );

        client_out_tx
            .send(NetOutbound::ContentPrepared {
                full_mod_sha256: expected_hash,
            })
            .expect("verified content prepared without a seat");
        assert!(matches!(
            client_in_rx.recv_timeout(Duration::from_secs(2)),
            Ok(NetEvent::Note(note)) if note.contains("without joining a gameplay seat")
        ));
        assert!(client.session_metadata().is_none());
        client.shutdown();

        assert!(
            server_in_rx
                .recv_timeout(Duration::from_millis(200))
                .is_err(),
            "content preflight must not allocate or announce a gameplay seat"
        );

        let (join_in_tx, join_in_rx) = channel::<NetEvent>();
        let (join_out_tx, join_out_rx) = channel::<NetOutbound>();
        let join = connect_client_with_key(
            client_key,
            _server.connect_string(),
            "alice".into(),
            join_in_tx,
            join_out_rx,
        )
        .expect("reconnect after exact content preflight");
        assert!(matches!(
            join_in_rx.recv_timeout(Duration::from_secs(2)),
            Ok(NetEvent::ContentOffer(seen)) if seen == offer
        ));
        join_out_tx
            .send(NetOutbound::ContentRequest(
                robin_engine::multiplayer::ContentRequest {
                    full_mod_sha256: expected_hash,
                    resume_offset: encoded.len() as u64,
                },
            ))
            .expect("resume from exact complete package");
        join_out_tx
            .send(NetOutbound::ContentReady {
                full_mod_sha256: expected_hash,
            })
            .expect("exact mounted content ready");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while join.session_metadata().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            join.session_metadata()
                .expect("admitted Welcome")
                .mission_id,
            "TestMission"
        );
        assert!(matches!(
            join_in_rx.recv_timeout(Duration::from_secs(2)),
            Ok(NetEvent::AssignedLocalSeat(PlayerId(1))) | Ok(NetEvent::MissionConfig { .. })
        ));
    }

    fn test_distributed_mod() -> crate::distributed_mod::ValidatedDistributedMod {
        use std::io::{Cursor, Write};

        let mut rhm = Vec::new();
        rhm.extend_from_slice(b"DUTY");
        rhm.extend_from_slice(&0u32.to_le_bytes());
        rhm.extend_from_slice(&2u32.to_le_bytes());
        rhm.extend_from_slice(b"FOOT");
        rhm.extend_from_slice(&0u32.to_le_bytes());
        rhm.extend_from_slice(&4u32.to_le_bytes());
        rhm.extend_from_slice(&0u32.to_le_bytes());
        rhm.extend_from_slice(&5u32.to_le_bytes());
        rhm.extend_from_slice(&7u16.to_le_bytes());
        rhm.extend_from_slice(b"TestMap");
        rhm.extend_from_slice(&0u32.to_le_bytes());

        let cursor = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(cursor);
        zip.start_file(
            "Data/Levels/TestMission.rhm",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(&rhm).unwrap();
        zip.start_file(
            "Data/Levels/TestMap.rhp",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"map-art-audio-text-fixture").unwrap();
        let mission_archive = zip.finish().unwrap().into_inner();
        DistributedModPackage::build(
            "test-mod".into(),
            "Test Mod".into(),
            "Test Author".into(),
            "1".into(),
            "https://example.invalid/test".into(),
            "CC0-1.0".into(),
            "TestMission".into(),
            "Data/Levels/TestMission.rhm".into(),
            "TestMap".into(),
            false,
            mission_archive,
            None,
        )
        .expect("build test full mod")
    }

    #[test]
    fn server_releases_begin_only_after_snapshot_and_both_ready_messages() {
        let (server_in_tx, server_in_rx) = channel::<NetEvent>();
        let (server_out_tx, server_out_rx) = channel::<NetOutbound>();
        let server_cursor = new_frame_cursor();
        let server_snapshot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let _server = start_server_with_key(
            iroh::SecretKey::generate(),
            ServerConfig {
                host_nickname: "host".into(),
                mission_id: "Dem_Lei_MP".into(),
                mission_seed: 42,
                sim_config: robin_engine::engine::SimConfig::default(),
                speech_timing_locale: Some("en-US".into()),
                expected_players: 2,
                browser_join_enabled: false,
            },
            ServerChannels {
                incoming_tx: server_in_tx,
                outgoing_rx: server_out_rx,
                frame_cursor: server_cursor,
                initial_snapshot: server_snapshot,
            },
            None,
        )
        .expect("start_server");
        let (client_in_tx, client_in_rx) = channel::<NetEvent>();
        let (client_out_tx, client_out_rx) = channel::<NetOutbound>();
        let _client = connect_client(
            _server.connect_string(),
            "alice".into(),
            client_in_tx,
            client_out_rx,
        )
        .expect("connect_client");

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "client did not receive seat assignment"
            );
            if matches!(
                client_in_rx.recv_timeout(Duration::from_millis(50)),
                Ok(NetEvent::AssignedLocalSeat(PlayerId(1)))
            ) {
                break;
            }
        }

        server_out_tx
            .send(NetOutbound::InitialSnapshot {
                frame: 0,
                engine_bytes: vec![1, 2, 3, 4],
            })
            .expect("publish snapshot");
        let snapshot_deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            assert!(
                std::time::Instant::now() < snapshot_deadline,
                "joining peer did not receive initial snapshot"
            );
            match client_in_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(NetEvent::InitialSnapshot {
                    frame: 0,
                    engine_bytes,
                }) => {
                    assert_eq!(engine_bytes, [1, 2, 3, 4]);
                    break;
                }
                Ok(NetEvent::BeginSim { .. }) => {
                    panic!("BeginSim arrived before the joining peer was ready")
                }
                Ok(_) | Err(_) => {}
            }
        }

        client_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .expect("peer ready");
        let no_begin_deadline = std::time::Instant::now() + Duration::from_millis(150);
        while std::time::Instant::now() < no_begin_deadline {
            if matches!(
                client_in_rx.recv_timeout(Duration::from_millis(20)),
                Ok(NetEvent::BeginSim { .. })
            ) {
                panic!("BeginSim arrived before the delayed host readiness");
            }
        }

        server_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .expect("host ready");
        let server_begin = loop {
            match server_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                }) => break (frame, start_epoch_ms),
                Ok(_) => continue,
                Err(error) => panic!("host did not receive BeginSim: {error}"),
            }
        };
        let client_begin = loop {
            match client_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                }) => break (frame, start_epoch_ms),
                Ok(_) => continue,
                Err(error) => panic!("peer did not receive BeginSim: {error}"),
            }
        };

        assert_eq!(server_begin, client_begin);
        assert_eq!(server_begin.0, 0);
    }
}
