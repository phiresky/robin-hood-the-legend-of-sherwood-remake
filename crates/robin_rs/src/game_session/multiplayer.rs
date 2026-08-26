//! Multiplayer session helpers extracted from `game_session`:
//! transport setup, per-frame net input drain, and rollback on
//! late inputs.

use crate::host::Host;
use crate::rewind::RewindBuffer;
use crate::sim_timeline::{RestorePolicy, SnapshotHistory, replay_authoritative_frame_profiled};
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
/// Native disconnects remain synchronized only while the transport's real
/// reconnect loop is active. Browser disconnects arrive as `Fatal` because
/// wasm has no reconnect implementation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_net_inputs(
    host: &mut Host,
    manager: &mut engine_manager_api::EngineManager,
    assets: &LevelAssets,
    rewind_buffer: &mut RewindBuffer,
    peer_hashes: &mut std::collections::BTreeMap<u32, u64>,
    recent_timeline_history: &mut SnapshotHistory,
) -> NetDrainResult {
    use crate::multiplayer::NetEvent;

    let Some(net) = host.transport.net.as_ref() else {
        // Not in a session — drain anything sitting in pending and
        // return.  Pending should be empty in single-player but is
        // safe to flush.
        return NetDrainResult {
            inputs: manager
                .pending_inputs
                .remove(&manager.sim_frame)
                .unwrap_or_default(),
            rewrote_sim_state: false,
            admission_events: Vec::new(),
            pause_simulation: false,
            latest_host_clock_sample: None,
            rollback: None,
        };
    };

    // 1. Drain transport into "future" and "late" buckets.
    let mut late_inputs: Vec<(u32, PlayerInput)> = Vec::new();
    let mut rewrote_sim_state = false;
    let mut admission_events = Vec::new();
    let mut latest_host_clock_sample: Option<(u32, u32)> = None;
    let mut rollback_telemetry = None;
    while let Ok(event) = net.try_recv_event() {
        match event {
            NetEvent::Input {
                server_frame,
                origin_frame,
                target_frame,
                input,
            } => {
                if target_frame >= manager.sim_frame {
                    manager
                        .pending_inputs
                        .entry(target_frame)
                        .or_default()
                        .push(input);
                } else {
                    tracing::info!(
                        local_frame = manager.sim_frame,
                        server_frame,
                        origin_frame,
                        target_frame,
                        late_by = manager.sim_frame.saturating_sub(target_frame),
                        local_minus_server = manager.sim_frame as i64 - server_frame as i64,
                        local_minus_origin = manager.sim_frame as i64 - origin_frame as i64,
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
                #[cfg(not(target_arch = "wasm32"))]
                {
                    tracing::warn!(
                        "multiplayer: peer disconnected — transport will auto-reconnect; \
                         simulation is held until an authoritative snapshot arrives"
                    );
                    host.transport.reconnecting = true;
                    admission_events.push(MultiplayerAdmissionEvent::Disconnected);
                }
                #[cfg(target_arch = "wasm32")]
                panic!(
                    "fatal multiplayer session error: browser transport disconnected and automatic reconnect is unavailable"
                );
            }
            NetEvent::Reconnected => {
                tracing::info!("multiplayer: transport reconnected; awaiting host snapshot");
            }
            NetEvent::MissionConfig {
                mission_id,
                rng_seed,
                sim_config,
            } => {
                // Welcome is awaited before Engine construction; retain the
                // event copy for diagnostics and reconnect validation.
                if host.transport.mission_id.as_deref() != Some(mission_id.as_str())
                    || host.transport.mission_seed != Some(rng_seed)
                    || host.transport.mission_sim_config != Some(sim_config)
                {
                    panic!(
                        "fatal multiplayer session error: Welcome/reconnect mission construction state changed"
                    );
                }
                host.transport.mission_seed = Some(rng_seed);
                host.transport.mission_sim_config = Some(sim_config);
                host.transport.mission_id = Some(mission_id);
            }
            NetEvent::Fatal(message) => panic!("fatal multiplayer session error: {message}"),
            NetEvent::InitialSnapshot {
                frame,
                engine_bytes,
            } => {
                if frame < manager.sim_frame {
                    tracing::debug!(
                        frame,
                        local_sim_frame = manager.sim_frame,
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
                if frame == 0 && manager.sim_frame == 0 {
                    let local_hash = robin_engine::replay::state_hash(&manager.engine);
                    match bincode::serde::decode_from_slice::<Engine, _>(
                        &engine_bytes,
                        bincode::config::standard(),
                    ) {
                        Ok((snapshot, _)) => {
                            let snap_hash = robin_engine::replay::state_hash(&snapshot);
                            if local_hash == snap_hash {
                                host.transport.reconnecting = false;
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
                            } else {
                                match manager.engine.try_adopt_snapshot(snapshot, assets) {
                                    Ok(()) => {
                                        host.transport.reconnecting = false;
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
                                        manager.drop_pending_inputs_before(frame);
                                        recent_timeline_history.clear();
                                        peer_hashes.retain(|&f, _| f >= frame);
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
                match bincode::serde::decode_from_slice::<Engine, _>(
                    &engine_bytes,
                    bincode::config::standard(),
                ) {
                    Ok((snapshot, _)) => {
                        match manager.engine.try_adopt_snapshot(snapshot, assets) {
                            Ok(()) => {
                                host.transport.reconnecting = false;
                                admission_events.push(
                                    MultiplayerAdmissionEvent::InitialSnapshotAdopted { frame },
                                );
                                let adopted_hash =
                                    robin_engine::replay::state_hash(&manager.engine);
                                tracing::info!(
                                    frame,
                                    local_sim_frame = manager.sim_frame,
                                    bytes = engine_bytes.len(),
                                    adopted_hash = format!("{adopted_hash:016x}"),
                                    "multiplayer: adopting host's engine snapshot"
                                );
                                manager.set_sim_frame(frame);
                                if let Some(net) = host.transport.net.as_ref() {
                                    net.send_ready_to_sim(frame);
                                }
                                *rewind_buffer = RewindBuffer::new();
                                manager.drop_pending_inputs_before(frame);
                                recent_timeline_history.clear();
                                peer_hashes.retain(|&f, _| f >= frame);
                                rewrote_sim_state = true;
                            }
                            Err(error) => panic!(
                                "multiplayer: rejected incompatible host snapshot at frame {frame}: {error}"
                            ),
                        }
                    }
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
                tracing::info!(
                    frame,
                    start_epoch_ms,
                    "multiplayer: begin-sim barrier released"
                );
                if manager.sim_frame != frame {
                    manager.set_sim_frame(frame);
                    manager.drop_pending_inputs_before(frame);
                    recent_timeline_history.clear();
                    peer_hashes.retain(|&f, _| f >= frame);
                    rewrote_sim_state = true;
                }
                admission_events.push(MultiplayerAdmissionEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                });
            }
            NetEvent::ModalDismiss { kind, result } => {
                tracing::debug!(
                    ?kind,
                    ?result,
                    "multiplayer: modal dismissal reached main drain after modal closed"
                );
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
        let mut earliest = u32::MAX;
        let mut late_input_count = 0usize;
        for (frame, input) in late_inputs {
            if rewind_buffer.splice_late_input(frame, input.clone()) {
                needs_rewind = true;
                earliest = earliest.min(frame);
                late_input_count += 1;
            } else {
                tracing::error!(
                    target_frame = frame,
                    oldest = rewind_buffer.oldest_cmd_frame(),
                    "multiplayer: late input below rewind horizon — applying at current frame as degraded fallback"
                );
                manager
                    .pending_inputs
                    .entry(manager.sim_frame)
                    .or_default()
                    .push(input);
            }
        }
        if needs_rewind {
            let rollback_start = web_time::Instant::now();
            if let Some((new_engine, mut telemetry)) = rewind_from_recent_timeline_history(
                manager.sim_frame,
                assets,
                rewind_buffer,
                recent_timeline_history,
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
            } else if let Some(new_engine) = rewind_buffer.rewind_to(assets, manager.sim_frame) {
                let telemetry = MultiplayerRollbackTelemetry {
                    path: "rewind-buffer",
                    earliest_frame: earliest,
                    target_frame: manager.sim_frame,
                    late_input_count,
                    replayed_frames: manager.sim_frame.saturating_sub(earliest),
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
                recent_timeline_history.truncate_after(earliest);
                rollback_telemetry = Some(telemetry);
                rewrote_sim_state = true;
            } else {
                // The late input was accepted into the authoritative command
                // journal, so checkpoints after its frame are stale even
                // when reconstruction cannot currently reach `sim_frame`.
                recent_timeline_history.truncate_after(earliest);
                tracing::error!(
                    earliest_frame = earliest,
                    target_frame = manager.sim_frame,
                    late_inputs = late_input_count,
                    "multiplayer rollback failed: no retained snapshot could reconstruct timeline"
                );
            }
        }
    }

    // 3. Return inputs scheduled for this frame.  The caller applies
    //    them to the live engine and folds them into `frame_cmds` so
    //    the recorder + rewind buffer capture them.
    let mut due_inputs = manager
        .pending_inputs
        .remove(&manager.sim_frame)
        .unwrap_or_default();
    canonicalize_player_input_order(&mut due_inputs);

    NetDrainResult {
        inputs: due_inputs,
        rewrote_sim_state,
        admission_events,
        pause_simulation: false,
        latest_host_clock_sample,
        rollback: rollback_telemetry,
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
    if let Some(net) = host.transport.net.as_ref() {
        net.publish_frame(manager.sim_frame);
    }
    let mut drain = drain_net_inputs(
        host,
        manager,
        assets,
        &mut timeline.rewind_buffer,
        &mut timeline.peer_hashes,
        &mut timeline.recent_timeline_history,
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

    let local_is_peer = host.transport.net.is_some()
        && host.transport.local_seat != robin_engine::player_command::PlayerId::HOST;
    if local_is_peer
        && let Some((clock_frame, ms_until_next_frame)) = drain.latest_host_clock_sample
    {
        accept_host_frame_schedule(
            &mut timeline.mp_host_frame_schedule,
            clock_frame,
            ms_until_next_frame,
            manager.sim_frame,
        );
    }

    let admission_pause = timeline.multiplayer_admission_paused(now_epoch_ms);
    let mut clock_pause = false;
    if local_is_peer && !admission_pause {
        if let Some(deadline_ms) =
            host_scheduled_frame_deadline_ms(timeline.mp_host_frame_schedule, manager.sim_frame)
        {
            let now_ms = crate::window::process_uptime_ms();
            let until_frame_ms = deadline_ms - i64::from(now_ms);
            if until_frame_ms > 0 {
                clock_pause = true;
                if now_ms.saturating_sub(timeline.last_mp_clock_ahead_log_ms) >= 1000 {
                    timeline.last_mp_clock_ahead_log_ms = now_ms;
                    tracing::info!(
                        scheduled_frame = timeline.mp_host_frame_schedule.map(|(frame, _)| frame),
                        local_frame = manager.sim_frame,
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
            .recent_timeline_history
            .checkpoint(manager.sim_frame, &manager.engine);
    }
    drain
}

fn rewind_from_recent_timeline_history(
    target_frame: u32,
    assets: &LevelAssets,
    rewind_buffer: &RewindBuffer,
    recent_timeline_history: &mut SnapshotHistory,
    start_frame: u32,
    late_input_count: usize,
) -> Option<(Engine, MultiplayerRollbackTelemetry)> {
    let restore_start = web_time::Instant::now();
    let mut snapshot = recent_timeline_history
        .restore(start_frame, RestorePolicy::Exact)
        .ok()?;
    let restore_us = restore_start.elapsed().as_micros();

    // Rebuild corrected checkpoints transactionally. A missing command (or
    // any future fallible replay input) must leave the last known-good recent
    // history available to a fallback path rather than publishing a partial
    // reconstruction.
    let mut corrected_history = recent_timeline_history.clone();
    corrected_history.truncate_after(start_frame);
    let mut scratch_host = Host::default();
    let mut scratch_dev = engine_api::DevState::default();
    let mut scratch_display = engine_api::HostDisplayState::default();
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
        let frame_timing = replay_authoritative_frame_profiled(
            &mut snapshot,
            &mut scratch_display,
            assets,
            &mut scratch_host,
            &mut scratch_dev,
            frame,
        );
        replay_apply_us += frame_timing.apply_us;
        replay_tick_us += frame_timing.tick_us;
    }
    let remember_start = web_time::Instant::now();
    corrected_history.remember(snapshot.clone());
    replay_remember_us += remember_start.elapsed().as_micros();
    let replay_us = replay_start.elapsed().as_micros();
    *recent_timeline_history = corrected_history;

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
            let (mut channels, in_tx, out_rx, frame_cursor, snapshot_slot) = NetChannels::new();
            match start_server(
                nickname.clone(),
                authoritative_mission_id.to_string(),
                authoritative_rng_seed,
                authoritative_sim_config,
                in_tx,
                out_rx,
                frame_cursor,
                snapshot_slot,
                args.mp_expected_players.unwrap_or(1),
            ) {
                Ok(handle) => {
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
                        || handle.mission_sim_config().is_none())
                        && web_time::Instant::now() < deadline
                    {
                        crate::window::sleep_ms(10).await;
                    }
                    if handle.mission_id().is_none()
                        || handle.mission_seed().is_none()
                        || handle.mission_sim_config().is_none()
                    {
                        return Err(
                            "multiplayer: timed out awaiting authoritative Welcome before Engine construction"
                                .to_string(),
                        );
                    }
                }
                let welcomed_mission = handle
                    .mission_id()
                    .expect("successful Welcome must include a mission id");
                if welcomed_mission != authoritative_mission_id {
                    return Err(format!(
                        "multiplayer: host mission `{welcomed_mission}` does not match requested mission `{authoritative_mission_id}`"
                    ));
                }
                host.transport.mission_id = Some(welcomed_mission.to_string());
                if let Some(seed) = handle.mission_seed() {
                    host.transport.mission_seed = Some(seed);
                }
                host.transport.mission_sim_config = handle.mission_sim_config();
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

fn validate_multiplayer_launch_args(args: &crate::main_entry::CliArgs) -> Result<(), String> {
    let multiplayer = args.server || args.connect.is_some();
    let replay = args.replay.is_some() || args.replay_data.is_some();
    if multiplayer && replay {
        return Err(
            "multiplayer cannot be combined with replay playback; Welcome mission/seed/SimConfig must be the sole frame-0 authority"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        MultiplayerAdmissionEvent, drain_net_inputs, rewind_from_recent_timeline_history,
        validate_multiplayer_launch_args,
    };
    use crate::host::Host;
    use crate::multiplayer::{NetChannels, NetEvent, NetOutbound};
    use crate::rewind::RewindBuffer;
    use crate::sim_timeline::{CheckpointPolicy, RestorePolicy, RetentionPolicy, SnapshotHistory};
    use robin_engine::campaign::Campaign;
    use robin_engine::engine::{Engine, LevelAssets};
    use robin_engine::engine_manager::EngineManager;
    use robin_engine::player_command::{PlayerCommand, PlayerId};

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
        let manager = EngineManager::new(engine, PlayerId(1));
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
    }

    #[test]
    fn snapshot_is_adopted_before_ready_is_announced() {
        let (mut host, mut manager, assets, incoming, outgoing) = network_drain_fixture();
        let mut snapshot = manager.engine.clone();
        snapshot
            .advance_frame(
                &mut robin_engine::engine::HostDisplayState::default(),
                &mut robin_engine::engine::InputState::default(),
                &assets,
                &mut robin_engine::engine::DevState::default(),
                robin_engine::engine::SimulationFrameInput::new(vec![
                    PlayerCommand::SetAmountOfSpeaking { amount: 9 }.into(),
                ])
                .with_hourglass(false),
            )
            .expect("snapshot command admission");
        let engine_bytes = bincode::serde::encode_to_vec(&snapshot, bincode::config::standard())
            .expect("serialize snapshot");
        incoming
            .send(NetEvent::InitialSnapshot {
                frame: 0,
                engine_bytes,
            })
            .expect("queue snapshot");
        let mut rewind = RewindBuffer::new();
        let mut hashes = std::collections::BTreeMap::new();
        let mut recent = SnapshotHistory::new(
            CheckpointPolicy::EveryFrame,
            RetentionPolicy::Latest { capacity: 8 },
        );

        let drain = drain_net_inputs(
            &mut host,
            &mut manager,
            &assets,
            &mut rewind,
            &mut hashes,
            &mut recent,
        );

        assert!(drain.rewrote_sim_state);
        assert_eq!(manager.engine.sim_config().amount_of_speaking, 9);
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
    #[should_panic(expected = "fatal multiplayer session error: test transport failure")]
    fn fatal_transport_event_fails_the_mission_drain_loudly() {
        let (mut host, mut manager, assets, incoming, _outgoing) = network_drain_fixture();
        incoming
            .send(NetEvent::Fatal("test transport failure".into()))
            .expect("queue fatal event");
        let mut rewind = RewindBuffer::new();
        let mut hashes = std::collections::BTreeMap::new();
        let mut recent = SnapshotHistory::new(
            CheckpointPolicy::EveryFrame,
            RetentionPolicy::Latest { capacity: 8 },
        );

        let _ = drain_net_inputs(
            &mut host,
            &mut manager,
            &assets,
            &mut rewind,
            &mut hashes,
            &mut recent,
        );
    }

    #[test]
    fn failed_recent_history_rebuild_does_not_publish_partial_checkpoints() {
        let (_host, manager, assets, _incoming, _outgoing) = network_drain_fixture();
        let mut rewind = RewindBuffer::new();
        for frame in 0..2 {
            rewind.begin_frame(frame, &manager.engine, &assets);
            rewind.end_frame(Vec::new());
        }
        let mut recent = SnapshotHistory::new(
            CheckpointPolicy::EveryFrame,
            RetentionPolicy::Latest { capacity: 8 },
        );
        for frame in 1..=3 {
            assert!(recent.checkpoint(frame, &manager.engine));
        }

        // Frame 2 has no command entry, so reconstruction from frame 1 to 3
        // must fail after doing some work without truncating frames 2 and 3.
        assert!(
            rewind_from_recent_timeline_history(3, &assets, &rewind, &mut recent, 1, 1).is_none()
        );
        assert!(recent.restore(2, RestorePolicy::Exact).is_ok());
        assert!(recent.restore(3, RestorePolicy::Exact).is_ok());
    }
}
