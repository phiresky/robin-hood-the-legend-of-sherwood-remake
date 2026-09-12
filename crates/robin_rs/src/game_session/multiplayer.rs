//! Multiplayer session helpers extracted from `game_session`:
//! transport setup, per-frame net input drain, and rollback on
//! late inputs.

use super::runtime::TimelineFrame;
use crate::host::Host;
use crate::rewind::RewindBuffer;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::engine_manager as engine_manager_api;
use robin_engine::player_command::PlayerInput;
use robin_engine::sim_timeline::{RestorePolicy, replay_authoritative_frame_profiled};
use robin_engine::spellforge::SpellforgeRuntime;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum MultiplayerAdmissionEvent {
    Disconnected,
    InitialSnapshotAdopted { frame: u32 },
    HostResynchronizing { frame: u32 },
    BeginSim { frame: u32, start_epoch_ms: u64 },
}

fn canonicalize_player_input_order(inputs: &mut Vec<PlayerInput>) {
    inputs.sort_by_key(|input| input.player_id.0);
}

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
pub(super) enum MultiplayerSessionError {
    #[error("multiplayer protocol failure: {0}")]
    Protocol(String),
}

fn require_protocol(condition: bool, message: &str) -> Result<(), MultiplayerSessionError> {
    if condition {
        Ok(())
    } else {
        Err(MultiplayerSessionError::Protocol(message.to_owned()))
    }
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

/// Attach the host snapshot's exact embedded Spellforge package before the
/// normal immutable-asset compatibility preflight runs.
///
/// Peers deliberately do not need a second local copy of a custom-mission
/// archive: the versioned package bytes are authoritative engine state and
/// are already covered by the snapshot hash. The gameplay setting remains a
/// local opt-out, so a peer that disabled Spellforge rejects the session
/// instead of silently executing downloaded mission code.
fn attach_snapshot_spellforge_runtime(
    snapshot: &Engine,
    assets: &mut Arc<LevelAssets>,
    spellforge_enabled: impl FnOnce() -> Result<bool, String>,
) -> Result<(), String> {
    let Some(package) = snapshot.spellforge_package() else {
        return Ok(());
    };
    if assets.attachments.spellforge_runtime.is_some() {
        return Ok(());
    }
    if !spellforge_enabled()? {
        return Err(format!(
            "host snapshot requires Spellforge package {}, but Spellforge missions are disabled in Gameplay settings",
            robin_engine::spellforge::hex_hash(&package.sha256)
        ));
    }

    let runtime: Arc<dyn SpellforgeRuntime> = Arc::new(
        robin_spellforge::SpellforgeRuntime51::new(package.as_ref().clone())
            .map_err(|error| format!("host package validation failed: {error}"))?,
    );
    runtime.set_name_bindings(assets.scripts.names.as_ref().clone());
    Arc::make_mut(assets).attachments.spellforge_runtime = Some(runtime);
    Ok(())
}

/// Drain pending wire events from the multiplayer transport into current
/// frame inputs and apply any required network state corrections.
///
/// Also folds `AssignedLocalSeat` events (late seat-assignment
/// races) into `host.transport.local_seat()` and logs other diagnostic events.
/// Native and browser disconnects remain synchronized only while their real
/// transport reconnect loops are active. Both abandon the old prediction
/// future and wait for an authoritative replacement snapshot.
#[allow(clippy::too_many_arguments)]
pub(super) fn drain_net_inputs(
    host: &mut Host,
    manager: &mut engine_manager_api::EngineManager,
    current_frame: u32,
    network: &mut super::runtime::reconciliation::NetworkReconciliation,
    assets: &mut Arc<LevelAssets>,
    rewind_buffer: &mut RewindBuffer,
) -> Result<NetDrainResult, MultiplayerSessionError> {
    use crate::multiplayer::NetEvent;

    if host.transport.net().is_none() {
        // Not in a session — drain anything sitting in pending and
        // return.  Pending should be empty in single-player but is
        // safe to flush.
        return Ok(NetDrainResult {
            inputs: network.take_inputs(TimelineFrame::from_wire(current_frame)),
            rewrote_sim_state: false,
            admission_events: Vec::new(),
            pause_simulation: false,
            latest_host_clock_sample: None,
            rollback: None,
            adopted_frame: None,
        });
    }

    // 1. Drain transport into "future" and "late" buckets.
    let mut late_inputs: Vec<(u32, PlayerInput)> = Vec::new();
    let mut rewrote_sim_state = false;
    let mut admission_events = Vec::new();
    let mut latest_host_clock_sample: Option<(u32, u32)> = None;
    let mut rollback_telemetry = None;
    let mut effective_frame = current_frame;
    loop {
        let event = match host
            .transport
            .net()
            .expect("session channel remains installed during event drain")
            .try_recv_event()
        {
            Ok(event) => event,
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(MultiplayerSessionError::Protocol(format!(
                    "fatal multiplayer session error: transport worker closed its event channel"
                )));
            }
        };
        match event {
            NetEvent::Input {
                server_frame,
                origin_frame,
                target_frame,
                input,
            } => {
                if host.transport.local_seat() == robin_engine::player_command::PlayerId::HOST
                    && host.transport.reconnecting()
                {
                    // The replacement snapshot is already published. Events
                    // queued before the outbound disconnect must not mutate
                    // it through either a late rollback or a future command.
                    tracing::warn!(
                        target_frame,
                        origin_frame,
                        "multiplayer: discarded input from abandoned host prediction during snapshot resynchronization"
                    );
                    continue;
                }
                if target_frame >= effective_frame {
                    network.queue_input(TimelineFrame::from_wire(target_frame), input);
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
                host.transport.confirm_local_seat(seat);
            }
            NetEvent::Note(s) => tracing::info!(note = %s, "multiplayer: note"),
            NetEvent::Disconnected => {
                tracing::warn!(
                    "multiplayer: peer disconnected — transport will auto-reconnect; \
                     simulation is held until an authoritative snapshot arrives"
                );
                host.transport.await_authoritative_snapshot();
                admission_events.push(MultiplayerAdmissionEvent::Disconnected);
                // Everything derived from the disconnected process's future
                // is invalid. Events already drained from that generation
                // occur before Disconnected and are removed here; events from
                // the replacement stream arrive afterward.
                late_inputs.clear();
                network.abandon_prediction();
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
                if host.transport.mission_id() != Some(mission_id.as_str())
                    || host.transport.mission_seed() != Some(rng_seed)
                    || host.transport.mission_sim_config() != Some(sim_config)
                    || host.transport.speech_timing_locale() != speech_timing_locale.as_deref()
                {
                    return Err(MultiplayerSessionError::Protocol(format!(
                        "fatal multiplayer session error: Welcome/reconnect mission construction state changed"
                    )));
                }
            }
            NetEvent::ContentOffer(offer) => {
                return Err(MultiplayerSessionError::Protocol(format!(
                    "fatal multiplayer session error: host offered distributed mod {} after gameplay admission",
                    robin_engine::spellforge::hex_hash(&offer.full_mod_sha256)
                )));
            }
            NetEvent::ContentChunk {
                full_mod_sha256,
                offset,
                ..
            } => {
                return Err(MultiplayerSessionError::Protocol(format!(
                    "fatal multiplayer session error: host sent distributed-mod chunk {} at offset {offset} after gameplay admission",
                    robin_engine::spellforge::hex_hash(&full_mod_sha256)
                )));
            }
            NetEvent::Fatal(message) => {
                return Err(MultiplayerSessionError::Protocol(format!(
                    "fatal multiplayer session error: {message}"
                )));
            }
            NetEvent::InitialSnapshot {
                frame,
                engine_bytes,
            } => {
                let replacing_prediction_future = host.transport.reconnecting();
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
                                if let Some(net) = host.transport.net() {
                                    net.send_ready_to_sim(frame).map_err(|error| MultiplayerSessionError::Protocol(format!("fatal multiplayer readiness publication failure: {error}")))?;
                                }
                                if replacing_prediction_future {
                                    *rewind_buffer = RewindBuffer::new();
                                    rewind_buffer.seed_initial_anchor(frame, &manager.engine);
                                    network.abandon_prediction();
                                    rewrote_sim_state = true;
                                }
                            } else {
                                if let Err(error) =
                                    attach_snapshot_spellforge_runtime(&snapshot, assets, || {
                                        host.application_context().with_active_profile(|profile| {
                                            profile.gameplay_config.enable_spellforge_missions
                                        })
                                    })
                                {
                                    return Err(MultiplayerSessionError::Protocol(format!(
                                        "multiplayer: failed to attach frame-0 Spellforge runtime: {error}"
                                    )));
                                }
                                match Engine::adopt_authoritative_snapshot(
                                    snapshot,
                                    assets.as_ref(),
                                ) {
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
                                        network.adopt_snapshot(
                                            TimelineFrame::from_wire(frame),
                                            replacing_prediction_future,
                                        );
                                        rewrote_sim_state = true;
                                        if let Some(net) = host.transport.net() {
                                            net.send_ready_to_sim(frame).map_err(|error| MultiplayerSessionError::Protocol(format!("fatal multiplayer readiness publication failure: {error}")))?;
                                        }
                                    }
                                    Err(error) => {
                                        return Err(MultiplayerSessionError::Protocol(format!(
                                            "multiplayer: rejected incompatible frame-0 host snapshot: {error}"
                                        )));
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            return Err(MultiplayerSessionError::Protocol(format!(
                                "multiplayer: failed to deserialize frame-0 host snapshot: {e}"
                            )));
                        }
                    }
                    continue;
                }

                // Mid-mission rejoin (frame > 0): atomically adopt the host's
                // snapshot after attaching immutable script/grid/sprite data
                // once from the locally loaded LevelAssets.
                match Engine::decode_native_snapshot(&engine_bytes) {
                    Ok(snapshot) => {
                        if let Err(error) =
                            attach_snapshot_spellforge_runtime(&snapshot, assets, || {
                                host.application_context().with_active_profile(|profile| {
                                    profile.gameplay_config.enable_spellforge_missions
                                })
                            })
                        {
                            return Err(MultiplayerSessionError::Protocol(format!(
                                "multiplayer: failed to attach Spellforge runtime at frame {frame}: {error}"
                            )));
                        }
                        match Engine::adopt_authoritative_snapshot(snapshot, assets.as_ref()) {
                            Ok(adopted_engine) => {
                                manager.engine = adopted_engine;
                                admission_events.push(
                                    MultiplayerAdmissionEvent::InitialSnapshotAdopted { frame },
                                );
                                let adopted_hash =
                                    robin_engine::replay::state_hash(&manager.engine);
                                tracing::info!(
                                    frame,
                                    local_timeline_frame = effective_frame,
                                    bytes = engine_bytes.len(),
                                    adopted_hash = format!("{adopted_hash:016x}"),
                                    "multiplayer: adopting host's engine snapshot"
                                );
                                effective_frame = frame;
                                if let Some(net) = host.transport.net() {
                                    net.send_ready_to_sim(frame).map_err(|error| MultiplayerSessionError::Protocol(format!("fatal multiplayer readiness publication failure: {error}")))?;
                                }
                                *rewind_buffer = RewindBuffer::new();
                                rewind_buffer.seed_initial_anchor(frame, &manager.engine);
                                network.adopt_snapshot(
                                    TimelineFrame::from_wire(frame),
                                    replacing_prediction_future,
                                );
                                rewrote_sim_state = true;
                            }
                            Err(error) => {
                                return Err(MultiplayerSessionError::Protocol(format!(
                                    "multiplayer: rejected incompatible host snapshot at frame {frame}: {error}"
                                )));
                            }
                        }
                    }
                    Err(e) => {
                        return Err(MultiplayerSessionError::Protocol(format!(
                            "multiplayer: failed to deserialize host snapshot at frame {frame}: {e}"
                        )));
                    }
                }
            }
            NetEvent::PeerStateHash {
                frame,
                hash,
                clock_frame,
                ms_until_next_frame,
            } => {
                if let Some(hash) = hash {
                    network.admit_remote_hash(frame, hash);
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
                host.transport.begin_simulation();
                tracing::info!(
                    frame,
                    start_epoch_ms,
                    "multiplayer: begin-sim barrier released"
                );
                if effective_frame != frame {
                    effective_frame = frame;
                    let adopted = TimelineFrame::from_wire(frame);
                    network.adopt_snapshot(adopted, false);
                    rewind_buffer.clear_recent_checkpoints();
                    rewrote_sim_state = true;
                }
                admission_events.push(MultiplayerAdmissionEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                });
            }
            NetEvent::PrepareSnapshotTransition { id, payload } => {
                require_protocol(
                    host.transport.local_seat() != robin_engine::player_command::PlayerId::HOST,
                    "authoritative host received its own snapshot transition prepare",
                )?;
                require_protocol(
                    id.session_id
                        == host
                            .transport
                            .net()
                            .expect("admitted session retains its channels")
                            .session_id()
                            .map_err(|error| {
                                MultiplayerSessionError::Protocol(format!(
                                    "snapshot transition is missing session identity: {error}"
                                ))
                            })?,
                    "snapshot transition prepare belongs to another session",
                )?;
                require_protocol(
                    !host.transport.has_snapshot_transition(),
                    "received a second snapshot transition while one is pending",
                )?;
                let payload = match payload {
                    robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                        mission_id,
                        save_bytes,
                    } => {
                        let save: crate::save_file::GameSaveFile =
                            serde_json::from_slice(&save_bytes).map_err(|error| {
                                MultiplayerSessionError::Protocol(format!(
                                    "multiplayer snapshot transition payload is invalid: {error}"
                                ))
                            })?;
                        save.validate_current_schema().map_err(|error| MultiplayerSessionError::Protocol(format!(
                                "multiplayer snapshot transition current schema is invalid: {error:#}"
                            )))?;
                        require_protocol(
                            save.header.mission_id == mission_id,
                            "snapshot transition wire mission differs from its exact payload",
                        )?;
                        // Do not consult the active mission's profile graph at
                        // this network boundary. Once every peer commits, the
                        // session exits, drops its old mount, restores the
                        // exact descriptor, and only then validates/rebuilds
                        // the saved profile before constructing an Engine.
                        let reencoded = serde_json::to_vec(&save).map_err(|error| {
                            MultiplayerSessionError::Protocol(format!(
                                "multiplayer snapshot transition could not be re-encoded: {error}"
                            ))
                        })?;
                        require_protocol(
                            reencoded == save_bytes,
                            "snapshot transition bytes changed during validation",
                        )?;
                        crate::host::PendingSnapshotTransitionPayload::Save {
                            load: crate::host::SnapshotSave::Remote(Box::new(save)),
                        }
                    }
                    robin_engine::multiplayer::SnapshotTransitionPayload::CampaignExit {
                        exit_code,
                        engine_bytes,
                    } => {
                        require_protocol(
                            exit_code == robin_engine::game_operation::GameCode::LevelInterrupted,
                            "campaign transition may only launch the selected mission",
                        )?;
                        let decoded =
                            Engine::decode_native_snapshot(&engine_bytes).map_err(|error| {
                                MultiplayerSessionError::Protocol(format!(
                                    "multiplayer campaign snapshot is invalid: {error}"
                                ))
                            })?;
                        let adopted = Engine::adopt_authoritative_snapshot(decoded, assets)
                            .map_err(|error| {
                                MultiplayerSessionError::Protocol(format!(
                                    "multiplayer campaign snapshot cannot be adopted: {error}"
                                ))
                            })?;
                        require_protocol(
                            adopted.encode_native_snapshot() == engine_bytes,
                            "campaign transition bytes changed during validation",
                        )?;
                        crate::host::PendingSnapshotTransitionPayload::CampaignExit {
                            exit_code,
                            engine: Some(Box::new(adopted)),
                        }
                    }
                };
                host.transport.prepare_snapshot_transition(
                    crate::host::PendingSnapshotTransition::new(id, payload),
                );
                host.transport
                    .net()
                    .expect("prepared transition retains session")
                    .acknowledge_snapshot_transition(id)
                    .map_err(|error| {
                        MultiplayerSessionError::Protocol(format!(
                            "failed to acknowledge multiplayer snapshot transition: {error}"
                        ))
                    })?;
            }
            NetEvent::CommitSnapshotTransition { id } => {
                host.transport
                    .commit_snapshot_transition(id)
                    .map_err(MultiplayerSessionError::Protocol)?;
            }
            event @ (NetEvent::ModalProposal { .. } | NetEvent::ModalDecision { .. }) => {
                host.transport
                    .net()
                    .expect("admitted session retains its channels")
                    .defer_modal_event(event)
                    .map_err(|error| {
                        MultiplayerSessionError::Protocol(format!(
                            "fatal multiplayer modal routing error: {error}"
                        ))
                    })?;
            }
            event @ (NetEvent::RankedCoSignContext(_)
            | NetEvent::RankedSubmissionAccepted(_)
            | NetEvent::RankedOfficialSessionSetup(_)
            | NetEvent::RankedContinuationReceiptSelectionRequest(_)
            | NetEvent::RankedContinuationReceiptSelection { .. }
            | NetEvent::RankedContinuationPreflightClaim(_)
            | NetEvent::RankedContinuationPreflightSignature { .. }
            | NetEvent::LeaderboardCoSignRequest(_)
            | NetEvent::LeaderboardCoSignResponse { .. }) => {
                host.transport
                    .net()
                    .expect("admitted session retains its channels")
                    .defer_leaderboard_cosign_event(event)
                    .map_err(|error| {
                        MultiplayerSessionError::Protocol(format!(
                            "fatal multiplayer leaderboard co-sign routing error: {error}"
                        ))
                    })?;
            }
            NetEvent::RankedJoinChallenge(_)
            | NetEvent::RankedJoinResponse { .. }
            | NetEvent::RankedJoinAccepted(_)
            | NetEvent::RankedParticipantRoster(_)
            | NetEvent::RankedBrowseOnly { .. } => {
                // The authenticated transport has already validated these
                // admission/control events and applied them to the shared
                // ranked lifecycle. They must never enter deterministic
                // simulation input or the mission-end authorization inbox.
                tracing::debug!("multiplayer: consumed transport-owned ranked admission status");
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
            host.transport.local_seat() != robin_engine::player_command::PlayerId::HOST;
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
            host.transport
                .net()
                .expect("admitted session retains its channels")
                .reconnect_for_snapshot(host.transport.local_seat(), reason.clone())
                .map_err(|error| {
                    MultiplayerSessionError::Protocol(format!(
                        "failed to request multiplayer snapshot reconnect: {error}"
                    ))
                })?;
            host.transport.await_authoritative_snapshot();
            network.discard_pending_inputs();
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
        if let Some(reason) = host_reconnect_reason
            && !host.transport.reconnecting()
        {
            // An earlier ingress batch may already have reset the barrier.
            // Queued obsolete inputs from that abandoned prediction must not
            // start another generation while replacement peers are joining.
            admission_events.push(MultiplayerAdmissionEvent::HostResynchronizing {
                frame: effective_frame,
            });
            host.transport
                .net()
                .expect("admitted session retains its channels")
                .set_initial_snapshot(effective_frame, &manager.engine);
            host.transport
                .net()
                .expect("admitted session retains its channels")
                .reconnect_all_for_snapshot(reason)
                .map_err(|error| {
                    MultiplayerSessionError::Protocol(format!(
                        "failed to require multiplayer snapshot reconnect: {error}"
                    ))
                })?;
            // ReconnectAll resets host readiness as well as peer readiness.
            // Publish this exact held boundary into the replacement barrier.
            host.transport
                .net()
                .expect("admitted session retains its channels")
                .send_ready_to_sim(effective_frame)
                .map_err(|error| {
                    MultiplayerSessionError::Protocol(format!(
                        "fatal multiplayer readiness publication failure: {error}"
                    ))
                })?;
            host.transport.await_authoritative_snapshot();
            network.discard_pending_inputs();
        }
    }

    // 3. Return inputs scheduled for this frame.  The caller applies
    //    them to the live engine and folds them into `frame_cmds` so
    //    the recorder + rewind buffer capture them.
    let mut due_inputs = network.take_inputs(TimelineFrame::from_wire(effective_frame));
    canonicalize_player_input_order(&mut due_inputs);

    Ok(NetDrainResult {
        inputs: due_inputs,
        rewrote_sim_state,
        admission_events,
        pause_simulation: false,
        latest_host_clock_sample,
        rollback: rollback_telemetry,
        adopted_frame: (effective_frame != current_frame).then_some(effective_frame),
    })
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
    assets: &mut Arc<LevelAssets>,
    checkpoint_always: bool,
    now_epoch_ms: u64,
) -> Result<NetDrainResult, MultiplayerSessionError> {
    let current_frame = timeline.frame_number();
    if let Some(net) = host.transport.net() {
        net.publish_frame(current_frame);
    }
    let mut drain = timeline.drain_network_inputs(host, manager, assets)?;
    if let Some(rollback) = drain.rollback.clone() {
        timeline.invalidate_local_mp_hashes_after(rollback.earliest_frame);
        timeline.last_mp_rollback = Some(rollback);
    } else if drain.rewrote_sim_state {
        timeline.clear_local_mp_hashes();
    }
    timeline.apply_multiplayer_admission_events(&drain.admission_events)?;
    if let Some(frame) = drain.adopted_frame {
        timeline.adopt_frame(super::runtime::TimelineFrame::from_wire(frame));
    }

    let local_is_peer = host.transport.net().is_some()
        && host.transport.local_seat() != robin_engine::player_command::PlayerId::HOST;
    if local_is_peer
        && let Some((clock_frame, ms_until_next_frame)) = drain.latest_host_clock_sample
    {
        timeline.accept_host_frame_schedule(clock_frame, ms_until_next_frame);
    }

    let admission_pause = timeline.multiplayer_admission_paused(now_epoch_ms);
    let mut clock_pause = false;
    if local_is_peer && !admission_pause {
        if let Some(deadline_ms) = timeline.host_frame_deadline_ms() {
            let now_ms = crate::window::process_uptime_ms();
            let until_frame_ms = deadline_ms - i64::from(now_ms);
            if until_frame_ms > 0 {
                clock_pause = true;
                if timeline.clock_ahead_log_due(now_ms) {
                    tracing::info!(
                        scheduled_frame = timeline.host_schedule_frame(),
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
    drain.pause_simulation = admission_pause || host.transport.reconnecting() || clock_pause;

    if host.transport.net().is_some() && (checkpoint_always || drain.rewrote_sim_state) {
        timeline.checkpoint_history(&manager.engine);
    }
    Ok(drain)
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

/// Initialise the multiplayer transport based on `--server` /
/// `--connect` / `--mp-nickname` CLI flags.  Populates
/// [`Host::net`] and [`Host::local_seat`] when active; no-op when
/// neither flag was given.
///
/// On `--server`: starts the listener thread with this process at
/// seat 0 ([`PlayerId::HOST`]).
/// On `--connect`: dials the server, blocks briefly waiting for
/// the assigned-seat handshake, then sets `host.transport.local_seat()` so
/// outgoing inputs are stamped correctly.
///
/// Network failures abort multiplayer startup so the caller can return
/// to the main menu instead of silently launching a different local game.
#[cfg(not(feature = "multiplayer"))]
pub(super) async fn setup_multiplayer_session(
    _host: &mut Host,
    args: &crate::main_entry::MissionLaunch,
    _authoritative_mission_id: &str,
    _authoritative_rng_seed: u64,
    _authoritative_sim_config: robin_engine::engine::SimConfig,
) -> Result<(), String> {
    if args.server || args.connect.is_some() || args.join.is_some() {
        return Err(
            "multiplayer was requested but is unavailable in this build; rebuild with `--features multiplayer`"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(feature = "multiplayer")]
#[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
pub(super) async fn setup_multiplayer_session(
    host: &mut Host,
    args: &crate::main_entry::MissionLaunch,
    authoritative_mission_id: &str,
    authoritative_rng_seed: u64,
    authoritative_sim_config: robin_engine::engine::SimConfig,
    #[cfg(not(target_arch = "wasm32"))] campaign: &crate::multiplayer::MultiplayerCampaignSession,
) -> Result<(), String> {
    use crate::multiplayer::NetChannels;
    #[cfg(not(target_arch = "wasm32"))]
    use crate::multiplayer::NetEvent;
    #[cfg(target_arch = "wasm32")]
    use crate::multiplayer::connect_client;
    #[cfg(not(target_arch = "wasm32"))]
    use crate::multiplayer::{HostedModContent, start_server_in_campaign};
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
            if !args.mp_continue_session {
                campaign.discard_host_continuation()?;
            }
            let publish_browser_links = resolve_browser_join_publication(args)?;
            let speech_timing_locale = host
                .application_context()
                .canonical_speech_timing_locale()
                .map_err(|error| {
                    format!("multiplayer: cannot select authoritative speech timing: {error}")
                })?;
            let (mut channels, in_tx, out_rx, frame_cursor, snapshot_slot) = NetChannels::new();
            let content = args
                .pending_distributed_mod
                .as_ref()
                .map(|encoded| {
                    HostedModContent::from_encoded(encoded.to_vec()).map_err(|error| {
                        format!("multiplayer: invalid hosted full-mod package: {error}")
                    })
                })
                .transpose()?;
            let started = start_server_in_campaign(
                campaign,
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
                content,
                publish_browser_links,
            );
            match started {
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
                            crate::multiplayer::content_identity::active_content_identity(args.global_options.preparation_files()?)
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
                        host.frontend
                            .diagnostics_mut()
                            .queue_console_output(format!(
                                "Browser join code (expires after 30 minutes if unused): {}",
                                ticket.encode()
                            ));
                        host.frontend
                            .diagnostics_mut()
                            .queue_console_output(format!("Browser join link: {share_url}"));
                        host.frontend.diagnostics_mut().queue_console_output(format!(
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
                    let seat = handle.local_seat;
                    channels.attach_runtime(handle);
                    host.transport.install_session(
                        channels,
                        seat,
                        authoritative_mission_id.to_string(),
                        authoritative_rng_seed,
                        authoritative_sim_config,
                        speech_timing_locale,
                    );
                }
                Err(e) => {
                    return Err(format!("multiplayer: failed to start server: {e}"));
                }
            }
        }
    } else if let Some(addr) = args.connect.as_deref() {
        let (mut channels, in_tx, out_rx, _client_frame_cursor, _client_snapshot) =
            NetChannels::new();
        #[cfg(not(target_arch = "wasm32"))]
        let connection = crate::multiplayer::connect_client_in_campaign(
            campaign,
            addr,
            nickname.clone(),
            in_tx,
            out_rx,
        );
        #[cfg(target_arch = "wasm32")]
        let connection = connect_client(addr, nickname.clone(), in_tx, out_rx);
        match connection {
            Ok(handle) => {
                #[cfg(not(target_arch = "wasm32"))]
                let offered_content = handle.content_offer();
                #[cfg(target_arch = "wasm32")]
                let offered_content = {
                    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(15);
                    while handle.content_offer().is_none()
                        && handle.session_metadata().is_none()
                        && web_time::Instant::now() < deadline
                    {
                        crate::window::sleep_ms(10).await;
                    }
                    if handle.content_offer().is_none() && handle.session_metadata().is_none() {
                        return Err(
                            "multiplayer: timed out awaiting browser Welcome/content offer"
                                .to_owned(),
                        );
                    }
                    handle.content_offer()
                };
                if let Err(error) = validate_preflighted_content(
                    args.pending_distributed_mod.as_deref(),
                    offered_content.as_ref(),
                ) {
                    if let Some(offer) = offered_content.as_ref() {
                        channels.reject_content(
                            offer.full_mod_sha256,
                            "host content differs from interactive preflight".to_owned(),
                        );
                    }
                    return Err(error);
                }
                if let Some(offer) = offered_content {
                    let admitted = crate::distributed_mod_admission::admit_trusted_distributed_mod(
                        host.application_context(),
                        &channels,
                        &offer,
                        crate::distributed_mod_admission::DistributedModAdmissionPurpose::JoinSession,
                    )
                    .await
                    .map_err(|error| {
                        format!("multiplayer: host-content admission failed: {error}")
                    })?;
                    host.transport.retain_distributed_mod(admitted);
                    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(15);
                    while handle.session_metadata().is_none() && web_time::Instant::now() < deadline
                    {
                        crate::window::sleep_ms(10).await;
                    }
                    if handle.session_metadata().is_none() {
                        return Err(
                            "multiplayer: timed out awaiting Welcome after verified host-content admission"
                                .to_owned(),
                        );
                    }
                    if offer.mission_basename != authoritative_mission_id {
                        return Err(format!(
                            "multiplayer: offered mod mission `{}` does not match requested mission `{authoritative_mission_id}`",
                            offer.mission_basename
                        ));
                    }
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(10);
                    while handle.session_metadata().is_none() && web_time::Instant::now() < deadline
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
                    if handle.session_metadata().is_none() {
                        return Err(
                            "multiplayer: timed out awaiting authoritative Welcome before Engine construction"
                                .to_string(),
                        );
                    }
                }
                let session = handle.session_metadata().ok_or_else(|| {
                    "multiplayer: authoritative Welcome is not available".to_string()
                })?;
                channels
                    .install_session_id(session.session_id)
                    .map_err(|error| format!("multiplayer: {error}"))?;
                let welcomed_mission = session.mission_id;
                if welcomed_mission != authoritative_mission_id {
                    return Err(format!(
                        "multiplayer: host mission `{welcomed_mission}` does not match requested mission `{authoritative_mission_id}`"
                    ));
                }
                let speech_timing_locale = session.speech_timing_locale;
                if let Some(authoritative_locale) = speech_timing_locale.as_deref() {
                    let has_timing_pack = host
                        .application_context()
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
                tracing::info!(
                    server = %addr,
                    nickname = %nickname,
                    "multiplayer: connected to {addr}"
                );
                // Wait briefly for the AssignedLocalSeat event so
                // host.transport.local_seat() is correct before the mission
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
                                assert_eq!(
                                    seat, session.seat,
                                    "assigned seat differs from admitted Welcome"
                                );
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
                host.transport.install_session(
                    channels,
                    session.seat,
                    welcomed_mission.to_string(),
                    session.mission_seed,
                    session.sim_config,
                    speech_timing_locale,
                );
            }
            Err(e) => {
                return Err(format!("multiplayer: failed to connect to {addr}: {e}"));
            }
        }
    }
    Ok(())
}

#[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
fn resolve_browser_join_publication(
    args: &crate::main_entry::MissionLaunch,
) -> Result<bool, String> {
    let saved = args
        .global_options
        .with_active_profile(|profile| profile.multiplayer_config.publish_browser_join_links)
        .map_err(|error| {
            format!("multiplayer: cannot read browser publication preference: {error}")
        })?;
    Ok(resolve_publication_preference(
        args.mp_browser_join_links,
        saved,
    ))
}

#[cfg(any(test, all(feature = "multiplayer", not(target_arch = "wasm32"))))]
fn resolve_publication_preference(cli_override: Option<bool>, saved: bool) -> bool {
    cli_override.unwrap_or(saved)
}

#[cfg(any(test, feature = "multiplayer"))]
fn validate_multiplayer_launch_args(args: &crate::main_entry::MissionLaunch) -> Result<(), String> {
    if args.server && args.connect.is_some() {
        return Err("multiplayer host and client modes are mutually exclusive".to_string());
    }
    if let Some(expected) = args.mp_expected_players
        && !(1..=crate::multiplayer::MAX_MULTIPLAYER_PLAYERS).contains(&expected)
    {
        return Err(format!(
            "multiplayer expected player count must be between 1 and {}",
            crate::multiplayer::MAX_MULTIPLAYER_PLAYERS
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
    Ok(())
}

#[cfg(any(feature = "multiplayer", test))]
fn validate_preflighted_content(
    expected_bytes: Option<&[u8]>,
    offered: Option<&robin_engine::multiplayer::DistributedModOffer>,
) -> Result<(), String> {
    match (expected_bytes, offered) {
        (Some(expected_bytes), Some(offered)) => {
            let expected = crate::distributed_mod::DistributedModPackage::decode(expected_bytes)
                .map_err(|error| {
                    format!("multiplayer: prepared full-mod package is invalid: {error}")
                })?;
            let expected_offer = crate::distributed_mod::make_distributed_mod_offer(
                &expected,
                expected_bytes.len() as u64,
                offered.host_endpoint_id.clone(),
            )
            .map_err(|error| format!("multiplayer: derive prepared content offer: {error}"))?;
            if offered != &expected_offer {
                return Err(format!(
                    "multiplayer: host content changed after preflight from {} to {}",
                    robin_engine::spellforge::hex_hash(&expected_offer.full_mod_sha256),
                    robin_engine::spellforge::hex_hash(&offered.full_mod_sha256)
                ));
            }
            Ok(())
        }
        (Some(expected_bytes), None) => {
            let expected = crate::distributed_mod::DistributedModPackage::decode(expected_bytes)
                .map_err(|error| {
                    format!("multiplayer: prepared full-mod package is invalid: {error}")
                })?;
            Err(format!(
                "multiplayer: host omitted preflighted content {} on reconnect",
                robin_engine::spellforge::hex_hash(&expected.package.manifest.full_mod_sha256)
            ))
        }
        (None, Some(offered)) => Err(format!(
            "multiplayer: host introduced un-preflighted content {}",
            robin_engine::spellforge::hex_hash(&offered.full_mod_sha256)
        )),
        (None, None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MultiplayerAdmissionEvent, attach_snapshot_spellforge_runtime, drain_net_inputs,
        resolve_publication_preference, rewind_from_recent_timeline_history,
        validate_multiplayer_launch_args, validate_preflighted_content,
    };
    use crate::host::Host;
    use crate::multiplayer::{NetChannels, NetEvent, NetOutbound};
    use crate::rewind::RewindBuffer;
    use robin_engine::campaign::Campaign;
    use robin_engine::engine::{Engine, LevelAssets};
    use robin_engine::engine_manager::EngineManager;
    use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
    use robin_engine::sim_timeline::RestorePolicy;
    use robin_run_protocol::{
        Digest32, LeaderboardCoSignInstanceV1, LeaderboardCoSignPurposeV1,
        LeaderboardCoSignRequestV1,
    };

    fn leaderboard_cosign_request() -> LeaderboardCoSignRequestV1 {
        LeaderboardCoSignRequestV1 {
            instance: LeaderboardCoSignInstanceV1 {
                purpose: LeaderboardCoSignPurposeV1::Submission,
                replay_session_id: Digest32::from_bytes([81; 32]),
                submission_offer_sha256: Digest32::from_bytes([82; 32]),
            },
            run_digest: Digest32::from_bytes([83; 32]),
        }
    }

    use robin_engine::spellforge::{
        SPELLFORGE_CONTRACT_VERSION, SpellforgePackage, SpellforgeRuntime, SpellforgeScriptMode,
    };

    fn spellforge_package(source: &str) -> SpellforgePackage {
        let mut package = SpellforgePackage {
            contract_version: SPELLFORGE_CONTRACT_VERSION,
            vm_abi: robin_spellforge::spellforge_vm_abi().to_owned(),
            script_mode: SpellforgeScriptMode::Replace,
            entrypoint: "mission.lua".to_owned(),
            files: std::collections::BTreeMap::from([(
                "mission.lua".to_owned(),
                source.as_bytes().to_vec(),
            )]),
            sha256: [0; 32],
        };
        package.sha256 = robin_spellforge::compute_package_sha256(&package);
        package
    }

    fn distributed_package_and_offer() -> (Vec<u8>, robin_engine::multiplayer::DistributedModOffer)
    {
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
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
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
        let archive = zip.finish().unwrap().into_inner();
        let package = crate::distributed_mod::DistributedModPackage::build(
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
            archive,
            None,
        )
        .unwrap();
        let encoded = package.package.encode().unwrap();
        let offer = crate::distributed_mod::make_distributed_mod_offer(
            &package,
            encoded.len() as u64,
            "authenticated-host-key".into(),
        )
        .unwrap();
        (encoded, offer)
    }

    fn network_drain_fixture() -> (
        Host,
        EngineManager,
        std::sync::Arc<LevelAssets>,
        std::sync::mpsc::Sender<NetEvent>,
        std::sync::mpsc::Receiver<NetOutbound>,
    ) {
        let mut assets = LevelAssets::new();
        let engine = Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets)
            .expect("fixture engine");
        let manager = EngineManager::new(engine);
        let (channels, incoming, outgoing, _, _) = NetChannels::new();
        let mut host = Host::default();
        host.transport = crate::host::HostTransport::test_session(channels, PlayerId(1));
        (
            host,
            manager,
            std::sync::Arc::new(assets),
            incoming,
            outgoing,
        )
    }

    #[test]
    fn multiplayer_rejects_replay_before_engine_construction() {
        let args = crate::main_entry::MissionLaunch {
            config: crate::main_entry::CliArgs {
                connect: Some("127.0.0.1:7878".to_string()),
                replay: Some("session.rhrec.jsonl".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(validate_multiplayer_launch_args(&args).is_err());

        let multiplayer_only = crate::main_entry::MissionLaunch {
            config: crate::main_entry::CliArgs {
                connect: Some("127.0.0.1:7878".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(validate_multiplayer_launch_args(&multiplayer_only).is_ok());

        let peer_recording = crate::main_entry::MissionLaunch {
            config: crate::main_entry::CliArgs {
                connect: Some("127.0.0.1:7878".to_string()),
                record: Some("peer-canonical-replay.rhrec.jsonl".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(validate_multiplayer_launch_args(&peer_recording).is_ok());
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
        let both = crate::main_entry::MissionLaunch {
            config: crate::main_entry::CliArgs {
                server: true,
                connect: Some("host".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(validate_multiplayer_launch_args(&both).is_err());

        let too_many = crate::main_entry::MissionLaunch {
            config: crate::main_entry::CliArgs {
                server: true,
                mp_expected_players: Some(5),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(validate_multiplayer_launch_args(&too_many).is_err());
    }

    #[test]
    fn lobby_preflight_requires_the_exact_reconnect_offer_without_downgrade() {
        let (encoded, offer) = distributed_package_and_offer();
        assert!(validate_preflighted_content(Some(&encoded), Some(&offer)).is_ok());
        assert!(validate_preflighted_content(None, None).is_ok());
        assert!(validate_preflighted_content(Some(&encoded), None).is_err());
        assert!(validate_preflighted_content(None, Some(&offer)).is_err());

        let mut metadata_changed = offer.clone();
        metadata_changed.title.push_str(" impersonated");
        assert!(
            validate_preflighted_content(Some(&encoded), Some(&metadata_changed)).is_err(),
            "matching package hashes do not excuse changed consent metadata"
        );
    }

    #[test]
    fn host_snapshot_supplies_exact_spellforge_runtime_to_peer() {
        let package = spellforge_package("function Initialize() return 0 end");
        let runtime: std::sync::Arc<dyn SpellforgeRuntime> = std::sync::Arc::new(
            robin_spellforge::SpellforgeRuntime51::new(package.clone()).unwrap(),
        );
        let mut host_assets = LevelAssets::new();
        host_assets.attachments.spellforge_runtime = Some(runtime);
        let snapshot = Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut host_assets)
            .expect("Spellforge host snapshot");

        let mut peer_assets = std::sync::Arc::new(LevelAssets::new());
        attach_snapshot_spellforge_runtime(&snapshot, &mut peer_assets, || Ok(true)).unwrap();

        let attached = peer_assets
            .attachments
            .spellforge_runtime
            .as_ref()
            .expect("peer runtime attached");
        assert_eq!(attached.package(), &package);
        let adopted = Engine::adopt_authoritative_snapshot(snapshot, peer_assets.as_ref())
            .expect("snapshot package matches the peer runtime reconstructed from it");
        assert_eq!(adopted.spellforge_package().as_deref(), Some(&package));
    }

    #[test]
    fn peer_gameplay_opt_out_rejects_spellforge_snapshot() {
        let package = spellforge_package("function Initialize() return 0 end");
        let runtime: std::sync::Arc<dyn SpellforgeRuntime> = std::sync::Arc::new(
            robin_spellforge::SpellforgeRuntime51::new(package.clone()).unwrap(),
        );
        let mut host_assets = LevelAssets::new();
        host_assets.attachments.spellforge_runtime = Some(runtime);
        let snapshot = Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut host_assets)
            .expect("Spellforge host snapshot");

        let mut peer_assets = std::sync::Arc::new(LevelAssets::new());
        let error = attach_snapshot_spellforge_runtime(&snapshot, &mut peer_assets, || Ok(false))
            .expect_err("disabled Spellforge must reject host package");
        assert!(error.contains("disabled in Gameplay settings"));
        assert!(peer_assets.attachments.spellforge_runtime.is_none());
    }

    #[test]
    fn snapshot_is_adopted_before_ready_is_announced() {
        let (mut host, mut manager, mut assets, incoming, outgoing) = network_drain_fixture();
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
        let mut pending = super::super::runtime::reconciliation::NetworkReconciliation::default();
        let drain = drain_net_inputs(
            &mut host,
            &mut manager,
            0,
            &mut pending,
            &mut assets,
            &mut rewind,
        )
        .expect("network drain succeeds");

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
        let (mut host, mut manager, mut assets, incoming, outgoing) = network_drain_fixture();
        let engine_bytes = manager.engine.encode_native_snapshot();
        incoming
            .send(NetEvent::InitialSnapshot {
                frame: 32,
                engine_bytes,
            })
            .expect("queue mid-mission snapshot");
        let mut rewind = RewindBuffer::new();
        let mut pending = super::super::runtime::reconciliation::NetworkReconciliation::default();

        let drain = drain_net_inputs(
            &mut host,
            &mut manager,
            31,
            &mut pending,
            &mut assets,
            &mut rewind,
        )
        .expect("network drain succeeds");
        assert_eq!(drain.adopted_frame, Some(32));
        assert!(matches!(
            outgoing.recv().expect("ReadyToSim after adoption"),
            NetOutbound::ReadyToSim { frame: 32 }
        ));

        rewind.begin_frame(32, &manager.engine);
        rewind.end_frame_input(robin_engine::engine::SimulationFrameInput::default());
        assert!(rewind.frame_for(32).is_some());
        assert_eq!(rewind.oldest_cmd_frame(), 32);
    }

    #[test]
    fn reconnect_adopts_older_host_snapshot_and_discards_prediction_future() {
        let (mut host, mut manager, mut assets, incoming, outgoing) = network_drain_fixture();
        host.transport.await_authoritative_snapshot();
        let engine_bytes = manager.engine.encode_native_snapshot();
        incoming
            .send(NetEvent::InitialSnapshot {
                frame: 30,
                engine_bytes,
            })
            .expect("queue reconnect snapshot");
        let mut rewind = RewindBuffer::new();
        let mut pending = super::super::runtime::reconciliation::NetworkReconciliation::default();
        pending.admit_remote_hash(36, 0x0BAD_5EED);
        pending.queue_input(
            super::TimelineFrame::from_wire(36),
            PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown),
        );

        let drain = drain_net_inputs(
            &mut host,
            &mut manager,
            35,
            &mut pending,
            &mut assets,
            &mut rewind,
        )
        .expect("network drain succeeds");

        assert_eq!(drain.adopted_frame, Some(30));
        assert!(drain.rewrote_sim_state);
        assert!(
            host.transport.reconnecting(),
            "controls stay disabled until the ready barrier releases"
        );
        assert!(
            pending.pending_frame_count() == 0,
            "old predicted inputs must not cross sessions"
        );
        assert!(
            pending
                .take_due_comparisons(robin_engine::replay::TimelineFrame::from_wire(100))
                .is_empty(),
            "old peer hashes must not cross sessions"
        );
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
            &mut assets,
            &mut rewind,
        )
        .expect("network drain succeeds");
        assert!(!host.transport.reconnecting());
    }

    #[test]
    fn closed_worker_fails_the_mission_drain_instead_of_waiting_for_reconnect() {
        let (mut host, mut manager, mut assets, incoming, _outgoing) = network_drain_fixture();
        drop(incoming);
        let error = drain_net_inputs(
            &mut host,
            &mut manager,
            0,
            &mut super::super::runtime::reconciliation::NetworkReconciliation::default(),
            &mut assets,
            &mut RewindBuffer::new(),
        )
        .err()
        .expect("remote fault must fail the drain");
        assert!(
            error
                .to_string()
                .contains("transport worker closed its event channel")
        );
        assert!(matches!(error, MultiplayerSessionError::Protocol(_)));
    }

    #[test]
    fn fatal_transport_event_fails_the_mission_drain_loudly() {
        let (mut host, mut manager, mut assets, incoming, _outgoing) = network_drain_fixture();
        incoming
            .send(NetEvent::Fatal("test transport failure".into()))
            .expect("queue fatal event");
        let mut rewind = RewindBuffer::new();
        let mut pending = super::super::runtime::reconciliation::NetworkReconciliation::default();
        let error = drain_net_inputs(
            &mut host,
            &mut manager,
            0,
            &mut pending,
            &mut assets,
            &mut rewind,
        )
        .err()
        .expect("remote fault must fail the drain");
        assert!(error.to_string().contains("test transport failure"));
        assert!(matches!(error, MultiplayerSessionError::Protocol(_)));
    }

    #[test]
    fn malformed_remote_snapshots_fail_without_changing_the_engine() {
        for frame in [0, 1] {
            let (mut host, mut manager, mut assets, incoming, _outgoing) = network_drain_fixture();
            let before = manager.engine.encode_native_snapshot();
            incoming
                .send(NetEvent::InitialSnapshot {
                    frame,
                    engine_bytes: vec![0, 1, 2],
                })
                .unwrap();
            let result = drain_net_inputs(
                &mut host,
                &mut manager,
                0,
                &mut super::super::runtime::reconciliation::NetworkReconciliation::default(),
                &mut assets,
                &mut RewindBuffer::new(),
            );
            assert!(matches!(result, Err(MultiplayerSessionError::Protocol(_))));
            assert_eq!(before, manager.engine.encode_native_snapshot());
        }
    }

    #[test]
    fn mission_drain_routes_cosign_to_dedicated_inbox_without_requeue_spin() {
        let (mut host, mut manager, mut assets, incoming, _outgoing) = network_drain_fixture();
        let request = leaderboard_cosign_request();
        incoming
            .send(NetEvent::LeaderboardCoSignRequest(request))
            .unwrap();
        let mut rewind = RewindBuffer::new();
        let mut pending = super::super::runtime::reconciliation::NetworkReconciliation::default();
        let _ = drain_net_inputs(
            &mut host,
            &mut manager,
            0,
            &mut pending,
            &mut assets,
            &mut rewind,
        )
        .expect("network drain succeeds");

        let net = host.transport.net().unwrap();
        assert!(matches!(
            net.try_recv_leaderboard_cosign_event().unwrap(),
            Some(NetEvent::LeaderboardCoSignRequest(decoded)) if decoded == request
        ));
        assert!(net.try_recv_leaderboard_cosign_event().unwrap().is_none());
    }

    #[test]
    fn mission_drain_routes_ranked_context_to_authorization_inbox() {
        let (mut host, mut manager, mut assets, incoming, _outgoing) = network_drain_fixture();
        let context = robin_engine::multiplayer::RankedCoSignContextDocument::new(
            br#"{"kind":"submission"}"#.to_vec(),
        )
        .unwrap();
        incoming
            .send(NetEvent::RankedCoSignContext(context.clone()))
            .unwrap();
        let mut rewind = RewindBuffer::new();
        let mut pending = super::super::runtime::reconciliation::NetworkReconciliation::default();
        let _ = drain_net_inputs(
            &mut host,
            &mut manager,
            0,
            &mut pending,
            &mut assets,
            &mut rewind,
        )
        .expect("network drain succeeds");

        let net = host.transport.net().unwrap();
        assert!(matches!(
            net.try_recv_leaderboard_cosign_event().unwrap(),
            Some(NetEvent::RankedCoSignContext(decoded)) if decoded == context
        ));
        assert!(net.try_recv_leaderboard_cosign_event().unwrap().is_none());
    }

    #[test]
    #[should_panic(expected = "leaderboard co-sign inbox exceeds its 64-event limit")]
    fn mission_drain_fails_closed_on_cosign_inbox_overflow() {
        let (mut host, mut manager, mut assets, incoming, _outgoing) = network_drain_fixture();
        let request = leaderboard_cosign_request();
        for _ in 0..=robin_engine::multiplayer::MAX_LEADERBOARD_COSIGN_INBOX_EVENTS {
            incoming
                .send(NetEvent::LeaderboardCoSignRequest(request))
                .unwrap();
        }
        let mut rewind = RewindBuffer::new();
        let mut pending = super::super::runtime::reconciliation::NetworkReconciliation::default();
        let _ = drain_net_inputs(
            &mut host,
            &mut manager,
            0,
            &mut pending,
            &mut assets,
            &mut rewind,
        )
        .expect("network drain succeeds");
    }

    fn rewind_with_horizon(manager: &EngineManager, start: u32, end: u32) -> RewindBuffer {
        let mut rewind = RewindBuffer::new();
        for frame in start..end {
            rewind.begin_frame(frame, &manager.engine);
            rewind.end_frame_input(robin_engine::engine::SimulationFrameInput::default());
        }
        assert_eq!(rewind.oldest_cmd_frame(), start);
        rewind
    }

    #[test]
    fn client_too_old_input_requests_a_complete_snapshot_reconnect() {
        let (mut host, mut manager, mut assets, incoming, outgoing) = network_drain_fixture();
        let mut rewind = rewind_with_horizon(&manager, 25, 35);
        incoming
            .send(NetEvent::Input {
                server_frame: 35,
                origin_frame: 23,
                target_frame: 24,
                input: PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown),
            })
            .expect("queue stale input");
        let mut pending = super::super::runtime::reconciliation::NetworkReconciliation::default();

        let drain = drain_net_inputs(
            &mut host,
            &mut manager,
            35,
            &mut pending,
            &mut assets,
            &mut rewind,
        )
        .expect("network drain succeeds");

        assert!(host.transport.reconnecting());
        assert!(pending.pending_frame_count() == 0);
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
        let (mut host, mut manager, mut assets, incoming, outgoing) = network_drain_fixture();
        host.transport.test_local_seat(PlayerId::HOST);
        let mut rewind = rewind_with_horizon(&manager, 25, 35);
        incoming
            .send(NetEvent::Input {
                server_frame: 35,
                origin_frame: 23,
                target_frame: 24,
                input: PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown),
            })
            .expect("queue stale peer input");
        let mut pending = super::super::runtime::reconciliation::NetworkReconciliation::default();

        let drain = drain_net_inputs(
            &mut host,
            &mut manager,
            35,
            &mut pending,
            &mut assets,
            &mut rewind,
        )
        .expect("network drain succeeds");

        assert!(host.transport.reconnecting());
        assert_eq!(
            drain.admission_events,
            [MultiplayerAdmissionEvent::HostResynchronizing { frame: 35 }]
        );
        assert!(drain.inputs.is_empty());
        assert!(matches!(
            outgoing.try_recv().expect("reconnect-all request"),
            NetOutbound::ReconnectAllForSnapshot { reason }
                if reason.contains("rollback horizon")
        ));
        assert!(matches!(
            outgoing.try_recv().expect("replacement host readiness"),
            NetOutbound::ReadyToSim { frame: 35 }
        ));
        incoming
            .send(NetEvent::Input {
                server_frame: 35,
                origin_frame: 23,
                target_frame: 24,
                input: PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown),
            })
            .expect("second obsolete input queued before disconnect");
        let repeated = drain_net_inputs(
            &mut host,
            &mut manager,
            35,
            &mut pending,
            &mut assets,
            &mut rewind,
        )
        .expect("network drain succeeds");
        assert!(repeated.admission_events.is_empty());
        assert!(
            outgoing.try_recv().is_err(),
            "already pending replacement must not reset again"
        );
        let published_hash = robin_engine::replay::state_hash(&manager.engine);
        for target_frame in [30, 35, 40] {
            incoming
                .send(NetEvent::Input {
                    server_frame: 35,
                    origin_frame: target_frame,
                    target_frame,
                    input: PlayerInput::new(PlayerId(2), PlayerCommand::CrouchDown),
                })
                .expect("in-flight viable input during replacement");
        }
        let frozen = drain_net_inputs(
            &mut host,
            &mut manager,
            35,
            &mut pending,
            &mut assets,
            &mut rewind,
        )
        .expect("network drain succeeds");
        assert!(frozen.inputs.is_empty());
        assert!(frozen.rollback.is_none());
        assert!(pending.pending_frame_count() == 0);
        assert_eq!(
            robin_engine::replay::state_hash(&manager.engine),
            published_hash
        );
    }

    #[test]
    fn failed_recent_history_rebuild_does_not_publish_partial_checkpoints() {
        let (_host, manager, assets, _incoming, _outgoing) = network_drain_fixture();
        let mut rewind = RewindBuffer::new();
        for frame in 0..2 {
            rewind.begin_frame(frame, &manager.engine);
            rewind.end_frame_input(robin_engine::engine::SimulationFrameInput::default());
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
