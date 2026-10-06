//! Deterministic integration coverage across mission ingress, effect admission,
//! presentation batches and modal authority. Only socket delivery and rendering
//! are substituted: each peer owns real session channels, timeline and engine.
//! No threads, sleeps, external game data or network services are required.

use super::drain_mission_network;
use crate::game_session::replay_init::ReplayAndRollback;
use crate::game_session::runtime::{FrameContract, TimelineFrame, TimelineRuntime};
use crate::game_session::session_policy::{ModalBatchState, take_next_scripted_batch};
use crate::host::{Host, HostTransport};
use crate::ingame_menu::modal_net::{ModalDismissalGate, ModalNet};
use crate::multiplayer::NetChannels;
use crate::rewind::RewindBuffer;
use robin_engine::engine::{HostEffects, LevelAssets};
use robin_engine::engine_manager::EngineManager;
use robin_engine::multiplayer::{
    ModalDecision, ModalInstanceId, ModalRecoveryState, MultiplayerSessionId, NetEvent, NetMsg,
    NetOutbound,
};
use robin_engine::player_command::{DialogResult, ModalKind, PlayerId};
use std::collections::VecDeque;
use std::sync::{Arc, mpsc};

const RESTORED_FRAME: u32 = 133;

struct Peer {
    host: Host,
    manager: EngineManager,
    assets: Arc<LevelAssets>,
    timeline: TimelineRuntime,
    incoming: mpsc::Sender<NetEvent>,
    outgoing: mpsc::Receiver<NetOutbound>,
    batch: Option<ModalBatchState<ModalKind>>,
    visible: Option<(ModalInstanceId, ModalKind)>,
    gate: ModalDismissalGate,
    opened: Vec<(ModalInstanceId, ModalKind)>,
    completed: Vec<(ModalInstanceId, ModalKind)>,
}

impl Peer {
    fn new(seat: PlayerId) -> Self {
        let is_host = seat == PlayerId::HOST;
        let (mut engine, assets) = robin_engine::test_support::fresh_engine_sized(640.0, 480.0);
        if is_host {
            engine.test_set_frame_counter(RESTORED_FRAME);
        }
        let (net, incoming, outgoing, _, _) = NetChannels::new();
        net.install_session_id(MultiplayerSessionId([83; 32]))
            .unwrap();
        net.set_modal_player_count(2);
        net.set_modal_player_names(vec!["Desktop".into(), "Laptop".into()]);
        net.publish_frame(if is_host { RESTORED_FRAME } else { 0 });
        let mut host = Host::scratch(640.0, 480.0);
        if is_host {
            host.effects
                .bind_modal_session(net.modal_effect_admission());
        } else {
            host.effects.bind_remote_modal_session();
        }
        host.transport = HostTransport::test_session(net, seat);
        let mut timeline = TimelineRuntime::new(
            ReplayAndRollback {
                recording_control: Arc::<crate::replay_service::ReplayService>::default()
                    .recording(),
                recorder: None,
                player: None,
                rollback_checker: None,
                rewind_buffer: RewindBuffer::new(),
                start_paused: false,
            },
            FrameContract::Graphical,
            true,
            is_host,
        );
        if is_host {
            timeline.adopt_frame(TimelineFrame::from_wire(RESTORED_FRAME));
        }
        Self {
            host,
            manager: EngineManager::new(engine),
            assets: Arc::new(assets),
            timeline,
            incoming,
            outgoing,
            batch: None,
            visible: None,
            gate: ModalDismissalGate::default(),
            opened: Vec::new(),
            completed: Vec::new(),
        }
    }

    fn net(&self) -> &NetChannels {
        self.host.transport.net().unwrap()
    }

    fn drain(&mut self) -> bool {
        let result = drain_mission_network(
            &mut self.timeline,
            &mut self.host,
            &mut self.manager,
            &mut self.assets,
            false,
            u64::MAX,
        )
        .expect("mission ingress must accept the scheduled control messages");
        assert!(
            result.inputs.is_empty(),
            "story control must not fabricate gameplay input"
        );
        result.pause_simulation
    }

    /// A renderer-free surface adapter using the same batch and dismissal gate
    /// as interactive play. Surface polling can stop while ingress keeps running.
    fn draw(&mut self) {
        if self.batch.as_ref().is_none_or(ModalBatchState::is_empty) {
            self.batch = take_next_scripted_batch(&mut self.host.effects, false)
                .map(|(_, items)| ModalBatchState::new(items));
        }
        if self.visible.is_none()
            && let Some(kind) = self
                .batch
                .as_mut()
                .and_then(|batch| batch.start_next(Clone::clone))
        {
            let instance = self
                .net()
                .present_modal(&kind, self.host.transport.local_seat() == PlayerId::HOST)
                .unwrap();
            self.opened.push((instance, kind.clone()));
            self.visible = Some((instance, kind));
            self.gate = ModalDismissalGate::default();
        }
        let Some((_, kind)) = &self.visible else {
            return;
        };
        let net = ModalNet::new(
            self.host.transport.net().unwrap(),
            kind.clone(),
            self.host.transport.local_seat() == PlayerId::HOST,
        );
        if let Some(result) = self.gate.poll(Some(&net)) {
            self.finish(result);
        }
    }

    fn click(&mut self) {
        let (_, kind) = self
            .visible
            .as_ref()
            .expect("cannot acknowledge an unseen scroll");
        let net = ModalNet::new(
            self.host.transport.net().unwrap(),
            kind.clone(),
            self.host.transport.local_seat() == PlayerId::HOST,
        );
        if let Some(result) = self.gate.request(DialogResult::Completed, Some(&net)) {
            self.finish(result);
        }
    }

    fn finish(&mut self, result: DialogResult) {
        assert_eq!(result, DialogResult::Completed);
        let (instance, kind) = self.visible.take().unwrap();
        self.batch.as_mut().unwrap().finish(&kind, result);
        self.completed.push((instance, kind));
    }

    fn emit(&mut self, kinds: &[ModalKind]) {
        let mut effects = HostEffects::default();
        effects.modals.extend_from_slice(kinds);
        self.host.apply_side_effects(effects);
    }
}

/// Ordered delivery in each direction, with explicit delay, duplication and
/// disconnection. The production recovery cache retains what was sent even if
/// its final delivery was lost. No replacement implementation of modal consensus.
#[derive(Default)]
struct Link {
    to_host: VecDeque<NetEvent>,
    to_client: VecDeque<NetEvent>,
    recovery: ModalRecoveryState,
    decisions: Vec<ModalDecision>,
    started: bool,
}

impl Link {
    fn capture(&mut self, host: &Peer, client: &Peer) {
        for message in host.outgoing.try_iter() {
            let event = match message {
                NetOutbound::ModalProgress(progress) => {
                    self.recovery.observe_progress(progress.clone());
                    NetEvent::ModalProgress(progress)
                }
                NetOutbound::ModalDecision(decision) => {
                    assert!(
                        !self
                            .decisions
                            .iter()
                            .any(|old| old.instance == decision.instance
                                && old.kind == decision.kind),
                        "host published the same completion twice"
                    );
                    self.recovery.observe_decision(decision.clone());
                    self.decisions.push(decision.clone());
                    NetEvent::ModalDecision(decision)
                }
                other => panic!("unexpected host output {other:?}"),
            };
            self.to_client.push_back(event);
        }
        for message in client.outgoing.try_iter() {
            match message {
                NetOutbound::ModalProposal(proposal) => {
                    self.to_host.push_back(NetEvent::ModalProposal {
                        from: PlayerId(1),
                        proposal,
                    })
                }
                NetOutbound::ReadyToSim { frame } => {
                    assert_eq!(frame, RESTORED_FRAME);
                    let begin = NetEvent::BeginSim {
                        frame,
                        start_epoch_ms: 0,
                    };
                    if !self.started {
                        self.to_host.push_back(begin.clone());
                        self.started = true;
                    }
                    self.to_client.push_back(begin);
                }
                other => panic!("unexpected client output {other:?}"),
            }
        }
    }

    fn deliver(queue: &mut VecDeque<NetEvent>, peer: &Peer, duplicate: bool) {
        if let Some(event) = queue.pop_front() {
            if duplicate
                && matches!(
                    event,
                    NetEvent::ModalProgress(_)
                        | NetEvent::ModalDecision(_)
                        | NetEvent::ModalProposal { .. }
                )
            {
                peer.incoming.send(event.clone()).unwrap();
            }
            peer.incoming.send(event).unwrap();
        }
    }

    fn pump(&mut self, host: &mut Peer, client: &mut Peer, duplicate: bool) {
        self.capture(host, client);
        Self::deliver(&mut self.to_host, host, duplicate);
        Self::deliver(&mut self.to_client, client, duplicate);
        host.drain();
        client.drain();
    }

    fn reconnect(&mut self, host: &Peer, client: &mut Peer) {
        self.capture(host, client);
        self.to_host.clear();
        self.to_client.clear();
        client.incoming.send(NetEvent::Disconnected).unwrap();
        assert!(client.drain(), "disconnected mission must hold simulation");
        client.incoming.send(NetEvent::Reconnected).unwrap();
        client
            .incoming
            .send(NetEvent::InitialSnapshot {
                frame: RESTORED_FRAME,
                engine_bytes: host.manager.engine.encode_native_snapshot(),
            })
            .unwrap();
        for message in self.recovery.messages() {
            self.to_client.push_back(match message {
                NetMsg::ModalProgress(progress) => NetEvent::ModalProgress(progress),
                NetMsg::ModalDecision(decision) => NetEvent::ModalDecision(decision),
                other => panic!("unexpected recovery message {other:?}"),
            });
        }
        client.drain(); // Production reconnect hook must retransmit a lost ACK.
    }
}

fn pair() -> (Peer, Peer, Link) {
    let mut host = Peer::new(PlayerId::HOST);
    let mut client = Peer::new(PlayerId(1));
    let mut link = Link::default();
    client
        .incoming
        .send(NetEvent::InitialSnapshot {
            frame: RESTORED_FRAME,
            engine_bytes: host.manager.engine.encode_native_snapshot(),
        })
        .unwrap();
    // Model slow mission construction: the host cannot leave its start barrier.
    for _ in 0..20 {
        assert!(host.drain());
        assert_eq!(host.timeline.frame_number(), RESTORED_FRAME);
        assert_eq!(client.timeline.frame_number(), 0);
    }
    for _ in 0..4 {
        link.pump(&mut host, &mut client, false);
    }
    assert!(!host.drain());
    assert_eq!(client.timeline.frame_number(), RESTORED_FRAME);
    assert_eq!(
        robin_engine::replay::state_hash(&host.manager.engine),
        robin_engine::replay::state_hash(&client.manager.engine)
    );
    (host, client, link)
}

fn assert_settled(host: &mut Peer, client: &mut Peer, link: &mut Link, expected: &[ModalKind]) {
    for _ in 0..16 {
        link.pump(host, client, true);
        host.draw();
        client.draw();
    }
    assert_eq!(
        host.opened, client.opened,
        "both peers must see each exact occurrence once"
    );
    assert_eq!(host.opened, host.completed);
    assert_eq!(client.opened, client.completed);
    assert_eq!(
        host.opened
            .iter()
            .map(|(_, kind)| kind.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(link.decisions.len(), expected.len());
    assert!(!host.net().story_barrier_pending());
    assert!(!client.net().story_barrier_pending());
    assert!(
        !host.drain(),
        "all acknowledgements release the host simulation barrier"
    );
    assert_eq!(host.timeline.frame_number(), RESTORED_FRAME);
    assert_eq!(client.timeline.frame_number(), RESTORED_FRAME);
    assert_eq!(
        robin_engine::replay::state_hash(&host.manager.engine),
        robin_engine::replay::state_hash(&client.manager.engine)
    );
    client.net().resend_pending_modal_proposals().unwrap();
    assert!(
        client.outgoing.try_recv().is_err(),
        "completed ACKs must not survive recovery"
    );
}

#[test]
fn modal_session_delayed_surface_and_delivery_matrix() {
    for dialogue in [false, true] {
        for host_first in [false, true] {
            for delivery_period in [1, 3, 7] {
                for surface_period in [1, 5] {
                    let (mut host, mut client, mut link) = pair();
                    let kind = if dialogue {
                        ModalKind::Dialog { dialog_id: 7 }
                    } else {
                        ModalKind::PopupText { text_id: 17 }
                    };
                    let expected = vec![kind.clone(), kind.clone(), kind.clone()];
                    host.emit(&expected);
                    client.emit(&expected); // Prediction cannot create an extra local surface.
                    assert!(client.host.effects.modals.is_empty());
                    for ordinal in 0..expected.len() {
                        host.draw();
                        let occurrence = host.visible.clone().unwrap();
                        assert_eq!(occurrence.0.occurrence, ordinal as u64 + 1);
                        if host_first {
                            host.click();
                        }
                        // Suspended client: host polling and transport queueing continue.
                        for _ in 0..11 {
                            link.capture(&host, &client);
                            assert!(host.drain());
                            host.draw();
                            assert_eq!(host.completed.len(), ordinal);
                        }
                        for step in 0..256 {
                            if step % delivery_period == 0 {
                                link.pump(&mut host, &mut client, true);
                            } else {
                                host.drain();
                                client.drain();
                            }
                            if step % surface_period == 0 {
                                client.draw();
                            }
                            if client.visible.as_ref() == Some(&occurrence)
                                && !client.gate.is_pending()
                            {
                                if !host_first {
                                    assert_eq!(
                                        client.net().modal_waiting_names(occurrence.0, &kind),
                                        ["Desktop", "Laptop"]
                                    );
                                }
                                client.click();
                            }
                            if !host_first
                                && !host.gate.is_pending()
                                && host.net().modal_waiting_names(occurrence.0, &kind)
                                    == ["Desktop"]
                            {
                                host.click();
                            }
                            // Retire this surface, but don't open the next until the next iteration.
                            if host.visible.is_some() {
                                host.draw();
                            }
                            if host.completed.len() > ordinal && client.completed.len() > ordinal {
                                break;
                            }
                        }
                        assert_eq!(
                            host.completed.len(),
                            ordinal + 1,
                            "host stuck: dialogue={dialogue}, host_first={host_first}, delivery={delivery_period}, surface={surface_period}"
                        );
                        assert_eq!(
                            client.completed.len(),
                            ordinal + 1,
                            "client did not retire scroll"
                        );
                    }
                    assert_settled(&mut host, &mut client, &mut link, &expected);
                }
            }
        }
    }
}

#[test]
fn modal_session_catch_up_cannot_skip_an_unacknowledged_scroll() {
    let (mut host, mut client, mut link) = pair();
    let kind = ModalKind::PopupText { text_id: 17 };
    host.emit(std::slice::from_ref(&kind));
    host.draw();
    let instance = host.visible.as_ref().unwrap().0;
    host.click();
    client
        .timeline
        .adopt_frame(TimelineFrame::from_wire(RESTORED_FRAME - 1));
    for _ in 0..8 {
        link.pump(&mut host, &mut client, true);
        client.draw();
    }
    assert!(
        client.opened.is_empty(),
        "future story cannot open before its source boundary"
    );
    assert!(host.completed.is_empty());

    // Catch-up can pass the announcement's exact frame before the UI polls.
    // It must still show the host's outstanding occurrence, not discard it.
    client.timeline.adopt_frame(TimelineFrame::from_wire(704));
    client.drain();
    assert!(
        client.net().story_barrier_pending(),
        "session pauses even before drawing"
    );
    client.draw();
    assert_eq!(client.visible, Some((instance, kind.clone())));
    for _ in 0..32 {
        link.pump(&mut host, &mut client, true);
        host.draw();
        client.draw();
        assert!(
            host.completed.is_empty(),
            "catch-up is not a player acknowledgement"
        );
        assert_eq!(client.opened.len(), 1);
    }
    client.click();
    for _ in 0..8 {
        link.pump(&mut host, &mut client, true);
        host.draw();
        client.draw();
    }
    assert_eq!(host.completed, vec![(instance, kind)]);
    assert_eq!(host.completed, client.completed);
    assert_eq!(link.decisions.len(), 1);
    assert!(!host.net().story_barrier_pending());
    assert!(!client.net().story_barrier_pending());
    assert_eq!(
        host.timeline.frame_number(),
        RESTORED_FRAME,
        "a future client cursor must not force the paused host to advance"
    );
}

#[test]
fn modal_session_reconnect_recovers_lost_ack_and_next_scroll() {
    let (mut host, mut client, mut link) = pair();
    let kind = ModalKind::PopupText { text_id: 17 };
    let expected = [kind.clone(), kind.clone()];
    host.emit(&expected);
    host.draw();
    host.click();
    for _ in 0..4 {
        link.pump(&mut host, &mut client, false);
    }
    client.draw();
    assert_eq!(
        client
            .net()
            .modal_waiting_names(client.visible.as_ref().unwrap().0, &kind),
        ["Laptop"]
    );
    client.click();
    link.capture(&host, &client);
    assert!(matches!(
        link.to_host.front(),
        Some(NetEvent::ModalProposal { .. })
    ));
    link.reconnect(&host, &mut client); // Lose the queued ACK with the old connection.
    assert!(
        client.gate.is_pending(),
        "recovery must not require a second click"
    );
    let first = client.visible.as_ref().unwrap().0;
    for _ in 0..16 {
        link.pump(&mut host, &mut client, true);
        if host.net().modal_decision(first, &kind).unwrap().is_some() {
            link.capture(&host, &client);
            break;
        }
    }
    assert_eq!(link.decisions.len(), 1);
    assert_eq!(
        client.net().modal_decision(first, &kind).unwrap(),
        None,
        "the first close must still be in flight when the socket is lost"
    );
    host.draw(); // Host consumes the decision while the client surface is stalled.
    host.draw(); // Host opens the next occurrence of the same text.
    assert_eq!(host.visible.as_ref().unwrap().0.occurrence, 2);
    host.click();
    // Lose the close/next-opening stream before the old client widget consumes it.
    // Recovery must retain both the old completion and the new opening.
    link.reconnect(&host, &mut client);
    for _ in 0..16 {
        link.pump(&mut host, &mut client, true);
        client.draw();
        if client.completed.len() == 1 && client.visible.is_some() {
            break;
        }
    }
    assert_eq!(client.completed.len(), 1);
    assert_eq!(client.opened.len(), 2);
    assert_eq!(client.visible, host.visible);
    client.click();
    assert_settled(&mut host, &mut client, &mut link, &expected);
}

#[test]
fn completed_mission_snapshot_commits_the_hosts_campaign_before_rebuilding() {
    use crate::host::{PendingSnapshotTransition, PendingSnapshotTransitionPayload};
    use robin_engine::game_operation::GameCode;
    use robin_engine::multiplayer::SnapshotTransitionPayload;
    for exit_code in [GameCode::LevelSucceeded, GameCode::LevelFailed] {
        let mut host = Peer::new(PlayerId::HOST);
        let mut client = Peer::new(PlayerId(1));
        let bytes = host.manager.engine.encode_native_snapshot();
        let id = host
            .net()
            .begin_campaign_exit_transition(exit_code, bytes.clone())
            .unwrap();
        host.host
            .transport
            .prepare_snapshot_transition(PendingSnapshotTransition::new(
                id,
                PendingSnapshotTransitionPayload::CampaignExit {
                    exit_code,
                    engine: None,
                },
            ));
        client
            .incoming
            .send(NetEvent::PrepareSnapshotTransition {
                id,
                payload: SnapshotTransitionPayload::CampaignExit {
                    exit_code,
                    engine_bytes: bytes.clone(),
                },
            })
            .unwrap();
        client.drain();
        assert!(
            matches!(client.outgoing.try_recv().unwrap(), NetOutbound::SnapshotTransitionReady { id: ready } if ready == id)
        );
        client
            .incoming
            .send(NetEvent::CommitSnapshotTransition { id })
            .unwrap();
        client.drain();
        let transition = client
            .host
            .transport
            .take_committed_snapshot_transition()
            .unwrap();
        match transition.into_payload() {
            PendingSnapshotTransitionPayload::CampaignExit {
                exit_code: actual,
                engine: Some(engine),
            } => {
                assert_eq!(actual, exit_code);
                assert_eq!(engine.encode_native_snapshot(), bytes);
            }
            _ => panic!("unexpected committed mission completion"),
        }
        // Consuming a commit retires the old transport; it must not resume
        // gameplay or hand out the committed payload a second time.
        assert!(
            client
                .host
                .transport
                .take_committed_snapshot_transition()
                .is_none()
        );
        assert!(client.host.transport.reconnecting());
    }
}
