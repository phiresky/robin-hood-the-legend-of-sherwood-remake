use robin_engine::multiplayer as engine_multiplayer;
use robin_engine::player_command::{DialogResult, ModalKind, PlayerId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModalPublication {
    HostDecisionQueued,
    ClientProposalQueued,
}

#[derive(Default, Serialize, Deserialize)]
enum DismissalState {
    #[default]
    Open,
    Pending {
        result: DialogResult,
    },
    AwaitingAuthority,
    Complete,
}

/// Shared driver policy: a local outcome is retained until publication succeeds.
/// Failed sends retry on subsequent UI ticks, without accepting another outcome
/// or confusing the failure with a successfully submitted client proposal.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct ModalDismissalGate {
    state: DismissalState,
    network: Option<(engine_multiplayer::ModalInstanceId, ModalKind)>,
    last_error: Option<String>,
}

impl ModalDismissalGate {
    /// Retire retry state when the owning driver applies an already-authoritative
    /// replay/remote decision or discards the screen. Emits no local decision.
    pub(crate) fn retire(&mut self) {
        self.state = DismissalState::Complete;
        self.network = None;
        self.last_error = None;
    }

    pub(crate) fn is_pending(&self) -> bool {
        matches!(
            self.state,
            DismissalState::Pending { .. } | DismissalState::AwaitingAuthority
        )
    }

    pub(crate) fn request(
        &mut self,
        result: DialogResult,
        net: Option<&ModalNet<'_>>,
    ) -> Option<DialogResult> {
        assert!(
            matches!(self.state, DismissalState::Open),
            "modal already has a local outcome"
        );
        self.network = net.map(|net| (net.instance, net.kind.clone()));
        self.state = DismissalState::Pending { result };
        self.poll(net)
    }

    pub(crate) fn poll(&mut self, net: Option<&ModalNet<'_>>) -> Option<DialogResult> {
        if matches!(self.state, DismissalState::Complete) {
            return None;
        }
        if let Some((instance, kind)) = &self.network
            && let Some(net) = net
            && (net.instance != *instance || net.kind != *kind)
        {
            self.record_error(
                "multiplayer modal identity changed before its pending decision completed"
                    .to_owned(),
            );
            return None;
        }
        if let Some(result) = net.and_then(ModalNet::poll_remote_dismissal) {
            self.state = DismissalState::Complete;
            return Some(result);
        }
        let DismissalState::Pending { result } = &self.state else {
            return None;
        };
        let publication = match net {
            Some(net) => net.publish(*result),
            None if self.network.is_none() => {
                let result = *result;
                self.state = DismissalState::Complete;
                return Some(result);
            }
            None => Err("multiplayer transport disappeared before modal publication".to_owned()),
        };
        match publication {
            Ok(ModalPublication::HostDecisionQueued) => {
                let result = *result;
                self.state = DismissalState::Complete;
                Some(result)
            }
            Ok(ModalPublication::ClientProposalQueued) => {
                self.state = DismissalState::AwaitingAuthority;
                None
            }
            Err(error) => {
                self.record_error(error);
                None
            }
        }
    }

    fn record_error(&mut self, error: String) {
        if self.last_error.as_ref() != Some(&error) {
            tracing::error!(%error, "modal publication failed; retaining outcome for next UI tick retry");
            self.last_error = Some(error);
        }
    }
}

/// Multiplayer synchronization hook for cooperative modal UI.
///
/// Each modal occurrence is identified by the host session, the frame on which
/// it opened, and a per-kind occurrence counter. Clients can submit visible
/// requests, but only a host-authored decision may close the surface.
pub struct ModalNet<'a> {
    net: &'a engine_multiplayer::NetChannels,
    kind: ModalKind,
    instance: engine_multiplayer::ModalInstanceId,
    is_host: bool,
}

impl<'a> ModalNet<'a> {
    pub fn new(net: &'a engine_multiplayer::NetChannels, kind: ModalKind, is_host: bool) -> Self {
        let instance = net.open_modal_instance(&kind).unwrap_or_else(|error| {
            panic!("failed to identify multiplayer modal {kind:?}: {error}")
        });
        Self {
            net,
            kind,
            instance,
            is_host,
        }
    }

    pub fn reborrow(&self) -> ModalNet<'_> {
        ModalNet {
            net: self.net,
            kind: self.kind.clone(),
            instance: self.instance,
            is_host: self.is_host,
        }
    }

    pub fn instance(&self) -> engine_multiplayer::ModalInstanceId {
        self.instance
    }

    pub fn is_authority(&self) -> bool {
        self.is_host
    }

    /// Publication success is distinct from permission to complete the modal.
    pub fn publish(&self, result: DialogResult) -> Result<ModalPublication, String> {
        let send = if self.is_host {
            self.net
                .decide_modal_dismiss(self.instance, self.kind.clone(), result)
        } else {
            self.net
                .propose_modal_dismiss(self.instance, self.kind.clone(), result)
        };
        send.map_err(|error| {
            format!(
                "queue modal {:?} {:?} {result:?}: {error}",
                self.instance, self.kind
            )
        })?;
        if !self.is_host {
            tracing::info!(
                ?self.instance,
                kind = ?self.kind,
                ?result,
                "multiplayer modal request sent to host"
            );
            return Ok(ModalPublication::ClientProposalQueued);
        }
        self.net
            .complete_modal_instance(&self.kind, self.instance)
            .unwrap_or_else(|error| {
                panic!(
                    "failed to complete authoritative multiplayer modal {:?}: {error}",
                    self.instance
                )
            });
        Ok(ModalPublication::HostDecisionQueued)
    }

    pub fn poll_remote_dismissal(&self) -> Option<DialogResult> {
        let mut deferred_modal = Vec::new();
        let mut deferred_other = Vec::new();
        let mut matched = None;
        while let Ok(event) = self.net.try_recv_modal_event() {
            match event {
                engine_multiplayer::NetEvent::ModalDecision {
                    instance,
                    kind,
                    result,
                    decision_frame,
                } if instance == self.instance
                    && kind == self.kind
                    && decision_frame >= instance.opened_frame
                    && decision_frame <= self.net.current_frame() =>
                {
                    if self.is_host {
                        panic!(
                            "host received a remote authoritative modal decision for {:?}",
                            self.instance
                        );
                    }
                    matched = Some(result);
                    break;
                }
                engine_multiplayer::NetEvent::ModalProposal {
                    from,
                    instance,
                    kind,
                    result,
                    requested_frame,
                } if self.is_host && instance == self.instance && kind == self.kind => {
                    self.net
                        .record_visible_modal_request(engine_multiplayer::VisibleModalRequest {
                            from,
                            instance,
                            kind,
                            result,
                            requested_frame,
                        })
                        .unwrap_or_else(|error| {
                            panic!("failed to retain visible multiplayer modal request: {error}")
                        });
                    tracing::info!(
                        ?from,
                        ?instance,
                        ?result,
                        "multiplayer client requested a host modal result"
                    );
                }
                event @ (engine_multiplayer::NetEvent::ModalProposal { .. }
                | engine_multiplayer::NetEvent::ModalDecision { .. }) => {
                    deferred_modal.push(event);
                }
                other => deferred_other.push(other),
            }
        }
        for event in deferred_modal {
            self.net.defer_modal_event(event).unwrap_or_else(|error| {
                panic!("failed to preserve unmatched multiplayer modal event: {error}")
            });
        }
        self.net.defer_events(deferred_other);
        if matched.is_some() {
            self.net
                .complete_modal_instance(&self.kind, self.instance)
                .unwrap_or_else(|error| {
                    panic!(
                        "failed to complete remote multiplayer modal {:?}: {error}",
                        self.instance
                    )
                });
        }
        matched
    }

    /// Requests are presentation only. Accepting one still requires a host
    /// action which publishes the authoritative decision.
    pub fn take_visible_requests(&self) -> Vec<(PlayerId, DialogResult)> {
        self.net
            .take_visible_modal_requests(self.instance)
            .unwrap_or_else(|error| {
                panic!("failed to read visible multiplayer modal requests: {error}")
            })
            .into_iter()
            .map(|request| (request.from, request.result))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::multiplayer::{NetChannels, NetEvent, NetOutbound};

    fn kind() -> ModalKind {
        ModalKind::PopupText { text_id: 7 }
    }

    fn fixture() -> (
        NetChannels,
        std::sync::mpsc::Sender<NetEvent>,
        std::sync::mpsc::Receiver<NetOutbound>,
    ) {
        let (net, incoming, outgoing, _cursor, _snapshot) = NetChannels::new();
        net.install_session_id(engine_multiplayer::MultiplayerSessionId([4; 32]))
            .unwrap();
        (net, incoming, outgoing)
    }

    #[test]
    fn client_proposal_does_not_become_a_decision_locally() {
        let (net, _incoming, outgoing) = fixture();
        let modal = ModalNet::new(&net, kind(), false);
        assert_eq!(
            modal.publish(DialogResult::Completed).unwrap(),
            ModalPublication::ClientProposalQueued
        );
        assert!(matches!(
            outgoing.try_recv().expect("proposal"),
            NetOutbound::ModalProposal { instance, kind: observed, result: DialogResult::Completed, requested_frame: 0 }
                if instance == modal.instance() && observed == kind()
        ));
        assert!(modal.poll_remote_dismissal().is_none());
    }

    #[test]
    fn host_proposal_is_advisory_until_host_ui_decides() {
        let (net, incoming, outgoing) = fixture();
        let modal = ModalNet::new(&net, kind(), true);
        incoming
            .send(NetEvent::ModalProposal {
                from: PlayerId(2),
                instance: modal.instance(),
                kind: kind(),
                result: DialogResult::Aborted,
                requested_frame: 0,
            })
            .unwrap();
        assert_eq!(modal.poll_remote_dismissal(), None);
        assert!(outgoing.try_recv().is_err());
        assert_eq!(
            modal.take_visible_requests(),
            vec![(PlayerId(2), DialogResult::Aborted)]
        );

        assert_eq!(
            modal.publish(DialogResult::Completed).unwrap(),
            ModalPublication::HostDecisionQueued
        );
        assert!(matches!(
            outgoing.try_recv().expect("decision"),
            NetOutbound::ModalDecision { kind: observed, result: DialogResult::Completed, .. }
                if observed == kind()
        ));
    }

    #[test]
    fn client_only_closes_on_host_decision() {
        let (net, incoming, _outgoing) = fixture();
        let modal = ModalNet::new(&net, kind(), false);
        incoming
            .send(NetEvent::ModalDecision {
                instance: modal.instance(),
                kind: kind(),
                result: DialogResult::Completed,
                decision_frame: 0,
            })
            .unwrap();
        assert_eq!(modal.poll_remote_dismissal(), Some(DialogResult::Completed));
    }

    #[test]
    fn disconnected_publication_is_an_error_for_both_roles_and_preserves_instance() {
        for is_host in [false, true] {
            let (net, _incoming, outgoing) = fixture();
            let modal = ModalNet::new(&net, kind(), is_host);
            let instance = modal.instance();
            drop(outgoing);
            assert!(modal.publish(DialogResult::Aborted).is_err());
            assert_eq!(ModalNet::new(&net, kind(), is_host).instance(), instance);
        }
    }

    #[test]
    fn driver_gate_retains_failed_outcome_and_retries_without_another_ui_action() {
        for is_host in [false, true] {
            let (net, _incoming, outgoing) = fixture();
            drop(outgoing);
            let modal = ModalNet::new(&net, kind(), is_host);
            let mut gate = ModalDismissalGate::default();
            assert_eq!(gate.request(DialogResult::Aborted, Some(&modal)), None);
            assert!(matches!(
                &gate.state,
                DismissalState::Pending {
                    result: DialogResult::Aborted,
                    ..
                }
            ));
            assert!(gate.last_error.is_some());
            assert_eq!(gate.poll(Some(&modal)), None);
            assert_eq!(
                gate.poll(None),
                None,
                "lost transport must not become local authority"
            );
            assert!(gate.is_pending());

            // Replace only the disconnected channel fixture, retaining the same
            // session/occurrence identity. No second request/UI action is made.
            let (replacement, _incoming, outgoing) = fixture();
            let replacement = ModalNet::new(&replacement, kind(), is_host);
            assert_eq!(replacement.instance(), modal.instance());
            let completed = gate.poll(Some(&replacement));
            assert_eq!(completed, is_host.then_some(DialogResult::Aborted));
            assert!(matches!(
                outgoing.try_recv().unwrap(),
                NetOutbound::ModalDecision {
                    result: DialogResult::Aborted,
                    ..
                } | NetOutbound::ModalProposal {
                    result: DialogResult::Aborted,
                    ..
                }
            ));
            assert_eq!(gate.poll(Some(&replacement)), None);
            assert!(
                outgoing.try_recv().is_err(),
                "never resend an accepted outcome"
            );
            assert_eq!(gate.is_pending(), !is_host);
        }
    }

    #[test]
    fn driver_gate_waits_for_authority_and_delivers_its_result_once() {
        let (net, incoming, outgoing) = fixture();
        let modal = ModalNet::new(&net, kind(), false);
        let mut gate = ModalDismissalGate::default();
        assert_eq!(gate.request(DialogResult::Aborted, Some(&modal)), None);
        assert!(matches!(gate.state, DismissalState::AwaitingAuthority));
        assert!(matches!(
            outgoing.try_recv().unwrap(),
            NetOutbound::ModalProposal { .. }
        ));
        assert_eq!(gate.poll(Some(&modal)), None);
        assert!(outgoing.try_recv().is_err());
        incoming
            .send(NetEvent::ModalDecision {
                instance: modal.instance(),
                kind: kind(),
                result: DialogResult::Completed,
                decision_frame: 0,
            })
            .unwrap();
        assert_eq!(gate.poll(Some(&modal)), Some(DialogResult::Completed));
        assert_eq!(gate.poll(Some(&modal)), None);
        assert!(!gate.is_pending());
    }

    #[test]
    fn driver_gate_local_completion_is_one_shot() {
        let mut gate = ModalDismissalGate::default();
        assert_eq!(
            gate.request(DialogResult::Completed, None),
            Some(DialogResult::Completed)
        );
        assert_eq!(gate.poll(None), None);
        assert!(!gate.is_pending());
    }

    #[test]
    fn authoritative_decision_can_complete_a_failed_proposal_without_resending() {
        let (net, incoming, outgoing) = fixture();
        drop(outgoing);
        let modal = ModalNet::new(&net, kind(), false);
        let mut gate = ModalDismissalGate::default();
        assert_eq!(gate.request(DialogResult::Aborted, Some(&modal)), None);
        incoming
            .send(NetEvent::ModalDecision {
                instance: modal.instance(),
                kind: kind(),
                result: DialogResult::Completed,
                decision_frame: 0,
            })
            .unwrap();
        assert_eq!(gate.poll(Some(&modal)), Some(DialogResult::Completed));
        assert_eq!(gate.poll(Some(&modal)), None);
    }

    #[test]
    fn retained_outcome_cannot_publish_into_a_different_modal_occurrence() {
        let (net, _incoming, outgoing) = fixture();
        drop(outgoing);
        let modal = ModalNet::new(&net, kind(), true);
        let mut gate = ModalDismissalGate::default();
        assert_eq!(gate.request(DialogResult::Aborted, Some(&modal)), None);
        let (replacement, _incoming, outgoing) = fixture();
        let wrong_modal = ModalNet::new(&replacement, ModalKind::PopupText { text_id: 8 }, true);
        assert_eq!(gate.poll(Some(&wrong_modal)), None);
        assert!(outgoing.try_recv().is_err());
        assert!(gate.is_pending());
        assert!(
            gate.last_error
                .as_ref()
                .unwrap()
                .contains("identity changed")
        );
    }
}
