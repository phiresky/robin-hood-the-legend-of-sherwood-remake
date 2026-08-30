//! Multiplayer session helpers extracted from `game_session`:
//! transport setup, per-frame net input drain, and rollback on
//! late inputs.

use super::runtime::TimelineFrame;
use crate::host::Host;
use crate::rewind::RewindBuffer;
use crate::sim_timeline::{RestorePolicy, replay_authoritative_frame_profiled};
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::engine_manager as engine_manager_api;
use robin_engine::player_command::PlayerInput;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum MultiplayerAdmissionEvent {
    Disconnected,
    InitialSnapshotAdopted { frame: u32 },
    BeginSim { frame: u32, start_epoch_ms: u64 },
}

fn canonicalize_player_input_order(inputs: &mut Vec<PlayerInput>) {
    if inputs.len() <= 1 {
        return;
    }

    let mut indexed: Vec<(usize, PlayerInput)> = inputs.drain(..).enumerate().collect();
    indexed.sort_by(|(a_idx, a), (b_idx, b)| {
        a.player_id
            .0
            .cmp(&b.player_id.0)
            .then_with(|| a_idx.cmp(b_idx))
    });
    inputs.extend(indexed.into_iter().map(|(_, input)| input));
}

pub(crate) struct NetDrainResult {
    /// Inputs scheduled for the current frame. The caller applies these
    /// and records them in the per-frame command log.
    pub inputs: Vec<PlayerInput>,
    /// True when multiplayer adopted or rewound simulation state. Any
    /// short-horizon diagnostic history captured before that point
    /// belongs to the previous timeline and must be discarded.
    pub rewrote_sim_state: bool,
    /// Admission events in exact wire-drain order. Snapshot events are only
    /// emitted after decode and adoption have succeeded.
    pub(super) admission_events: Vec<MultiplayerAdmissionEvent>,
    /// Network-owned pause sources after admission and host-clock scheduling.
    pub(super) pause_simulation: bool,
    /// Latest host clock phase sample observed this drain:
    /// `(host_frame, ms_until_next_frame)`.
    pub latest_host_clock_sample: Option<(u32, u32)>,
    /// Rollback diagnostic from this drain, if a late input rewrote
    /// the local timeline.
    pub rollback: Option<MultiplayerRollbackTelemetry>,
    /// Authoritative wire cursor adopted during this drain. The timeline
    /// owner applies this after all events have been processed.
    pub(super) adopted_frame: Option<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct MultiplayerRollbackTelemetry {
    pub(super) path: &'static str,
    pub(super) earliest_frame: u32,
    pub(super) target_frame: u32,
    pub(super) late_input_count: usize,
    pub(super) replayed_frames: u32,
    pub(super) total_us: u128,
    pub(super) restore_us: u128,
    pub(super) replay_us: u128,
    pub(super) replay_remember_us: u128,
    pub(super) replay_command_lookup_us: u128,
    pub(super) replay_apply_us: u128,
    pub(super) replay_tick_us: u128,
}

/// Drain pending wire events from the multiplayer transport into current
/// frame inputs and apply any required network state corrections.
///
/// Also folds `AssignedLocalSeat` events (late seat-assignment
/// races) into `host.transport.local_seat` and logs other diagnostic events.
/// Native and browser disconnects remain synchronized only while their real
/// transport reconnect loops are active. Both abandon the old prediction
/// future and wait for an authoritative replacement snapshot.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_net_inputs(
    host: &mut Host,
    manager: &mut engine_manager_api::EngineManager,
    current_frame: u32,
    pending_inputs: &mut std::collections::BTreeMap<TimelineFrame, Vec<PlayerInput>>,
    assets: &LevelAssets,
    rewind_buffer: &mut RewindBuffer,
    peer_hashes: &mut std::collections::BTreeMap<u32, u64>,
) -> NetDrainResult {
    use crate::multiplayer::NetEvent;

    let Some(net) = host.transport.net.as_ref() else {
        // Not in a session — drain anything sitting in pending and
        // return.  Pending should be empty in single-player but is
        // safe to flush.
        return NetDrainResult {
            inputs: pending_inputs
                .remove(&TimelineFrame::from_wire(current_frame))
                .unwrap_or_default(),
            rewrote_sim_state: false,
            admission_events: Vec::new(),
            pause_simulation: false,
            latest_host_clock_sample: None,
            rollback: None,
            adopted_frame: None,
        };
    };

    // 1. Drain transport into "future" and "late" buckets.
    let mut late_inputs: Vec<(u32, PlayerInput)> = Vec::new();
    let mut rewrote_sim_state = false;
    let mut admission_events = Vec::new();
    let mut latest_host_clock_sample: Option<(u32, u32)> = None;
    let mut rollback_telemetry = None;
    let mut effective_frame = current_frame;
    while let Ok(event) = net.try_recv_event() {
        match event {
            NetEvent::Input {
                server_frame,
                origin_frame,
                target_frame,
                input,
            } => {
                if target_frame >= effective_frame {
                    pending_inputs
                        .entry(TimelineFrame::from_wire(target_frame))
                        .or_default()
                        .push(input);
                } else {
                    tracing::info!(
                        local_frame = effective_frame,
                        server_frame,
                        origin_frame,
                        target_frame,
                        late_by = effective_frame.saturating_sub(target_frame),
                        local_minus_server = effective_frame as i64 - server_frame as i64,
                        local_minus_origin = effective_frame as i64 - origin_frame as i64,
                        "multiplayer late input received"
                    );
                    late_inputs.push((target_frame, input));
                }
            }
            NetEvent::AssignedLocalSeat(seat) => {
                tracing::info!(?seat, "multiplayer: local seat assigned (late)");
                host.transport.local_seat = seat;
            }
            NetEvent::Note(s) => tracing::info!(note = %s, "multiplayer: note"),
            NetEvent::Disconnected => {
                tracing::warn!(
                    "multiplayer: peer disconnected — transport will auto-reconnect; \
                     simulation is held until an authoritative snapshot arrives"
                );
                host.transport.reconnecting = true;
                admission_events.push(MultiplayerAdmissionEvent::Disconnected);
                // Everything derived from the disconnected process's future
                // is invalid. Events already drained from that generation
                // occur before Disconnected and are removed here; events from
                // the replacement stream arrive afterward.
                late_inputs.clear();
                pending_inputs.clear();
                peer_hashes.clear();
                *rewind_buffer = RewindBuffer::new();
                latest_host_clock_sample = None;
                rewrote_sim_state = true;
            }
            NetEvent::Reconnected => {
                tracing::info!("multiplayer: transport reconnected; awaiting host snapshot");
            }
            NetEvent::MissionConfig {
                mission_id,
                rng_seed,
                sim_config,
                speech_timing_locale,
            } => {
                // Welcome is awaited before Engine construction; retain the
                // event copy for diagnostics and reconnect validation.
                if host.transport.mission_id.as_deref() != Some(mission_id.as_str())
                    || host.transport.mission_seed != Some(rng_seed)
                    || host.transport.mission_sim_config != Some(sim_config)
                    || host.transport.speech_timing_locale != speech_timing_locale
                {
                    panic!(
                        "fatal multiplayer session error: Welcome/reconnect mission construction state changed"
                    );
                }
                host.transport.mission_seed = Some(rng_seed);
                host.transport.mission_sim_config = Some(sim_config);
                host.transport.speech_timing_locale = speech_timing_locale;
                host.transport.mission_id = Some(mission_id);
            }
            NetEvent::Fatal(message) => panic!("fatal multiplayer session error: {message}"),
            NetEvent::InitialSnapshot {
                frame,
                engine_bytes,
            } => {
                let replacing_prediction_future = host.transport.reconnecting;
                if frame < effective_frame && !replacing_prediction_future {
                    tracing::debug!(
                        frame,
                        local_timeline_frame = effective_frame,
                        "multiplayer: ignoring stale host engine snapshot"
                    );
                    continue;
                }
                // Frame-0 fast path: if local init already matches the
                // host, avoid replacing the just-loaded engine. If it
                // differs, adopt the host snapshot before simulation
                // begins; decoded snapshots now reattach LevelAssets
                // cleanly, so this is the same path as mid-mission
                // rejoin without advancing the frame cursor.
                if frame == 0 && effective_frame == 0 {
                    let local_hash = robin_engine::replay::state_hash(&manager.engine);
                    match Engine::decode_native_snapshot(&engine_bytes) {
                        Ok(snapshot) => {
                            let snap_hash = robin_engine::replay::state_hash(&snapshot);
                            if local_hash == snap_hash {
                                admission_events.push(
                                    MultiplayerAdmissionEvent::InitialSnapshotAdopted { frame },
                                );
                                tracing::info!(
                                    hash = format!("{local_hash:016x}"),
                                    "multiplayer: skipping frame-0 snapshot adopt; \
                                     local engine already matches host"
                                );
                                if let Some(net) = host.transport.net.as_ref() {
                                    net.send_ready_to_sim(frame);
                                }
                                if replacing_prediction_future {
                                    *rewind_buffer = RewindBuffer::new();
                                    rewind_buffer.seed_initial_anchor(frame, &manager.engine);
                                    pending_inputs.clear();
                                    peer_hashes.clear();
                                    rewrote_sim_state = true;
                                }
                            } else {
                                match Engine::adopt_authoritative_snapshot(snapshot, assets) {
                                    Ok(adopted) => {
                                        manager.engine = adopted;
                                        admission_events.push(
                                            MultiplayerAdmissionEvent::InitialSnapshotAdopted {
                                                frame,
                                            },
                                        );
                                        let adopted_hash =
                                            robin_engine::replay::state_hash(&manager.engine);
                                        tracing::info!(
                                            local = format!("{local_hash:016x}"),
                                            snap = format!("{snap_hash:016x}"),
                                            adopted = format!("{adopted_hash:016x}"),
                                            "multiplayer: adopted frame-0 host snapshot after \
                                             local init diverged"
                                        );
                                        *rewind_buffer = RewindBuffer::new();
                                        rewind_buffer.seed_initial_anchor(frame, &manager.engine);
                                        if replacing_prediction_future {
                                            pending_inputs.clear();
                                            peer_hashes.clear();
                                        } else {
                                            let adopted = TimelineFrame::from_wire(frame);
                                            pending_inputs.retain(|&queued, _| queued >= adopted);
                                            peer_hashes.retain(|&f, _| f >= frame);
                                        }
                                        rewrote_sim_state = true;
                                        if let Some(net) = host.transport.net.as_ref() {
                                            net.send_ready_to_sim(frame);
                                        }
                                    }
                                    Err(error) => panic!(
                                        "multiplayer: rejected incompatible frame-0 host snapshot: {error}"
                                    ),
                                }
                            }
                        }
                        Err(e) => {
                            panic!("multiplayer: failed to deserialize frame-0 host snapshot: {e}")
                        }
                    }
                    continue;
                }

                // Mid-mission rejoin (frame > 0): atomically adopt the host's
                // snapshot after attaching immutable script/grid/sprite data
                // once from the locally loaded LevelAssets.
                match Engine::decode_native_snapshot(&engine_bytes) {
                    Ok(snapshot) => match Engine::adopt_authoritative_snapshot(snapshot, assets) {
                        Ok(adopted_engine) => {
                            manager.engine = adopted_engine;
                            admission_events
                                .push(MultiplayerAdmissionEvent::InitialSnapshotAdopted { frame });
                            let adopted_hash = robin_engine::replay::state_hash(&manager.engine);
                            tracing::info!(
                                frame,
                                local_timeline_frame = effective_frame,
                                bytes = engine_bytes.len(),
                                adopted_hash = format!("{adopted_hash:016x}"),
                                "multiplayer: adopting host's engine snapshot"
                            );
                            effective_frame = frame;
                            if let Some(net) = host.transport.net.as_ref() {
                                net.send_ready_to_sim(frame);
                            }
                            *rewind_buffer = RewindBuffer::new();
                            rewind_buffer.seed_initial_anchor(frame, &manager.engine);
                            if replacing_prediction_future {
                                pending_inputs.clear();
                                peer_hashes.clear();
                            } else {
                                let adopted = TimelineFrame::from_wire(frame);
                                pending_inputs.retain(|&queued, _| queued >= adopted);
                                peer_hashes.retain(|&f, _| f >= frame);
                            }
                            rewrote_sim_state = true;
                        }
                        Err(error) => panic!(
                            "multiplayer: rejected incompatible host snapshot at frame {frame}: {error}"
                        ),
                    },
                    Err(e) => panic!(
                        "multiplayer: failed to deserialize host snapshot at frame {frame}: {e}"
                    ),
                }
            }
            NetEvent::PeerStateHash {
                frame,
                hash,
                clock_frame,
                ms_until_next_frame,
            } => {
                if let Some(hash) = hash {
                    peer_hashes.insert(frame, hash);
                }
                if let (Some(clock_frame), Some(ms_until_next_frame)) =
                    (clock_frame, ms_until_next_frame)
                {
                    latest_host_clock_sample = Some((clock_frame, ms_until_next_frame));
                }
            }
            NetEvent::BeginSim {
                frame,
                start_epoch_ms,
            } => {
                host.transport.reconnecting = false;
                tracing::info!(
                    frame,
                    start_epoch_ms,
                    "multiplayer: begin-sim barrier released"
                );
                if effective_frame != frame {
                    effective_frame = frame;
                    let adopted = TimelineFrame::from_wire(frame);
                    pending_inputs.retain(|&queued, _| queued >= adopted);
                    rewind_buffer.clear_recent_checkpoints();
                    peer_hashes.retain(|&f, _| f >= frame);
                    rewrote_sim_state = true;
                }
                admission_events.push(MultiplayerAdmissionEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                });
            }
            NetEvent::PrepareSnapshotTransition { id, payload } => {
                assert_ne!(
                    host.transport.local_seat,
                    robin_engine::player_command::PlayerId::HOST,
                    "authoritative host received its own snapshot transition prepare"
                );
                assert_eq!(
                    id.session_id,
                    net.session_id().unwrap_or_else(|error| {
                        panic!("snapshot transition is missing session identity: {error}")
                    }),
                    "snapshot transition prepare belongs to another session"
                );
                assert!(
                    host.transport.snapshot_transition.is_none(),
                    "received a second snapshot transition while one is pending"
                );
                let payload = match payload {
                    robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                        mission_id,
                        save_bytes,
                    } => {
                        let save: crate::save_file::GameSaveFile =
                            serde_json::from_slice(&save_bytes).unwrap_or_else(|error| {
                                panic!(
                                    "multiplayer snapshot transition payload is invalid: {error}"
                                )
                            });
                        save.header.validate().unwrap_or_else(|error| {
                            panic!("multiplayer snapshot transition header is invalid: {error:#}")
                        });
                        assert_eq!(
                            save.header.mission_id, mission_id,
                            "snapshot transition wire mission differs from its exact payload"
                        );
                        crate::main_entry::validate_save_mission(&save, &assets.profile_manager)
                            .unwrap_or_else(|error| {
                                panic!(
                                    "multiplayer snapshot transition mission is invalid: {error}"
                                )
                            });
                        let reencoded = serde_json::to_vec(&save).unwrap_or_else(|error| {
                            panic!(
                                "multiplayer snapshot transition could not be re-encoded: {error}"
                            )
                        });
                        assert_eq!(
                            reencoded, save_bytes,
                            "snapshot transition bytes changed during validation"
                        );
                        crate::host::PendingSnapshotTransitionPayload::Save {
                            slot: None,
                            save: Box::new(save),
                        }
                    }
                    robin_engine::multiplayer::SnapshotTransitionPayload::CampaignExit {
                        exit_code,
                        engine_bytes,
                    } => {
                        assert_eq!(
                            exit_code,
                            robin_engine::game_operation::GameCode::LevelInterrupted,
                            "campaign transition may only launch the selected mission"
                        );
                        let decoded =
                            Engine::decode_native_snapshot(&engine_bytes).unwrap_or_else(|error| {
                                panic!("multiplayer campaign snapshot is invalid: {error}")
                            });
                        let adopted = Engine::adopt_authoritative_snapshot(decoded, assets)
                            .unwrap_or_else(|error| {
                                panic!("multiplayer campaign snapshot cannot be adopted: {error}")
                            });
                        assert_eq!(
                            adopted.encode_native_snapshot(),
                            engine_bytes,
                            "campaign transition bytes changed during validation"
                        );
                        crate::host::PendingSnapshotTransitionPayload::CampaignExit {
                            exit_code,
                            engine: Some(Box::new(adopted)),
                        }
                    }
                };
                host.transport.snapshot_transition = Some(crate::host::PendingSnapshotTransition {
                    id,
                    payload,
                    committed: false,
                });
                host.transport.reconnecting = true;
                net.acknowledge_snapshot_transition(id)
                    .unwrap_or_else(|error| {
                        panic!("failed to acknowledge multiplayer snapshot transition: {error}")
                    });
            }
            NetEvent::CommitSnapshotTransition { id } => {
                let transition = host
                    .transport
                    .snapshot_transition
                    .as_mut()
                    .unwrap_or_else(|| {
                        panic!("snapshot transition commit has no prepared payload")
                    });
                assert_eq!(
                    transition.id, id,
                    "snapshot transition commit does not match prepared payload"
                );
                transition.committed = true;
                host.transport.reconnecting = true;
            }
            event @ (NetEvent::ModalProposal { .. } | NetEvent::ModalDecision { .. }) => {
                net.defer_modal_event(event).unwrap_or_else(|error| {
                    panic!("fatal multiplayer modal routing error: {error}")
                });
            }
        }
    }

    // 2. Late-input rollback.  Splice every late input into the
    //    rewind buffer's command log at its target frame, then
    //    reconstruct the engine state at `sim_frame` once.  Multiple
    //    splices share one rewind because `rewind_to` replays from
    //    snapshot through the entire log.
    if !late_inputs.is_empty() {
        let mut indexed: Vec<(usize, (u32, PlayerInput))> =
            late_inputs.drain(..).enumerate().collect();
        indexed.sort_by(|(a_idx, (a_frame, a_input)), (b_idx, (b_frame, b_input))| {
            a_frame
                .cmp(b_frame)
                .then_with(|| a_input.player_id.0.cmp(&b_input.player_id.0))
                .then_with(|| a_idx.cmp(b_idx))
        });
        late_inputs.extend(indexed.into_iter().map(|(_, input)| input));

        let mut needs_rewind = false;
        let local_is_peer =
            host.transport.local_seat != robin_engine::player_command::PlayerId::HOST;
        let mut local_reconnect_reason = None;
        let mut host_reconnect_reason = None;
        let mut earliest = u32::MAX;
        let mut late_input_count = 0usize;
        for (frame, input) in late_inputs {
            if rewind_buffer.splice_late_input(frame, input.clone()) {
                needs_rewind = true;
                earliest = earliest.min(frame);
                late_input_count += 1;
            } else {
                let reason = format!(
                    "input for frame {frame} arrived after rollback horizon {} at local frame {effective_frame}",
                    rewind_buffer.oldest_cmd_frame()
                );
                tracing::error!(
                    target_frame = frame,
                    oldest = rewind_buffer.oldest_cmd_frame(),
                    effective_frame,
                    player_id = input.player_id.0,
                    "multiplayer: late input below rewind horizon — requiring a full snapshot reconnect"
                );
                if local_is_peer {
                    local_reconnect_reason.get_or_insert(reason);
                } else {
                    assert_ne!(
                        input.player_id,
                        robin_engine::player_command::PlayerId::HOST,
                        "host-authored input fell below the authoritative host rollback horizon"
                    );
                    // The server already broadcast this input before the game
                    // loop discovered that its target predates the host's
                    // horizon. Defer the reconnect request until any other
                    // viable late inputs in this batch have been reconstructed
                    // and the replacement snapshot cache is current.
                    host_reconnect_reason.get_or_insert(reason);
                }
            }
        }
        if let Some(reason) = local_reconnect_reason {
            net.reconnect_for_snapshot(host.transport.local_seat, reason.clone())
                .unwrap_or_else(|error| {
                    panic!("failed to request multiplayer snapshot reconnect: {error}")
                });
            host.transport.reconnecting = true;
            pending_inputs.clear();
            admission_events.push(MultiplayerAdmissionEvent::Disconnected);
            tracing::warn!(
                %reason,
                "multiplayer: client suspended until complete disconnect/reconnect and host snapshot adoption"
            );
            // Any successful splices above belong to the abandoned local
            // future. The reconnect snapshot replaces both engine state and
            // reconstruction history before simulation is released again.
            needs_rewind = false;
        }
        if needs_rewind {
            let rollback_start = web_time::Instant::now();
            if let Some((new_engine, mut telemetry)) = rewind_from_recent_timeline_history(
                effective_frame,
                assets,
                rewind_buffer,
                earliest,
                late_input_count,
            ) {
                telemetry.total_us = rollback_start.elapsed().as_micros();
                tracing::info!(
                    path = telemetry.path,
                    earliest_frame = telemetry.earliest_frame,
                    target_frame = telemetry.target_frame,
                    replayed_frames = telemetry.replayed_frames,
                    late_inputs = telemetry.late_input_count,
                    total_us = telemetry.total_us,
                    restore_us = telemetry.restore_us,
                    replay_us = telemetry.replay_us,
                    replay_remember_us = telemetry.replay_remember_us,
                    replay_command_lookup_us = telemetry.replay_command_lookup_us,
                    replay_apply_us = telemetry.replay_apply_us,
                    replay_tick_us = telemetry.replay_tick_us,
                    "multiplayer rollback timing"
                );
                manager.engine = new_engine;
                rollback_telemetry = Some(telemetry);
                rewrote_sim_state = true;
            } else if let Some(new_engine) = rewind_buffer.rewind_to(assets, effective_frame) {
                let telemetry = MultiplayerRollbackTelemetry {
                    path: "rewind-buffer",
                    earliest_frame: earliest,
                    target_frame: effective_frame,
                    late_input_count,
                    replayed_frames: effective_frame.saturating_sub(earliest),
                    total_us: rollback_start.elapsed().as_micros(),
                    restore_us: 0,
                    replay_us: 0,
                    replay_remember_us: 0,
                    replay_command_lookup_us: 0,
                    replay_apply_us: 0,
                    replay_tick_us: 0,
                };
                tracing::info!(
                    path = telemetry.path,
                    earliest_frame = telemetry.earliest_frame,
                    target_frame = telemetry.target_frame,
                    replayed_frames = telemetry.replayed_frames,
                    late_inputs = telemetry.late_input_count,
                    total_us = telemetry.total_us,
                    "multiplayer rollback timing"
                );
                manager.engine = new_engine;
                rewind_buffer.truncate_recent_after(earliest);
                rollback_telemetry = Some(telemetry);
                rewrote_sim_state = true;
            } else {
                panic!(
                    "multiplayer rollback failed: canonical journal accepted {late_input_count} late input(s) from frame {earliest}, but no retained snapshot can reconstruct authoritative frame {effective_frame}"
                );
            }
        }
        if let Some(reason) = host_reconnect_reason {
            net.set_initial_snapshot(effective_frame, &manager.engine);
            net.reconnect_all_for_snapshot(reason)
                .unwrap_or_else(|error| {
                    panic!("failed to require multiplayer snapshot reconnect: {error}")
                });
        }
    }

    // 3. Return inputs scheduled for this frame.  The caller applies
    //    them to the live engine and folds them into `frame_cmds` so
    //    the recorder + rewind buffer capture them.
    let mut due_inputs = pending_inputs
        .remove(&TimelineFrame::from_wire(effective_frame))
        .unwrap_or_default();
    canonicalize_player_input_order(&mut due_inputs);

    NetDrainResult {
        inputs: due_inputs,
        rewrote_sim_state,
        admission_events,
        pause_simulation: false,
        latest_host_clock_sample,
        rollback: rollback_telemetry,
        adopted_frame: (effective_frame != current_frame).then_some(effective_frame),
    }
}

/// Drain one deterministic multiplayer ingress boundary and fold its
/// process-side effects into the timeline owner. The caller remains
/// responsible for applying returned inputs and recording them in its own
/// frame, so graphical and true-headless drivers keep their distinct input
/// contracts without duplicating admission state.
#[allow(clippy::too_many_arguments)]
pub(super) fn drain_mission_network(
    timeline: &mut super::runtime::TimelineRuntime,
    host: &mut Host,
    manager: &mut engine_manager_api::EngineManager,
    assets: &LevelAssets,
    checkpoint_always: bool,
    now_epoch_ms: u64,
) -> NetDrainResult {
    let current_frame = timeline.frame_number();
    if let Some(net) = host.transport.net.as_ref() {
        net.publish_frame(current_frame);
    }
    let mut drain = drain_net_inputs(
        host,
        manager,
        current_frame,
        &mut timeline.pending_inputs,
        assets,
        &mut timeline.rewind_buffer,
        &mut timeline.peer_hashes,
    );
    if drain.rewrote_sim_state
        && let Some(checker) = timeline.rollback_checker.as_mut()
    {
        checker.reset();
    }
    if let Some(rollback) = drain.rollback.clone() {
        timeline.last_mp_rollback = Some(rollback);
    }
    timeline.apply_multiplayer_admission_events(&drain.admission_events);
    if let Some(frame) = drain.adopted_frame {
        timeline.adopt_frame(super::runtime::TimelineFrame::from_wire(frame));
    }

    let local_is_peer = host.transport.net.is_some()
        && host.transport.local_seat != robin_engine::player_command::PlayerId::HOST;
    if local_is_peer
        && let Some((clock_frame, ms_until_next_frame)) = drain.latest_host_clock_sample
    {
        let current_frame = timeline.frame_number();
        accept_host_frame_schedule(
            &mut timeline.mp_host_frame_schedule,
            clock_frame,
            ms_until_next_frame,
            current_frame,
        );
    }

    let admission_pause = timeline.multiplayer_admission_paused(now_epoch_ms);
    let mut clock_pause = false;
    if local_is_peer && !admission_pause {
        if let Some(deadline_ms) = host_scheduled_frame_deadline_ms(
            timeline.mp_host_frame_schedule,
            timeline.frame_number(),
        ) {
            let now_ms = crate::window::process_uptime_ms();
            let until_frame_ms = deadline_ms - i64::from(now_ms);
            if until_frame_ms > 0 {
                clock_pause = true;
                if now_ms.saturating_sub(timeline.last_mp_clock_ahead_log_ms) >= 1000 {
                    timeline.last_mp_clock_ahead_log_ms = now_ms;
                    tracing::info!(
                        scheduled_frame = timeline.mp_host_frame_schedule.map(|(frame, _)| frame),
                        local_frame = timeline.frame_number(),
                        until_frame_ms,
                        "multiplayer: local frame is ahead of host schedule; holding sim"
                    );
                }
            }
        } else {
            clock_pause = true;
        }
    }
    drain.pause_simulation = admission_pause || host.transport.reconnecting || clock_pause;

    if host.transport.net.is_some() && (checkpoint_always || drain.rewrote_sim_state) {
        timeline
            .rewind_buffer
            .checkpoint_recent(timeline.frame_number(), &manager.engine);
    }
    drain
}

fn rewind_from_recent_timeline_history(
    target_frame: u32,
    assets: &LevelAssets,
    rewind_buffer: &mut RewindBuffer,
    start_frame: u32,
    late_input_count: usize,
) -> Option<(Engine, MultiplayerRollbackTelemetry)> {
    let restore_start = web_time::Instant::now();
    let mut snapshot = rewind_buffer.restore_recent(start_frame, RestorePolicy::Exact)?;
    let restore_us = restore_start.elapsed().as_micros();

    // Rebuild corrected checkpoints transactionally. A missing command (or
    // any future fallible replay input) must leave the last known-good recent
    // history available to a fallback path rather than publishing a partial
    // reconstruction.
    let mut corrected_history = rewind_buffer.recent_checkpoints().clone();
    corrected_history.truncate_after(start_frame);
    let mut replay_remember_us = 0;
    let mut replay_command_lookup_us = 0;
    let mut replay_apply_us = 0;
    let mut replay_tick_us = 0;
    let replay_start = web_time::Instant::now();
    while snapshot.frame < target_frame {
        let remember_start = web_time::Instant::now();
        corrected_history.remember(snapshot.clone());
        replay_remember_us += remember_start.elapsed().as_micros();
        let command_lookup_start = web_time::Instant::now();
        let frame = rewind_buffer.frame_for(snapshot.frame)?;
        replay_command_lookup_us += command_lookup_start.elapsed().as_micros();
        let replayed_frame = replay_authoritative_frame_profiled(&mut snapshot, assets, frame);
        replay_apply_us += replayed_frame.timing.apply_us;
        replay_tick_us += replayed_frame.timing.tick_us;
        let _discarded_frame_output = replayed_frame.output;
    }
    let remember_start = web_time::Instant::now();
    corrected_history.remember(snapshot.clone());
    replay_remember_us += remember_start.elapsed().as_micros();
    let replay_us = replay_start.elapsed().as_micros();
    rewind_buffer.replace_recent_checkpoints(corrected_history);

    Some((
        snapshot.engine,
        MultiplayerRollbackTelemetry {
            path: "recent-timeline-history",
            earliest_frame: start_frame,
            target_frame,
            late_input_count,
            replayed_frames: target_frame.saturating_sub(start_frame),
            total_us: 0,
            restore_us,
            replay_us,
            replay_remember_us,
            replay_command_lookup_us,
            replay_apply_us,
            replay_tick_us,
        },
    ))
}

pub(super) fn accept_host_frame_schedule(
    mp_host_frame_schedule: &mut Option<(u32, u32)>,
    clock_frame: u32,
    ms_until_next_frame: u32,
    local_frame: u32,
) {
    if mp_host_frame_schedule.is_some_and(|(sample_frame, _)| clock_frame < sample_frame) {
        tracing::trace!(
            clock_frame,
            current_sample_frame = mp_host_frame_schedule.map(|(frame, _)| frame),
            "multiplayer: ignored stale host frame schedule"
        );
        return;
    }

    let now_ms = crate::window::process_uptime_ms();
    let scheduled_deadline_ms = now_ms.saturating_add(ms_until_next_frame);
    *mp_host_frame_schedule = Some((clock_frame, scheduled_deadline_ms));
    let local_frame_deadline_ms =
        host_scheduled_frame_deadline_ms(*mp_host_frame_schedule, local_frame)
            .expect("host schedule was just installed");
    tracing::info!(
        host_clock_frame = clock_frame,
        ms_until_next_frame,
        local_frame_at_receive = local_frame,
        deadline_delta_ms_for_local_frame = local_frame_deadline_ms - i64::from(now_ms),
        "multiplayer: received host frame schedule"
    );
}

pub(super) fn host_scheduled_frame_deadline_ms(
    mp_host_frame_schedule: Option<(u32, u32)>,
    local_frame: u32,
) -> Option<i64> {
    let (scheduled_frame, scheduled_deadline_ms) = mp_host_frame_schedule?;
    let frame_delta = i64::from(local_frame) - i64::from(scheduled_frame);
    Some(i64::from(scheduled_deadline_ms) + frame_delta * i64::from(engine_api::FRAME_TIME_MS))
}

/// Initialise the multiplayer transport based on `--server` /
/// `--connect` / `--mp-nickname` CLI flags.  Populates
/// [`Host::net`] and [`Host::local_seat`] when active; no-op when
/// neither flag was given.
///
/// On `--server`: starts the listener thread with this process at
/// seat 0 ([`PlayerId::HOST`]).
/// On `--connect`: dials the server, blocks briefly waiting for
/// the assigned-seat handshake, then sets `host.transport.local_seat` so
/// outgoing inputs are stamped correctly.
///
/// Network failures abort multiplayer startup so the caller can return
/// to the main menu instead of silently launching a different local game.
pub(super) async fn setup_multiplayer_session(
    host: &mut Host,
    args: &crate::main_entry::CliArgs,
    authoritative_mission_id: &str,
    authoritative_rng_seed: u64,
    authoritative_sim_config: robin_engine::engine::SimConfig,
) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    use crate::multiplayer::NetEvent;
    #[cfg(not(target_arch = "wasm32"))]
    use crate::multiplayer::start_server;
    use crate::multiplayer::{NetChannels, connect_client};
    #[cfg(not(target_arch = "wasm32"))]
    use std::time::{Duration, Instant};

    validate_multiplayer_launch_args(args)?;

    let nickname = if args.mp_nickname.is_empty() {
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "player".to_string())
    } else {
        args.mp_nickname.clone()
    };

    if args.server {
        #[cfg(target_arch = "wasm32")]
        return Err(
            "multiplayer: browser builds cannot host; connect to a native host".to_string(),
        );

        #[cfg(not(target_arch = "wasm32"))]
        {
            let publish_browser_links = resolve_browser_join_publication(args)?;
            let speech_timing_locale = host
                .application_context
                .canonical_speech_timing_locale()
                .map_err(|error| {
                    format!("multiplayer: cannot select authoritative speech timing: {error}")
                })?;
            let (mut channels, in_tx, out_rx, frame_cursor, snapshot_slot) = NetChannels::new();
            match start_server(
                nickname.clone(),
                authoritative_mission_id.to_string(),
                authoritative_rng_seed,
                authoritative_sim_config,
                speech_timing_locale.clone(),
                in_tx,
                out_rx,
                frame_cursor,
                snapshot_slot,
                args.mp_expected_players.unwrap_or(1),
                publish_browser_links,
            ) {
                Ok(handle) => {
                    channels
                        .install_session_id(handle.session_id())
                        .map_err(|error| format!("multiplayer: {error}"))?;
                    if publish_browser_links {
                        let content_edition = if crate::main_entry::detect_demo_mode_with_context(
                            &args.global_options,
                        )
                        .is_some()
                        {
                            crate::multiplayer::join_ticket::BrowserContentEdition::Demo
                        } else {
                            crate::multiplayer::join_ticket::BrowserContentEdition::Full
                        };
                        let content_identity_sha256 =
                            crate::multiplayer::content_identity::active_content_identity()
                                .map_err(|error| {
                                    format!(
                                        "multiplayer: cannot publish an exact browser content invitation: {error}"
                                    )
                                })?;
                        let ticket = handle
                            .browser_join_ticket(
                                content_edition,
                                content_identity_sha256.clone(),
                                args.mp_mission_profile_id,
                                args.mp_expected_players.unwrap_or(1),
                            )
                            .map_err(|error| {
                                format!("multiplayer: browser invitation unavailable: {error}")
                            })?;
                        let browser_base =
                            std::env::var("ROBINHOOD_BROWSER_URL").unwrap_or_else(|_| {
                                crate::multiplayer::join_ticket::DEFAULT_BROWSER_URL.to_string()
                            });
                        let share_url = ticket.share_url(&browser_base).map_err(|error| {
                            format!("multiplayer: browser share URL unavailable: {error}")
                        })?;
                        tracing::info!(
                            browser_join_code = %ticket.encode(),
                            %share_url,
                            relay = %ticket.payload().relay_url,
                            ?content_edition,
                            %content_identity_sha256,
                            "browser multiplayer invitation (relay can observe participant IPs, connection times, and byte counts; game traffic remains end-to-end encrypted)"
                        );
                        host.pending_console_output.push(format!(
                            "Browser join code (expires after 30 minutes if unused): {}",
                            ticket.encode()
                        ));
                        host.pending_console_output
                            .push(format!("Browser join link: {share_url}"));
                        host.pending_console_output.push(format!(
                            "Privacy: relay {} can observe IPs, timing, and byte counts; gameplay is end-to-end encrypted.",
                            ticket.payload().relay_url
                        ));
                    }
                    tracing::info!(
                        endpoint_id = %handle.endpoint_id(),
                        nickname = %nickname,
                        seed = authoritative_rng_seed,
                        "multiplayer: hosting on iroh endpoint {}",
                        handle.endpoint_id()
                    );
                    host.transport.local_seat = handle.local_seat;
                    channels.attach_runtime(handle);
                    host.transport.net = Some(channels);
                    host.transport.mission_seed = Some(authoritative_rng_seed);
                    host.transport.mission_sim_config = Some(authoritative_sim_config);
                    host.transport.speech_timing_locale = speech_timing_locale;
                    host.transport.mission_id = Some(authoritative_mission_id.to_string());
                }
                Err(e) => {
                    return Err(format!("multiplayer: failed to start server: {e}"));
                }
            }
        }
    } else if let Some(addr) = args.connect.as_deref() {
        let (mut channels, in_tx, out_rx, _client_frame_cursor, _client_snapshot) =
            NetChannels::new();
        match connect_client(addr, nickname.clone(), in_tx, out_rx) {
            Ok(handle) => {
                #[cfg(target_arch = "wasm32")]
                {
                    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(10);
                    while (handle.mission_id().is_none()
                        || handle.mission_seed().is_none()
                        || handle.mission_sim_config().is_none()
                        || handle.assigned_seat().is_none()
                        || handle.session_id().is_none()
                        || handle.speech_timing_authority().is_none())
                        && web_time::Instant::now() < deadline
                    {
                        if let Some(error) = handle.startup_error() {
                            return Err(format!(
                                "multiplayer: browser relay startup failed: {error}"
                            ));
                        }
                        crate::window::sleep_ms(10).await;
                    }
                    if let Some(error) = handle.startup_error() {
                        return Err(format!(
                            "multiplayer: browser relay startup failed: {error}"
                        ));
                    }
                    if handle.mission_id().is_none()
                        || handle.mission_seed().is_none()
                        || handle.mission_sim_config().is_none()
                        || handle.assigned_seat().is_none()
                        || handle.session_id().is_none()
                        || handle.speech_timing_authority().is_none()
                    {
                        return Err(
                            "multiplayer: timed out awaiting authoritative Welcome before Engine construction"
                                .to_string(),
                        );
                    }
                }
                let session_id = handle.session_id().ok_or_else(|| {
                    "multiplayer: Welcome omitted the required session identity".to_string()
                })?;
                channels
                    .install_session_id(session_id)
                    .map_err(|error| format!("multiplayer: {error}"))?;
                let welcomed_mission = handle
                    .mission_id()
                    .expect("successful Welcome must include a mission id");
                let assigned_seat = handle
                    .assigned_seat()
                    .expect("successful Welcome must assign a local seat");
                host.transport.local_seat = assigned_seat;
                if welcomed_mission != authoritative_mission_id {
                    return Err(format!(
                        "multiplayer: host mission `{welcomed_mission}` does not match requested mission `{authoritative_mission_id}`"
                    ));
                }
                #[cfg(target_arch = "wasm32")]
                let speech_timing_locale = handle
                    .speech_timing_authority()
                    .expect("successful browser Welcome must publish speech timing authority");
                #[cfg(not(target_arch = "wasm32"))]
                let speech_timing_locale = handle.speech_timing_locale();
                if let Some(authoritative_locale) = speech_timing_locale.as_deref() {
                    let has_timing_pack = host
                        .application_context
                        .installed_languages()
                        .map_err(|error| {
                            format!("multiplayer: cannot inspect installed voice packs: {error}")
                        })?
                        .into_iter()
                        .any(|pack| pack.locale == authoritative_locale && pack.has_voice);
                    if !has_timing_pack {
                        return Err(format!(
                            "multiplayer: host requires voice pack `{authoritative_locale}` for deterministic speech timing, but that validated pack is not installed"
                        ));
                    }
                }
                host.transport.mission_id = Some(welcomed_mission.to_string());
                if let Some(seed) = handle.mission_seed() {
                    host.transport.mission_seed = Some(seed);
                }
                host.transport.mission_sim_config = handle.mission_sim_config();
                host.transport.speech_timing_locale = speech_timing_locale;
                tracing::info!(
                    server = %addr,
                    nickname = %nickname,
                    "multiplayer: connected to {addr}"
                );
                // Wait briefly for the AssignedLocalSeat event so
                // host.transport.local_seat is correct before the mission
                // starts emitting outgoing inputs.  Long timeouts
                // get logged but don't abort — inputs queued before
                // the assignment lands just sit in the channel until
                // the I/O thread drains them.  Skipped on wasm —
                // blocking on a channel would freeze the browser
                // event loop, so we let the per-frame
                // `drain_net_inputs` pick up the AssignedLocalSeat
                // event when it arrives.
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while Instant::now() < deadline {
                        match channels.incoming.recv_timeout(Duration::from_millis(100)) {
                            Ok(NetEvent::AssignedLocalSeat(seat)) => {
                                host.transport.local_seat = seat;
                                tracing::info!(?seat, "multiplayer: assigned seat");
                                break;
                            }
                            Ok(NetEvent::Note(s)) => tracing::info!(note = %s, "mp note"),
                            Ok(event) => channels.defer_events(vec![event]),
                            Err(_) => continue,
                        }
                    }
                }
                channels.attach_runtime(handle);
                host.transport.net = Some(channels);
            }
            Err(e) => {
                return Err(format!("multiplayer: failed to connect to {addr}: {e}"));
            }
        }
    }
    Ok(())
}

fn resolve_browser_join_publication(args: &crate::main_entry::CliArgs) -> Result<bool, String> {
    let saved = args
        .global_options
        .active_profile_snapshot()
        .map(|profile| profile.multiplayer_config.publish_browser_join_links)
        .map_err(|error| {
            format!("multiplayer: cannot read browser publication preference: {error}")
        })?;
    Ok(resolve_publication_preference(
        args.mp_browser_join_links,
        saved,
    ))
}

fn resolve_publication_preference(cli_override: Option<bool>, saved: bool) -> bool {
    cli_override.unwrap_or(saved)
}

fn validate_multiplayer_launch_args(args: &crate::main_entry::CliArgs) -> Result<(), String> {
    if args.server && args.connect.is_some() {
        return Err("multiplayer host and client modes are mutually exclusive".to_string());
    }
    if let Some(expected) = args.mp_expected_players
        && !(1..=crate::multiplayer::join_ticket::MAX_MULTIPLAYER_PLAYERS).contains(&expected)
    {
        return Err(format!(
            "multiplayer expected player count must be between 1 and {}",
            crate::multiplayer::join_ticket::MAX_MULTIPLAYER_PLAYERS
        ));
    }
    let multiplayer = args.server || args.connect.is_some();
    let replay = args.replay.is_some() || args.replay_data.is_some();
    if multiplayer && replay {
        return Err(
            "multiplayer cannot be combined with replay playback; Welcome mission/seed/SimConfig must be the sole frame-0 authority"
                .to_string(),
        );
    }
    if args.connect.is_some() && args.record.is_some() {
        return Err(
            "multiplayer peers cannot choose a replay output; only the host records the canonical ordered session"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        MultiplayerAdmissionEvent, TimelineFrame, drain_net_inputs, resolve_publication_preference,
        rewind_from_recent_timeline_history, validate_multiplayer_launch_args,
    };
    use crate::host::Host;
    use crate::multiplayer::{NetChannels, NetEvent, NetOutbound};
    use crate::rewind::RewindBuffer;
    use crate::sim_timeline::RestorePolicy;
    use robin_engine::campaign::Campaign;
    use robin_engine::engine::{Engine, LevelAssets};
    use robin_engine::engine_manager::EngineManager;
    use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};

    fn network_drain_fixture() -> (
        Host,
        EngineManager,
        LevelAssets,
        std::sync::mpsc::Sender<NetEvent>,
        std::sync::mpsc::Receiver<NetOutbound>,
    ) {
        let mut assets = LevelAssets::new();
        let engine = Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets)
            .expect("fixture engine");
        let manager = EngineManager::new(engine);
        let (channels, incoming, outgoing, _, _) = NetChannels::new();
        let mut host = Host::default();
        host.transport.local_seat = PlayerId(1);
        host.transport.net = Some(channels);
        (host, manager, assets, incoming, outgoing)
    }

    #[test]
    fn multiplayer_rejects_replay_before_engine_construction() {
        let args = crate::main_entry::CliArgs {
            connect: Some("127.0.0.1:7878".to_string()),
            replay: Some("session.rhrec.jsonl".to_string()),
            ..Default::default()
        };
        assert!(validate_multiplayer_launch_args(&args).is_err());

        let multiplayer_only = crate::main_entry::CliArgs {
            connect: Some("127.0.0.1:7878".to_string()),
            ..Default::default()
        };
        assert!(validate_multiplayer_launch_args(&multiplayer_only).is_ok());

        let peer_recording = crate::main_entry::CliArgs {
            connect: Some("127.0.0.1:7878".to_string()),
            record: Some("peer-is-not-canonical.rhrec.jsonl".to_string()),
            ..Default::default()
        };
        assert!(
            validate_multiplayer_launch_args(&peer_recording)
                .unwrap_err()
                .contains("only the host records")
        );
    }

    #[test]
    fn browser_publication_cli_override_precedes_saved_preference() {
        assert!(resolve_publication_preference(None, true));
        assert!(!resolve_publication_preference(None, false));
        assert!(resolve_publication_preference(Some(true), false));
        assert!(!resolve_publication_preference(Some(false), true));
    }

    #[test]
    fn multiplayer_launch_rejects_ambiguous_mode_and_player_count() {
        let both = crate::main_entry::CliArgs {
            server: true,
            connect: Some("host".to_string()),
            ..Default::default()
        };
        assert!(validate_multiplayer_launch_args(&both).is_err());

        let too_many = crate::main_entry::CliArgs {
            server: true,
            mp_expected_players: Some(5),
            ..Default::default()
        };
        assert!(validate_multiplayer_launch_args(&too_many).is_err());
    }

    #[test]
    fn snapshot_is_adopted_before_ready_is_announced() {
        let (mut host, mut manager, assets, incoming, outgoing) = network_drain_fixture();
        let mut snapshot = manager.engine.clone();
        snapshot
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::new(vec![
                    PlayerCommand::SetAmountOfSpeaking { amount: 9 }.into(),
                    PlayerCommand::SetUnbindingEnabled { enabled: false }.into(),
                ])
                .with_hourglass(false),
            )
            .expect("snapshot command admission");
        let engine_bytes = snapshot.encode_native_snapshot();
        incoming
            .send(NetEvent::InitialSnapshot {
                frame: 0,
                engine_bytes,
            })
            .expect("queue snapshot");
        let mut rewind = RewindBuffer::new();
        let mut hashes = std::collections::BTreeMap::new();
        let mut pending = std::collections::BTreeMap::new();
        let drain = drain_net_inputs(
            &mut host,
            &mut manager,
            0,
            &mut pending,
            &assets,
            &mut rewind,
            &mut hashes,
        );

        assert!(drain.rewrote_sim_state);
        assert_eq!(manager.engine.sim_config().amount_of_speaking, 9);
        assert!(!manager.engine.sim_config().enable_unbinding);
        assert_eq!(
            drain.admission_events,
            [MultiplayerAdmissionEvent::InitialSnapshotAdopted { frame: 0 }]
        );
        assert!(matches!(
            outgoing.recv().expect("ReadyToSim after adoption"),
            NetOutbound::ReadyToSim { frame: 0 }
        ));
    }

    #[test]
    fn mid_mission_snapshot_seeds_history_between_sparse_boundaries() {
        let (mut host, mut manager, assets, incoming, outgoing) = network_drain_fixture();
        let engine_bytes = manager.engine.encode_native_snapshot();
        incoming
            .send(NetEvent::InitialSnapshot {
                frame: 32,
                engine_bytes,
            })
            .expect("queue mid-mission snapshot");
        let mut rewind = RewindBuffer::new();
        let mut hashes = std::collections::BTreeMap::new();
        let mut pending = std::collections::BTreeMap::new();

        let drain = drain_net_inputs(
            &mut host,
            &mut manager,
            31,
            &mut pending,
            &assets,
            &mut rewind,
            &mut hashes,
        );
        assert_eq!(drain.adopted_frame, Some(32));
        assert!(matches!(
            outgoing.recv().expect("ReadyToSim after adoption"),
            NetOutbound::ReadyToSim { frame: 32 }
        ));

        rewind.begin_frame(32, &manager.engine, &assets);
        rewind.end_frame(Vec::new());
        assert!(rewind.frame_for(32).is_some());
        assert_eq!(rewind.oldest_cmd_frame(), 32);
    }

    #[test]
    fn reconnect_adopts_older_host_snapshot_and_discards_prediction_future() {
        let (mut host, mut manager, assets, incoming, outgoing) = network_drain_fixture();
        host.transport.reconnecting = true;
        let engine_bytes = manager.engine.encode_native_snapshot();
        incoming
            .send(NetEvent::InitialSnapshot {
                frame: 30,
                engine_bytes,
            })
            .expect("queue reconnect snapshot");
        let mut rewind = RewindBuffer::new();
        let mut hashes = std::collections::BTreeMap::from([(36, 0xBAD5_EED)]);
        let mut pending = std::collections::BTreeMap::from([(
            super::TimelineFrame::from_wire(36),
            vec![PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown)],
        )]);

        let drain = drain_net_inputs(
            &mut host,
            &mut manager,
            35,
            &mut pending,
            &assets,
            &mut rewind,
            &mut hashes,
        );

        assert_eq!(drain.adopted_frame, Some(30));
        assert!(drain.rewrote_sim_state);
        assert!(
            host.transport.reconnecting,
            "controls stay disabled until the ready barrier releases"
        );
        assert!(
            pending.is_empty(),
            "old predicted inputs must not cross sessions"
        );
        assert!(hashes.is_empty(), "old peer hashes must not cross sessions");
        assert_eq!(rewind.oldest_reachable_frame(), Some(30));
        assert!(rewind.restore_recent(30, RestorePolicy::Exact).is_some());
        assert!(matches!(
            outgoing
                .recv()
                .expect("ReadyToSim after reconnect adoption"),
            NetOutbound::ReadyToSim { frame: 30 }
        ));
        incoming
            .send(NetEvent::BeginSim {
                frame: 30,
                start_epoch_ms: 123,
            })
            .unwrap();
        let _ = drain_net_inputs(
            &mut host,
            &mut manager,
            30,
            &mut pending,
            &assets,
            &mut rewind,
            &mut hashes,
        );
        assert!(!host.transport.reconnecting);
    }

    #[test]
    #[should_panic(expected = "fatal multiplayer session error: test transport failure")]
    fn fatal_transport_event_fails_the_mission_drain_loudly() {
        let (mut host, mut manager, assets, incoming, _outgoing) = network_drain_fixture();
        incoming
            .send(NetEvent::Fatal("test transport failure".into()))
            .expect("queue fatal event");
        let mut rewind = RewindBuffer::new();
        let mut hashes = std::collections::BTreeMap::new();
        let mut pending = std::collections::BTreeMap::new();
        let _ = drain_net_inputs(
            &mut host,
            &mut manager,
            0,
            &mut pending,
            &assets,
            &mut rewind,
            &mut hashes,
        );
    }

    fn rewind_with_horizon(
        manager: &EngineManager,
        assets: &LevelAssets,
        start: u32,
        end: u32,
    ) -> RewindBuffer {
        let mut rewind = RewindBuffer::new();
        for frame in start..end {
            rewind.begin_frame(frame, &manager.engine, assets);
            rewind.end_frame(Vec::new());
        }
        assert_eq!(rewind.oldest_cmd_frame(), start);
        rewind
    }

    #[test]
    fn client_too_old_input_requests_a_complete_snapshot_reconnect() {
        let (mut host, mut manager, assets, incoming, outgoing) = network_drain_fixture();
        let mut rewind = rewind_with_horizon(&manager, &assets, 25, 35);
        incoming
            .send(NetEvent::Input {
                server_frame: 35,
                origin_frame: 23,
                target_frame: 24,
                input: PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown),
            })
            .expect("queue stale input");
        let mut hashes = std::collections::BTreeMap::new();
        let mut pending = std::collections::BTreeMap::new();

        let drain = drain_net_inputs(
            &mut host,
            &mut manager,
            35,
            &mut pending,
            &assets,
            &mut rewind,
            &mut hashes,
        );

        assert!(host.transport.reconnecting);
        assert!(pending.is_empty());
        assert_eq!(
            drain.admission_events,
            [MultiplayerAdmissionEvent::Disconnected]
        );
        assert!(drain.inputs.is_empty());
        assert!(matches!(
            outgoing.try_recv().expect("reconnect request"),
            NetOutbound::ReconnectForSnapshot {
                player_id: PlayerId(1),
                reason,
            } if reason.contains("rollback horizon")
        ));
    }

    #[test]
    fn host_too_old_peer_input_reconnects_every_predicting_client() {
        let (mut host, mut manager, assets, incoming, outgoing) = network_drain_fixture();
        host.transport.local_seat = PlayerId::HOST;
        let mut rewind = rewind_with_horizon(&manager, &assets, 25, 35);
        incoming
            .send(NetEvent::Input {
                server_frame: 35,
                origin_frame: 23,
                target_frame: 24,
                input: PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown),
            })
            .expect("queue stale peer input");
        let mut hashes = std::collections::BTreeMap::new();
        let mut pending = std::collections::BTreeMap::new();

        let drain = drain_net_inputs(
            &mut host,
            &mut manager,
            35,
            &mut pending,
            &assets,
            &mut rewind,
            &mut hashes,
        );

        assert!(!host.transport.reconnecting);
        assert!(drain.inputs.is_empty());
        assert!(matches!(
            outgoing.try_recv().expect("reconnect-all request"),
            NetOutbound::ReconnectAllForSnapshot { reason }
                if reason.contains("rollback horizon")
        ));
    }

    #[test]
    fn failed_recent_history_rebuild_does_not_publish_partial_checkpoints() {
        let (_host, manager, assets, _incoming, _outgoing) = network_drain_fixture();
        let mut rewind = RewindBuffer::new();
        for frame in 0..2 {
            rewind.begin_frame(frame, &manager.engine, &assets);
            rewind.end_frame(Vec::new());
        }
        for frame in 1..=3 {
            rewind.checkpoint_recent(frame, &manager.engine);
        }

        // Frame 2 has no command entry, so reconstruction from frame 1 to 3
        // must fail after doing some work without truncating frames 2 and 3.
        assert!(rewind_from_recent_timeline_history(3, &assets, &mut rewind, 1, 1).is_none());
        assert!(rewind.restore_recent(2, RestorePolicy::Exact).is_some());
        assert!(rewind.restore_recent(3, RestorePolicy::Exact).is_some());
    }
}
