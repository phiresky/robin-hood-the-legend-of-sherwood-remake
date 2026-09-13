//! Browser stand-ins for the native-only transport roles, so shared session
//! code names the same types and calls on every target.

use super::{ClientHandle, NetEvent, NetOutbound, connect_client};
use crate::leaderboard_ranked_session::{
    OfficialRankedSessionSetupV1, RankedPreflightLobbyV1, SharedRankedSessionLifecycle,
};
use robin_engine::player_command::PlayerId;
use robin_run_protocol::PublicKey32;
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
    pub(crate) fn ranked_lifecycle(&self) -> SharedRankedSessionLifecycle {
        match *self {}
    }

    pub(crate) fn ranked_local_seat(&self) -> Result<PlayerId, String> {
        match *self {}
    }

    pub(crate) fn ranked_host_public_key(&self) -> PublicKey32 {
        match *self {}
    }

    pub(crate) fn ranked_authenticated_seats(&self) -> Vec<(PlayerId, PublicKey32)> {
        match *self {}
    }

    pub(crate) fn ranked_preflight_lobby(&self) -> Option<RankedPreflightLobbyV1> {
        match *self {}
    }

    pub(crate) fn install_ranked_session_setup(
        &self,
        _setup: Option<OfficialRankedSessionSetupV1>,
    ) -> Result<(), String> {
        match *self {}
    }

    pub fn shutdown(&mut self) {
        match *self {}
    }

    pub(in crate::multiplayer) fn preserve_session_for_next_mission(&mut self) {
        match *self {}
    }
}
