//! Multiplayer wire-format types and channel plumbing.
//!
//! This module defines the platform-pure layer of multiplayer
//! infrastructure: the wire-format enums (`NetMsg`, `NetEvent`,
//! `NetOutbound`), the cross-thread channel bundle (`NetChannels`), and
//! the protocol constants.  The actual transport (websocket I/O via
//! `tungstenite` on native, `web_sys::WebSocket` on wasm) lives in
//! `robin_rs::multiplayer::{native, wasm}` and feeds events into these
//! channels.
//!
//! `EngineManager` (this crate) owns a `NetChannels` and uses it to
//! route locally-sourced player commands over the wire and drain
//! peer-sourced inputs back into the engine at the correct frames.

use crate::engine::Engine;
use crate::player_command::{DialogResult, ModalKind, PlayerCommand, PlayerId, PlayerInput};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

/// Cross-thread snapshot of the local game loop's current sim frame.
///
/// Updated by the game loop at the top of every tick (just after
/// rewind/auto-replay accounting).  Read by the server's broadcast
/// pump and per-peer reader threads to stamp `BroadcastInput` with a
/// fresh `target_frame` so every peer applies the input at the same
/// frame.  An `Arc<AtomicU32>` is the simplest thread-safe handoff —
/// the rate is one update per tick (25 Hz) and reads happen at most
/// per inbound input frame.
pub type FrameCursor = Arc<AtomicU32>;

/// Shared initial-state snapshot offered by the host to joining peers.
pub type InitialSnapshot = Arc<Mutex<Option<(u32, Engine)>>>;

/// Make a new [`FrameCursor`] starting at frame 0.
pub fn new_frame_cursor() -> FrameCursor {
    Arc::new(AtomicU32::new(0))
}

/// Number of frames of "input delay" the server adds when stamping
/// peer inputs with a target frame.  At 25 Hz this is ~80 ms.  The rollback path picks
/// up the slack on slower links by rewinding when an input arrives
/// late.  Tuneable; mirrors the `MAX_INPUT_DELAY` constant in classic
/// GGPO-style netcode.
pub const INPUT_DELAY_FRAMES: u32 = 2;

/// Wire-format protocol version. Bump on any breaking change to [`NetMsg`] or
/// an engine snapshot carried by it. Both sides exchange this in the
/// handshake; mismatches abort the connection.
pub const NET_PROTOCOL_VERSION: u32 = 26;

/// Default TCP port for the multiplayer server.
pub const DEFAULT_PORT: u16 = 7878;

/// Frame cadence at which the host samples its engine state hash and
/// broadcasts it for clients to verify against.  Matches the replay
/// recorder's `frame % 25 == 0` cadence (one hash per simulated
/// second at 25 Hz) so the same sampling point is reused.
pub const STATE_HASH_INTERVAL: u32 = 25;

/// One on-the-wire message.  Encoded as a bitcode binary blob inside
/// each WebSocket frame.
#[derive(Clone, Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub enum NetMsg {
    /// Client → server: opening handshake.
    Hello {
        protocol_version: u32,
        nickname: String,
    },
    /// Server → client: handshake response.  Tells the client which
    /// seat it owns and gives it the mission seed it must use to
    /// initialise its sim deterministically.
    Welcome {
        your_seat: PlayerId,
        mission_id: String,
        mission_seed: u64,
        sim_config: crate::engine::SimConfig,
        host_nickname: String,
    },
    /// Client → server: an input the client wants applied this tick,
    /// tagged with the sender's local frame at dispatch time.  The
    /// server uses `origin_frame` as a lower bound when assigning the
    /// shared target frame so a slightly-ahead client does not receive
    /// its own input in the past on localhost.
    Input {
        origin_frame: u32,
        command: PlayerCommand,
    },
    /// Server → all peers: a tagged input ready for engine dispatch
    /// at `target_frame`.
    BroadcastInput {
        /// Server/host sim frame observed when this input was stamped.
        server_frame: u32,
        /// Sender's local sim frame at dispatch time.
        origin_frame: u32,
        target_frame: u32,
        input: PlayerInput,
    },
    /// Either direction, advisory.
    Note(String),
    /// Server → all peers: deterministic engine state hash at the
    /// start of `frame` (pre-tick), broadcast every
    /// [`STATE_HASH_INTERVAL`] frames.
    StateHash {
        frame: u32,
        hash: Option<u64>,
        clock_frame: Option<u32>,
        ms_until_next_frame: Option<u32>,
    },
    /// Server → newly-handshaking peer: an authoritative engine
    /// snapshot for mid-mission joins. `engine_bytes` uses native bitcode.
    InitialSnapshot { frame: u32, engine_bytes: Vec<u8> },
    /// Client → server: this peer has loaded the mission, installed
    /// the host snapshot, and is ready to enter the synchronized sim.
    ReadyToSim { frame: u32 },
    /// Server → all peers: every expected player is loaded and ready;
    /// begin simulating `frame` at this wall-clock timestamp.
    BeginSim { frame: u32, start_epoch_ms: u64 },
    /// Either direction: a blocking modal was dismissed.  This is
    /// immediate UI synchronization rather than a sim-frame command,
    /// because modal loops block the normal per-frame command drain.
    ModalDismiss {
        kind: ModalKind,
        result: DialogResult,
    },
}

/// One incoming wire event ready for the game loop.
#[derive(Clone, Debug)]
pub enum NetEvent {
    /// A peer's input arrived, ready to apply at `target_frame`.
    Input {
        server_frame: u32,
        origin_frame: u32,
        target_frame: u32,
        input: PlayerInput,
    },
    /// The server (or our own client connection) has decided we own
    /// this seat in the simulation.
    AssignedLocalSeat(PlayerId),
    /// Best-effort diagnostic from the network layer.
    Note(String),
    /// The connection ended.
    Disconnected,
    /// I/O thread successfully re-handshook with the server after a
    /// drop.  Followed by a fresh `AssignedLocalSeat`.
    Reconnected,
    /// Authoritative state hash and/or clock sample from the host at `frame`.
    PeerStateHash {
        frame: u32,
        hash: Option<u64>,
        clock_frame: Option<u32>,
        ms_until_next_frame: Option<u32>,
    },
    /// Mission construction state announced by the server in `Welcome`.
    /// Only the wasm path emits this; native captures it synchronously.
    MissionConfig {
        mission_id: String,
        rng_seed: u64,
        sim_config: crate::engine::SimConfig,
    },
    /// Unrecoverable transport/session compatibility failure.
    Fatal(String),
    /// Authoritative initial-state snapshot from the host.
    InitialSnapshot { frame: u32, engine_bytes: Vec<u8> },
    /// The server released the multiplayer start barrier.
    BeginSim { frame: u32, start_epoch_ms: u64 },
    /// A peer dismissed a blocking modal.
    ModalDismiss {
        kind: ModalKind,
        result: DialogResult,
    },
}

/// What the game loop pushes into the outgoing channel.
#[derive(Clone, Debug)]
pub enum NetOutbound {
    Input {
        origin_frame: u32,
        command: PlayerCommand,
    },
    StateHash {
        frame: u32,
        hash: Option<u64>,
        clock_frame: Option<u32>,
        ms_until_next_frame: Option<u32>,
    },
    InitialSnapshot {
        frame: u32,
        engine_bytes: Vec<u8>,
    },
    ReadyToSim {
        frame: u32,
    },
    ModalDismiss {
        kind: ModalKind,
        result: DialogResult,
    },
}

/// Channel pair + frame cursor held by the [`crate::engine_manager::EngineManager`].
pub struct NetChannels {
    pub outgoing: Sender<NetOutbound>,
    pub incoming: Receiver<NetEvent>,
    pub deferred_events: Arc<Mutex<std::collections::VecDeque<NetEvent>>>,
    pub frame_cursor: FrameCursor,
    /// Latest authoritative engine snapshot the host wants to share
    /// with newly-handshaking peers.  Set once after mission init via
    /// [`Self::set_initial_snapshot`]; the server's handshake handler
    /// reads it and sends `NetMsg::InitialSnapshot` to each new peer
    /// immediately after `Welcome`.
    pub initial_snapshot: InitialSnapshot,
}

impl NetChannels {
    /// Build the channels + cursor.  Returns `(NetChannels,
    /// incoming_tx, outgoing_rx, frame_cursor, snapshot_arc)`; the
    /// transport thread keeps the latter four.
    pub fn new() -> (
        Self,
        Sender<NetEvent>,
        Receiver<NetOutbound>,
        FrameCursor,
        InitialSnapshot,
    ) {
        let (out_tx, out_rx) = channel::<NetOutbound>();
        let (in_tx, in_rx) = channel::<NetEvent>();
        let cursor = new_frame_cursor();
        let snapshot = Arc::new(std::sync::Mutex::new(None));
        let deferred_events = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        (
            Self {
                outgoing: out_tx,
                incoming: in_rx,
                deferred_events,
                frame_cursor: Arc::clone(&cursor),
                initial_snapshot: Arc::clone(&snapshot),
            },
            in_tx,
            out_rx,
            cursor,
            snapshot,
        )
    }

    /// Cache an initial-state snapshot the host will offer to every
    /// new peer that handshakes.
    pub fn set_initial_snapshot(&self, frame: u32, engine: &Engine) {
        if let Ok(mut slot) = self.initial_snapshot.lock() {
            *slot = Some((frame, engine.clone()));
        }
    }

    /// Cache an authoritative host snapshot and push it to peers
    /// that already handshook before the cache was populated.
    pub fn publish_initial_snapshot(&self, frame: u32, engine: &Engine) {
        self.set_initial_snapshot(frame, engine);
        let engine_bytes = engine.encode_native_snapshot();
        let _ = self.outgoing.send(NetOutbound::InitialSnapshot {
            frame,
            engine_bytes,
        });
    }

    /// Announce that this process has loaded the mission, adopted any
    /// required initial snapshot, and is ready for the host-controlled
    /// sim start barrier.
    pub fn send_ready_to_sim(&self, frame: u32) {
        let _ = self.outgoing.send(NetOutbound::ReadyToSim { frame });
    }

    /// Poll a network event, including events deferred by nested UI
    /// loops that only consumed modal-specific messages.
    pub fn try_recv_event(&self) -> Result<NetEvent, std::sync::mpsc::TryRecvError> {
        if let Ok(mut deferred) = self.deferred_events.lock()
            && let Some(event) = deferred.pop_front()
        {
            return Ok(event);
        }
        self.incoming.try_recv()
    }

    /// Poll only the transport receiver.  Modal loops use this to
    /// avoid repeatedly re-reading their own deferred events.
    pub fn try_recv_transport_event(&self) -> Result<NetEvent, std::sync::mpsc::TryRecvError> {
        self.incoming.try_recv()
    }

    /// Push events back in front of the main game-loop drain.
    pub fn defer_events(&self, events: Vec<NetEvent>) {
        if events.is_empty() {
            return;
        }
        if let Ok(mut deferred) = self.deferred_events.lock() {
            for event in events.into_iter().rev() {
                deferred.push_front(event);
            }
        }
    }

    /// Update the frame cursor.  Call once per tick from the game
    /// loop with the engine's `sim_frame`.
    pub fn publish_frame(&self, frame: u32) {
        self.frame_cursor.store(frame, Ordering::Relaxed);
    }

    /// Push a locally-produced [`PlayerCommand`] onto the wire.
    pub fn send_input(&self, cmd: PlayerCommand) {
        let origin_frame = self.frame_cursor.load(Ordering::Relaxed);
        let _ = self.outgoing.send(NetOutbound::Input {
            origin_frame,
            command: cmd,
        });
    }

    /// Push an authoritative state hash for `frame`.  Server-side only.
    pub fn send_state_hash(
        &self,
        frame: u32,
        hash: u64,
        clock_frame: u32,
        ms_until_next_frame: u32,
    ) {
        let _ = self.outgoing.send(NetOutbound::StateHash {
            frame,
            hash: Some(hash),
            clock_frame: Some(clock_frame),
            ms_until_next_frame: Some(ms_until_next_frame),
        });
    }

    /// Broadcast an immediate modal dismissal.  This bypasses the
    /// normal frame-delayed input stream because modal UI blocks the
    /// frame loop that would otherwise drain those inputs.
    pub fn send_modal_dismiss(&self, kind: ModalKind, result: DialogResult) {
        let _ = self
            .outgoing
            .send(NetOutbound::ModalDismiss { kind, result });
    }
}

/// Encode a [`NetMsg`] as a binary WebSocket payload.
pub fn encode_msg(msg: &NetMsg) -> Vec<u8> {
    bitcode::encode(msg)
}

/// Decode a binary WebSocket payload into a [`NetMsg`].
pub fn decode_msg(bytes: &[u8]) -> Result<NetMsg, bitcode::Error> {
    bitcode::decode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_includes_achievements_and_item_rebalance_state() {
        // Version 26 combines achievement tracker/rule state with item rules,
        // cached ale eligibility, and ground-stone command/projectile state.
        assert_eq!(NET_PROTOCOL_VERSION, 26);
    }

    #[test]
    fn netmsg_roundtrips() {
        let msg = NetMsg::BroadcastInput {
            server_frame: 40,
            origin_frame: 41,
            target_frame: 42,
            input: PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown),
        };
        let bytes = encode_msg(&msg);
        let back = decode_msg(&bytes).expect("decode");
        match back {
            NetMsg::BroadcastInput {
                server_frame,
                origin_frame,
                target_frame,
                input,
            } => {
                assert_eq!(server_frame, 40);
                assert_eq!(origin_frame, 41);
                assert_eq!(target_frame, 42);
                assert_eq!(input.player_id, PlayerId(2));
                assert!(matches!(input.command, PlayerCommand::CrouchDown));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn hello_welcome_roundtrips() {
        let hello = NetMsg::Hello {
            protocol_version: NET_PROTOCOL_VERSION,
            nickname: "alice".into(),
        };
        let welcome = NetMsg::Welcome {
            your_seat: PlayerId(2),
            mission_id: "Dem_Lei_MP".into(),
            mission_seed: 42,
            sim_config: crate::engine::SimConfig::default(),
            host_nickname: "host".into(),
        };
        let h = decode_msg(&encode_msg(&hello)).unwrap();
        let w = decode_msg(&encode_msg(&welcome)).unwrap();
        match (h, w) {
            (
                NetMsg::Hello {
                    protocol_version,
                    nickname,
                },
                NetMsg::Welcome {
                    your_seat,
                    mission_id,
                    mission_seed,
                    sim_config,
                    host_nickname,
                },
            ) => {
                assert_eq!(protocol_version, NET_PROTOCOL_VERSION);
                assert_eq!(nickname, "alice");
                assert_eq!(your_seat, PlayerId(2));
                assert_eq!(mission_id, "Dem_Lei_MP");
                assert_eq!(mission_seed, 42);
                assert_eq!(sim_config, crate::engine::SimConfig::default());
                assert_eq!(host_nickname, "host");
            }
            _ => panic!("wrong variants"),
        }
    }

    #[test]
    fn quit_updates_roundtrip_the_resolved_difficulty() {
        let msg = NetMsg::Input {
            origin_frame: 73,
            command: PlayerCommand::ApplyQuitMissionUpdates {
                exit_code: crate::game_operation::GameCode::LevelSucceeded,
                difficulty: crate::player_profile::DifficultyLevel::Hard,
                completed_at_unix_seconds: None,
                campaign_run_nonce: Some(1),
            },
        };

        let decoded = decode_msg(&encode_msg(&msg)).expect("decode quit-update command");
        assert!(matches!(
            decoded,
            NetMsg::Input {
                origin_frame: 73,
                command: PlayerCommand::ApplyQuitMissionUpdates {
                    exit_code: crate::game_operation::GameCode::LevelSucceeded,
                    difficulty: crate::player_profile::DifficultyLevel::Hard,
                    completed_at_unix_seconds: None,
                    campaign_run_nonce: Some(1),
                },
            }
        ));
    }

    #[test]
    fn resolved_drop_ale_route_roundtrips_over_bitcode_wire() {
        let route = crate::gate::RecordedGatePath {
            source_sector: crate::sector::SectorNumber::new(133),
            source_sector_index: crate::fast_find_grid::SectorIndex::new(57),
            source_layer: 11,
            outcome: crate::gate::RecordedGateOutcome::Success(vec![crate::gate::GatePathStep {
                door_index: crate::gate::DoorIndex(7),
                direct: false,
            }]),
        };
        let msg = NetMsg::Input {
            origin_frame: 35_283,
            command: PlayerCommand::DropAleAt {
                actor: crate::element::EntityId::Pc(crate::entity_id::PcId(36)),
                target_pos: crate::coordinates::MapPoint::new(778.0, 1714.0),
                running: false,
                already_authorized: true,
                goal_override: Some((crate::sector::SectorNumber::new(0), 0)),
                goal_sector_index_override: crate::fast_find_grid::SectorIndex::new(0),
                recorded_gate_path: Some(route.clone()),
            },
        };

        let decoded = decode_msg(&encode_msg(&msg)).expect("decode resolved DropAle command");
        let NetMsg::Input {
            origin_frame: 35_283,
            command:
                PlayerCommand::DropAleAt {
                    actor,
                    target_pos,
                    running: false,
                    already_authorized: true,
                    goal_override: Some((goal_sector, 0)),
                    goal_sector_index_override,
                    recorded_gate_path: Some(decoded_route),
                },
        } = decoded
        else {
            panic!("resolved DropAle command must survive bitcode wire round-trip");
        };
        assert_eq!(
            actor,
            crate::element::EntityId::Pc(crate::entity_id::PcId(36))
        );
        assert_eq!(target_pos.x.to_bits(), 778.0_f32.to_bits());
        assert_eq!(target_pos.y.to_bits(), 1714.0_f32.to_bits());
        assert_eq!(goal_sector, crate::sector::SectorNumber::new(0));
        assert_eq!(
            goal_sector_index_override,
            crate::fast_find_grid::SectorIndex::new(0)
        );
        assert_eq!(decoded_route, route);
    }
}
