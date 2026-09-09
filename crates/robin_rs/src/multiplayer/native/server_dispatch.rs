//! Authoritative host command dispatch. The parent owns socket lifetimes,
//! authenticated seats and admission; this module owns game-loop fan-out.
//!
//! Recoverable sends rely on the existing snapshot/reconnect protocol. Required
//! control sends cannot be dropped without splitting authoritative decisions.

use super::*;

/// Delivery is a protocol decision, not a property of the socket API.
/// Recoverable state is replayed from the host cache on reconnect; required
/// controls cannot be reconstructed by that protocol. Diagnostics never gate
/// deterministic progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Delivery {
    Required,
    ReconnectRecoverable,
    Diagnostic,
}

impl Delivery {
    fn for_message(message: &NetMsg) -> Self {
        match message {
            NetMsg::StateHash { .. } | NetMsg::Note(_) => Self::Diagnostic,
            NetMsg::InitialSnapshot { .. }
            | NetMsg::BeginSim { .. }
            | NetMsg::BroadcastInput { .. } => Self::ReconnectRecoverable,
            // The irreversible downgrade is retained in ranked_browse_reason
            // and replayed by prepare_peer_session before cached BeginSim.
            NetMsg::RankedBrowseOnly { .. } => Self::ReconnectRecoverable,
            // New control messages default to required until their owner
            // explicitly establishes a recovery protocol.
            _ => Self::Required,
        }
    }
}

fn queue_peer_message(
    sender: &UnboundedSender<NetMsg>,
    message: NetMsg,
    delivery: Delivery,
) -> Result<(), String> {
    if sender.send(message).is_ok() {
        return Ok(());
    }
    match delivery {
        Delivery::Required => Err("authoritative multiplayer writer queue is closed".into()),
        Delivery::ReconnectRecoverable => {
            // The writer is owned by drive_server_peer_io, which races it
            // against the reader and releases this generation even if the
            // reader remains open. Recovery does not depend on reader EOF.
            tracing::warn!(
                "multiplayer writer closed; peer teardown will require snapshot recovery"
            );
            Ok(())
        }
        Delivery::Diagnostic => {
            tracing::debug!("multiplayer diagnostic skipped for closed peer writer");
            Ok(())
        }
    }
}

/// Take locally-produced messages from the game loop, stamp them with
/// seat 0 + a target frame, fan them out to every peer's writer
/// queue, AND echo them back into `incoming_tx` so the local game
/// loop applies them in the same input order every other machine
/// does.  Target frame = current sim frame + [`INPUT_DELAY_FRAMES`]
/// so peers (which receive the broadcast over the wire with some
/// latency) still have time to apply at the matching frame; if a peer
/// is already past the target, the rollback path picks up the slack.
pub(super) async fn run_server_outgoing_pump(
    context: Arc<ServerContext>,
    mut outgoing_async_rx: UnboundedReceiver<NetOutbound>,
) -> Result<(), String> {
    while let Some(msg) = outgoing_async_rx.recv().await {
        let _authority = context.session_dispatch.lock();
        validate_server_gameplay_outbound(&msg)?;
        match msg {
            NetOutbound::Input {
                origin_frame,
                command,
            } => {
                let now = context.frame_cursor.load(Ordering::Relaxed);
                let target = now.max(origin_frame).saturating_add(INPUT_DELAY_FRAMES);
                let inp = PlayerInput::new(PlayerId::HOST, command);
                broadcast_input(&context, now, origin_frame, target, inp);
            }
            NetOutbound::StateHash {
                frame,
                hash,
                clock_frame,
                ms_until_next_frame,
            } => {
                // Authoritative-host state hash: broadcast as a wire
                // `StateHash` to every peer.  No echo into our own
                // incoming channel — the local game loop already has
                // the value (it just computed the hash before pushing
                // here).
                broadcast_diagnostic(
                    &context,
                    NetMsg::StateHash {
                        frame,
                        hash,
                        clock_frame,
                        ms_until_next_frame,
                    },
                );
            }
            NetOutbound::InitialSnapshot {
                frame,
                engine_bytes,
            } => {
                // A peer can complete the handshake before mission
                // setup has produced the frame-0 snapshot.  Push the
                // snapshot to all currently-connected peers as soon
                // as it exists; later peers still receive it through
                // the handshake cache.
                broadcast_recoverable(
                    &context,
                    NetMsg::InitialSnapshot {
                        frame,
                        engine_bytes,
                    },
                );
            }
            NetOutbound::ReadyToSim { frame } => {
                resolve_ranked_before_ready(&context);
                let begin = {
                    let mut p = context.peers.lock();
                    p.readiness.host_frame = Some(frame);
                    maybe_begin_sim_locked(&mut p)
                }?;
                announce_begin_sim(&context, begin);
            }
            NetOutbound::ModalProposal { .. } => {
                tracing::error!("multiplayer host attempted to send a client-only modal proposal");
            }
            NetOutbound::ModalDecision {
                instance,
                kind,
                result,
                decision_frame,
            } => {
                if instance.session_id != context.session_id {
                    tracing::error!(
                        ?instance,
                        "multiplayer host rejected a modal decision for another session"
                    );
                    continue;
                }
                if let Err(error) = broadcast_msg_required(
                    &context,
                    NetMsg::ModalDecision {
                        instance,
                        kind,
                        result,
                        decision_frame,
                    },
                ) {
                    tracing::error!(%error, "authoritative modal broadcast failed");
                    let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                }
            }
            NetOutbound::ReconnectForSnapshot { player_id, reason } => {
                assert_ne!(
                    player_id,
                    PlayerId::HOST,
                    "authoritative host cannot reconnect itself for a stale input"
                );
                let sender = context.peers.lock().take_sender(&player_id.0);
                if let Some(sender) = sender {
                    tracing::warn!(
                        ?player_id,
                        %reason,
                        "multiplayer: dropping peer for full-snapshot resynchronization"
                    );
                    // Tell the peer why this otherwise-graceful stream close
                    // requires reconnecting. The queue drains this message
                    // before observing that its last sender was dropped.
                    let _ = sender.send(NetMsg::ReconnectRequired {
                        reason: reason.clone(),
                    });
                    drop(sender);
                } else {
                    tracing::warn!(
                        ?player_id,
                        %reason,
                        "multiplayer: stale-input peer was already disconnected"
                    );
                }
            }
            NetOutbound::ReconnectAllForSnapshot { reason } => {
                let senders = {
                    let mut peers = context.peers.lock();
                    peers.readiness.reset();
                    peers.clear_ready();
                    peers.take_senders()
                };
                tracing::warn!(
                    peers = senders.len(),
                    %reason,
                    "multiplayer: dropping every peer for full-snapshot resynchronization"
                );
                for sender in &senders {
                    let _ = sender.send(NetMsg::ReconnectRequired {
                        reason: reason.clone(),
                    });
                }
                drop(senders);
            }
            NetOutbound::BeginSnapshotTransition { id, payload } => {
                assert_eq!(
                    id.session_id, context.session_id,
                    "host snapshot transition belongs to another multiplayer session"
                );
                let committed = {
                    let mut peers = context.peers.lock();
                    assert!(
                        peers.transitions.pending().is_none(),
                        "another multiplayer snapshot transition is already pending"
                    );
                    let awaiting = peers
                        .senders()
                        .map(|(seat, _)| seat)
                        .copied()
                        .collect::<HashSet<_>>();
                    peers.transitions.begin(PendingSnapshotTransition {
                        id,
                        payload: payload.clone(),
                        awaiting,
                    });
                    let prepare = NetMsg::PrepareSnapshotTransition { id, payload };
                    // Keep the peer-state lock until every current writer has
                    // queued Prepare. Otherwise its reader could disconnect,
                    // empty the readiness set, and queue Commit first.
                    for sender in peers.senders().map(|(_, sender)| sender) {
                        queue_peer_message(sender, prepare.clone(), Delivery::Required)?;
                    }
                    take_committed_snapshot_transition(&mut peers)
                };
                commit_snapshot_transition(&context, committed);
            }
            NetOutbound::SnapshotTransitionReady { .. } => {
                tracing::error!(
                    "multiplayer host attempted to acknowledge its own snapshot transition"
                );
            }
            NetOutbound::RankedBrowseOnly { reason } => {
                downgrade_ranked_session(
                    &context,
                    reason,
                    "host runtime explicitly resolved this multiplayer session as browse-only",
                );
            }
            NetOutbound::RankedOfficialSessionSetup(setup) => {
                if let Err(error) =
                    crate::leaderboard_ranked_session::decode_canonical_ranked_wire_document::<
                        OfficialRankedSessionWireSetupV1,
                    >(setup.as_bytes())
                {
                    let error =
                        format!("host rejected invalid official ranked wire setup: {error}");
                    tracing::error!(%error);
                    let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                    continue;
                }
                if let Err(error) =
                    broadcast_msg_required(&context, NetMsg::RankedOfficialSessionSetup(setup))
                {
                    tracing::error!(%error, "official ranked setup broadcast failed");
                    let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                }
            }
            NetOutbound::RankedContinuationReceiptSelectionRequest(request) => {
                if let Err(error) = decode_ranked_wire_document::<
                    CampaignContinuationReceiptSelectionRequestV1,
                >(request.as_bytes())
                {
                    let error = format!(
                        "host rejected invalid continuation receipt selection request: {error}"
                    );
                    tracing::error!(%error);
                    let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                    continue;
                }
                if let Err(error) = broadcast_msg_required(
                    &context,
                    NetMsg::RankedContinuationReceiptSelectionRequest(request),
                ) {
                    tracing::error!(%error, "continuation receipt selection broadcast failed");
                    let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                }
            }
            NetOutbound::RankedContinuationReceiptSelection(_) => {
                let error = "multiplayer host attempted to send a client-only continuation receipt selection".to_string();
                tracing::error!(%error);
                let _ = context.incoming_tx.send(NetEvent::Fatal(error));
            }
            NetOutbound::RankedContinuationPreflightClaim { to, claim } => {
                let decoded = decode_ranked_wire_document::<
                    CampaignContinuationPreflightRequestClaimV1,
                >(claim.as_bytes());
                let claim_document = match decoded {
                    Ok(document) => document,
                    Err(error) => {
                        let error = format!(
                            "host rejected invalid continuation preflight claim for {to:?}: {error}"
                        );
                        tracing::error!(%error);
                        let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                        continue;
                    }
                };
                let sender = {
                    let peers = context.peers.lock();
                    let expected_controller = peers
                        .ranked_identity(&to.0)
                        .and_then(|identity| identity.durable_public_key)
                        .map(PublicKey32::from_bytes);
                    if to == PlayerId::HOST
                        || expected_controller
                            != Some(claim_document.campaign_controller_public_key)
                    {
                        None
                    } else {
                        peers.sender(&to.0).cloned()
                    }
                };
                match sender {
                    Some(sender) => {
                        if sender
                            .send(NetMsg::RankedContinuationPreflightClaim(claim))
                            .is_err()
                        {
                            let error = format!(
                                "continuation preflight controller {to:?} disconnected before claim delivery"
                            );
                            tracing::error!(%error);
                            let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                        }
                    }
                    None => {
                        let error = format!(
                            "continuation preflight target {to:?} is not the authenticated controller"
                        );
                        tracing::error!(%error);
                        let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                    }
                }
            }
            NetOutbound::RankedContinuationPreflightSignature(_) => {
                let error = "multiplayer host attempted to send a client-only continuation preflight signature".to_string();
                tracing::error!(%error);
                let _ = context.incoming_tx.send(NetEvent::Fatal(error));
            }
            NetOutbound::RankedCoSignContext {
                to,
                context: document,
            } => {
                if ranked_lifecycle_lock(&context.ranked_lifecycle)
                    .ranked_session()
                    .is_none()
                {
                    tracing::warn!(
                        ?to,
                        "ignored ranked co-sign context after eligibility ended"
                    );
                    continue;
                }
                if let Err(error) = decode_ranked_wire_document::<
                    crate::leaderboard_ranked_session::RankedCoSignContextV1,
                >(document.as_bytes())
                {
                    tracing::error!(%error, ?to, "rejected invalid ranked co-sign context");
                    continue;
                }
                let sender = {
                    let peers = context.peers.lock();
                    peers
                        .is_sim_connected(&to.0)
                        .then(|| peers.sender(&to.0).cloned())
                        .flatten()
                };
                match sender {
                    Some(sender) => {
                        if sender.send(NetMsg::RankedCoSignContext(document)).is_err() {
                            tracing::warn!(?to, "ranked co-sign context target disconnected");
                        }
                    }
                    None => tracing::warn!(?to, "ranked co-sign context target is not admitted"),
                }
            }
            NetOutbound::RankedSubmissionAccepted { to, accepted } => {
                if ranked_lifecycle_lock(&context.ranked_lifecycle)
                    .ranked_session()
                    .is_none()
                {
                    tracing::warn!(
                        ?to,
                        "ignored ranked submission acknowledgement after eligibility ended"
                    );
                    continue;
                }
                if let Err(error) = decode_ranked_wire_document::<
                    robin_run_protocol::SubmissionAcceptedV1,
                >(accepted.as_bytes())
                {
                    tracing::error!(%error, ?to, "rejected invalid ranked submission acknowledgement");
                    continue;
                }
                let sender = {
                    let peers = context.peers.lock();
                    peers
                        .is_sim_connected(&to.0)
                        .then(|| peers.sender(&to.0).cloned())
                        .flatten()
                };
                match sender {
                    Some(sender) => {
                        if sender
                            .send(NetMsg::RankedSubmissionAccepted(accepted))
                            .is_err()
                        {
                            tracing::warn!(
                                ?to,
                                "ranked submission acknowledgement target disconnected"
                            );
                        }
                    }
                    None => tracing::warn!(
                        ?to,
                        "ranked submission acknowledgement target is not admitted"
                    ),
                }
            }
            NetOutbound::RankedJoinChallenge { .. }
            | NetOutbound::RankedJoinAccepted { .. }
            | NetOutbound::RankedParticipantRoster { .. }
            | NetOutbound::ArmRankedJoin { .. }
            | NetOutbound::RankedJoinResponse(_) => {
                tracing::error!(
                    "native host ignored an externally-authored ranked admission control"
                );
            }
            NetOutbound::LeaderboardCoSignRequest { to, request } => {
                if ranked_lifecycle_lock(&context.ranked_lifecycle)
                    .ranked_session()
                    .is_none()
                {
                    tracing::warn!(
                        ?to,
                        "ignored leaderboard co-sign request after ranked eligibility ended"
                    );
                    continue;
                }
                let sender = {
                    let mut peers = context.peers.lock();
                    peers.begin_leaderboard_cosign(to, request)
                };
                match sender {
                    Ok(sender) => {
                        if sender
                            .send(NetMsg::LeaderboardCoSignRequest(request))
                            .is_err()
                        {
                            let error = format!(
                                "authenticated leaderboard co-sign target {to:?} closed before request delivery"
                            );
                            tracing::error!(%error);
                            let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "leaderboard co-sign request rejected");
                        let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                    }
                }
            }
            NetOutbound::ArmLeaderboardCoSignRequest { .. } => {
                let error =
                    "multiplayer host attempted to arm a client-only leaderboard co-sign request"
                        .to_string();
                tracing::error!(%error);
                let _ = context.incoming_tx.send(NetEvent::Fatal(error));
            }
            NetOutbound::LeaderboardCoSignResponse(_) => {
                let error =
                    "multiplayer host attempted to send a client-only leaderboard co-sign response"
                        .to_string();
                tracing::error!(%error);
                let _ = context.incoming_tx.send(NetEvent::Fatal(error));
            }
            NetOutbound::ContentRequest { .. }
            | NetOutbound::ContentReject { .. }
            | NetOutbound::ContentReady { .. }
            | NetOutbound::ContentPrepared { .. } => {
                unreachable!("server gameplay outbound was validated before dispatch")
            }
        }
    }
    tracing::info!("server outgoing pump stopped");
    Ok(())
}

pub(super) fn validate_server_gameplay_outbound(outgoing: &NetOutbound) -> Result<(), String> {
    match outgoing {
        NetOutbound::Input { .. }
        | NetOutbound::StateHash { .. }
        | NetOutbound::InitialSnapshot { .. }
        | NetOutbound::ReadyToSim { .. }
        | NetOutbound::ModalDecision { .. }
        | NetOutbound::ReconnectForSnapshot { .. }
        | NetOutbound::ReconnectAllForSnapshot { .. }
        | NetOutbound::BeginSnapshotTransition { .. }
        | NetOutbound::RankedBrowseOnly { .. }
        | NetOutbound::RankedOfficialSessionSetup(_)
        | NetOutbound::RankedContinuationReceiptSelectionRequest(_)
        | NetOutbound::RankedContinuationPreflightClaim { .. }
        | NetOutbound::RankedCoSignContext { .. }
        | NetOutbound::RankedSubmissionAccepted { .. }
        | NetOutbound::RankedJoinChallenge { .. }
        | NetOutbound::RankedJoinAccepted { .. }
        | NetOutbound::RankedParticipantRoster { .. }
        | NetOutbound::LeaderboardCoSignRequest { .. } => Ok(()),
        NetOutbound::ModalProposal { .. }
        | NetOutbound::SnapshotTransitionReady { .. }
        | NetOutbound::ContentRequest { .. }
        | NetOutbound::ContentReject { .. }
        | NetOutbound::ContentReady { .. }
        | NetOutbound::ContentPrepared { .. }
        | NetOutbound::RankedContinuationReceiptSelection(_)
        | NetOutbound::RankedContinuationPreflightSignature(_)
        | NetOutbound::ArmRankedJoin { .. }
        | NetOutbound::RankedJoinResponse(_)
        | NetOutbound::ArmLeaderboardCoSignRequest { .. }
        | NetOutbound::LeaderboardCoSignResponse(_) => {
            Err("multiplayer host queued a client-only output".to_owned())
        }
    }
}

pub(super) fn announce_begin_sim(
    context: &ServerContext,
    begin: Option<(u32, u64, Vec<UnboundedSender<NetMsg>>)>,
) {
    if let Some((begin_frame, start_epoch_ms, senders)) = begin {
        tracing::info!(
            frame = begin_frame,
            start_epoch_ms,
            "multiplayer: ready barrier complete"
        );
        let _ = context.incoming_tx.send(NetEvent::BeginSim {
            frame: begin_frame,
            start_epoch_ms,
        });
        for sender in senders {
            queue_peer_message(
                &sender,
                NetMsg::BeginSim {
                    frame: begin_frame,
                    start_epoch_ms,
                },
                Delivery::ReconnectRecoverable,
            )
            .expect("recoverable delivery cannot fail the session");
        }
    }
}

/// Send one message to every connected peer's writer queue.
pub(super) fn broadcast_recoverable(context: &ServerContext, msg: NetMsg) {
    broadcast_with_delivery(context, msg, Delivery::ReconnectRecoverable)
        .expect("recoverable delivery cannot fail the session");
}

fn broadcast_diagnostic(context: &ServerContext, msg: NetMsg) {
    broadcast_with_delivery(context, msg, Delivery::Diagnostic)
        .expect("diagnostic delivery cannot fail the session");
}

fn broadcast_with_delivery(
    context: &ServerContext,
    msg: NetMsg,
    delivery: Delivery,
) -> Result<(), String> {
    assert_eq!(
        Delivery::for_message(&msg),
        delivery,
        "incorrect multiplayer delivery policy"
    );
    let to_send: Vec<UnboundedSender<NetMsg>> = {
        let p = context.peers.lock();
        p.senders().map(|(_, sender)| sender).cloned().collect()
    };
    for sender in to_send {
        queue_peer_message(&sender, msg.clone(), delivery)?;
    }
    Ok(())
}

/// Queue an authoritative message for every currently connected peer. A
/// closed writer queue is a fatal session split, not a best-effort diagnostic.
pub(super) fn broadcast_msg_required(context: &ServerContext, msg: NetMsg) -> Result<(), String> {
    assert_eq!(
        Delivery::for_message(&msg),
        Delivery::Required,
        "required broadcast must contain an authoritative control"
    );
    let to_send: Vec<(u8, UnboundedSender<NetMsg>)> = {
        let peers = context.peers.lock();
        peers
            .senders()
            .map(|(seat, sender)| (*seat, sender.clone()))
            .collect()
    };
    for (seat, sender) in to_send {
        queue_peer_message(&sender, msg.clone(), Delivery::Required).map_err(|_| {
            format!("authoritative multiplayer send queue for seat {seat} is closed")
        })?;
    }
    Ok(())
}

/// Send a [`NetMsg::BroadcastInput`] to every peer plus echo it into
/// the local game-loop event stream.  A send failure just means that
/// peer's writer task ended; the peer I/O driver releases its generation
/// without waiting for the reader and emits `DisconnectSeat` on the way out.
pub(super) fn broadcast_input(
    context: &ServerContext,
    server_frame: u32,
    origin_frame: u32,
    target_frame: u32,
    inp: PlayerInput,
) {
    // Local fan-in: feed the input back into our own game loop.
    let _ = context.incoming_tx.send(NetEvent::Input {
        server_frame,
        origin_frame,
        target_frame,
        input: inp.clone(),
    });

    broadcast_recoverable(
        context,
        NetMsg::BroadcastInput {
            server_frame,
            origin_frame,
            target_frame,
            input: inp,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_snapshot_reconstructible_messages_are_reconnect_recoverable() {
        assert_eq!(
            Delivery::for_message(&NetMsg::BeginSim {
                frame: 2,
                start_epoch_ms: 3
            }),
            Delivery::ReconnectRecoverable
        );
        assert_eq!(
            Delivery::for_message(&NetMsg::InitialSnapshot {
                frame: 2,
                engine_bytes: vec![1]
            }),
            Delivery::ReconnectRecoverable
        );
        assert_eq!(
            Delivery::for_message(&NetMsg::Note("diagnostic".into())),
            Delivery::Diagnostic
        );
        assert_eq!(
            Delivery::for_message(&NetMsg::RankedBrowseOnly {
                reason: RankedBrowseOnlyReason::RankedProtocolViolation,
            }),
            Delivery::ReconnectRecoverable
        );
        assert_eq!(
            Delivery::for_message(&NetMsg::CommitSnapshotTransition {
                id: robin_engine::multiplayer::SnapshotTransitionId {
                    session_id: MultiplayerSessionId([4; 32]),
                    sequence: 1
                }
            }),
            Delivery::Required
        );
    }

    #[test]
    fn closed_writer_delivery_distinguishes_controls_from_recovery_and_diagnostics() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        drop(receiver);
        let message = || NetMsg::Note("test".into());
        assert!(queue_peer_message(&sender, message(), Delivery::Required).is_err());
        assert!(queue_peer_message(&sender, message(), Delivery::ReconnectRecoverable).is_ok());
        assert!(queue_peer_message(&sender, message(), Delivery::Diagnostic).is_ok());
    }
}
