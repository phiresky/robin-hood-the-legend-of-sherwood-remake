use robin_engine::multiplayer as engine_multiplayer;
use robin_engine::player_command::{DialogResult, ModalKind, PlayerId};

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

    /// Returns true only when an authoritative host decision was queued.
    /// Client choices are visible requests and leave local modal state open.
    pub fn publish(&self, result: DialogResult) -> bool {
        let send = if self.is_host {
            self.net
                .decide_modal_dismiss(self.instance, self.kind.clone(), result)
        } else {
            self.net
                .propose_modal_dismiss(self.instance, self.kind.clone(), result)
        };
        if let Err(error) = send {
            tracing::error!(
                ?self.instance,
                kind = ?self.kind,
                ?result,
                %error,
                "multiplayer modal result was not queued; keeping modal open"
            );
            return false;
        }
        if !self.is_host {
            tracing::info!(
                ?self.instance,
                kind = ?self.kind,
                ?result,
                "multiplayer modal request sent to host"
            );
            return false;
        }
        self.net
            .complete_modal_instance(&self.kind, self.instance)
            .unwrap_or_else(|error| {
                panic!(
                    "failed to complete authoritative multiplayer modal {:?}: {error}",
                    self.instance
                )
            });
        true
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
