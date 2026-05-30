use robin_engine::multiplayer as engine_multiplayer;
use robin_engine::player_command::{DialogResult, ModalKind};

/// Multiplayer synchronization hook for blocking modal UI.
///
/// The normal multiplayer command drain only runs from the outer game
/// loop, but dialogue / popup windows run nested event loops.  This
/// helper lets those nested loops consume only immediate modal-dismiss
/// messages and defer every other network event back to the main loop.
pub struct ModalNet<'a> {
    net: &'a engine_multiplayer::NetChannels,
    kind: ModalKind,
}

impl<'a> ModalNet<'a> {
    pub fn new(net: &'a engine_multiplayer::NetChannels, kind: ModalKind) -> Self {
        Self { net, kind }
    }

    pub fn reborrow(&self) -> ModalNet<'_> {
        ModalNet {
            net: self.net,
            kind: self.kind.clone(),
        }
    }

    pub fn publish(&self, result: DialogResult) {
        self.net.send_modal_dismiss(self.kind.clone(), result);
    }

    pub fn poll_remote_dismissal(&self) -> Option<DialogResult> {
        let mut deferred = Vec::new();
        let mut matched = None;
        while let Ok(event) = self.net.try_recv_transport_event() {
            match event {
                engine_multiplayer::NetEvent::ModalDismiss { kind, result }
                    if kind == self.kind =>
                {
                    matched = Some(result);
                    break;
                }
                other => deferred.push(other),
            }
        }
        self.net.defer_events(deferred);
        matched
    }
}
