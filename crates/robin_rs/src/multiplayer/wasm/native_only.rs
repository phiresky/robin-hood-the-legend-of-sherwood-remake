//! Browser stand-ins for the native-only transport roles, so shared session
//! code names the same types and calls on every target.

use super::{ClientHandle, NetEvent, NetOutbound, connect_client};
use std::sync::mpsc::{Receiver, Sender};

/// Browser campaign token. Browser clients keep no campaign transport state:
/// their durable browser owner reclaims a retained seat on reconnect.
#[derive(Default)]
pub struct MultiplayerCampaignSession;

/// Campaign-scoped connect with the native signature. The browser campaign
/// carries nothing, so this is exactly [`connect_client`].
pub fn connect_client_in_campaign(
    _campaign: &MultiplayerCampaignSession,
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    connect_client(addr, nickname, incoming_tx, outgoing_rx)
}

/// Browser builds cannot host, so no server handle can exist.
pub enum ServerHandle {}

impl ServerHandle {
    pub fn shutdown(&mut self) {
        match *self {}
    }

    pub(in crate::multiplayer) fn preserve_session_for_next_mission(&mut self) {
        match *self {}
    }
}
