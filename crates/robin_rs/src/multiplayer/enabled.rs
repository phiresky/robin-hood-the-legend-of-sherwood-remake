//! Multiplayer transport with the `multiplayer` feature: the iroh server and
//! client on native targets, the relay-only browser client on wasm.
//!
//! Mounted by `multiplayer.rs` as `transport` and re-exported from there, so
//! every item keeps its `crate::multiplayer::…` path. `disabled.rs` provides
//! the same shared names (`MultiplayerRuntime`, `MultiplayerCampaignSession`,
//! the ranked runtime entry points) when the feature is off. Submodule files
//! stay next to this one in `multiplayer/`.

use super::*;

pub(super) mod client_gameplay;
pub(super) mod client_outgoing;
pub(super) mod client_protocol;
pub(super) mod content_transfer;
pub use client_protocol::ClientSessionMetadata;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use robin_engine::multiplayer::INPUT_DELAY_FRAMES;
pub(crate) use robin_engine::multiplayer::MultiplayerSessionId;
pub(crate) use robin_engine::multiplayer::{NET_PROTOCOL_VERSION, NetMsg, decode_msg, encode_msg};
pub(crate) use robin_engine::multiplayer::{RankedBrowseOnlyReason, RankedJoinUnavailableReason};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use robin_engine::multiplayer::{
    RankedJoinAccepted, RankedJoinClaimDocument, RankedParticipantRosterDocument,
    RankedSessionGenesisDocument,
};
pub(crate) use robin_engine::multiplayer::{
    RankedJoinAttestationDocument, RankedJoinChallenge, RankedJoinResponse,
    RankedSessionConfigDocument,
};

pub(super) mod framing;
pub(crate) use framing::*;

pub(super) mod ranked_client;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use ranked_client::{
    MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION, verify_leaderboard_cosign_response,
};
pub(crate) use ranked_client::{SharedClientLeaderboardCoSignState, SharedClientRankedJoinState};

pub mod identity;
pub mod join_ticket;
pub mod matchmaking;
#[cfg(not(target_arch = "wasm32"))]
pub mod rendezvous;

#[cfg(not(target_arch = "wasm32"))]
pub(super) mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::{
    ClientHandle, HostedModContent, MultiplayerCampaignSession, ServerChannels, ServerConfig,
    ServerHandle, connect_client, connect_client_in_campaign, start_server_in_campaign,
};

#[cfg(target_arch = "wasm32")]
pub(super) mod wasm;
#[cfg(target_arch = "wasm32")]
pub use wasm::{
    ClientHandle, MultiplayerCampaignSession, ServerHandle, connect_client,
    connect_client_in_campaign,
};

/// Owns every worker and platform resource for one multiplayer transport.
///
/// Dropping the runtime cancels its workers, closes the iroh endpoint
/// (ending every peer connection), and joins native threads.
///
/// The original game's networking behavior
/// Original-game network shutdown closes an active session and releases the
/// DirectPlay object. This runtime preserves that resource-owning RAII
/// behavior for the port's iroh transport.
///
/// Browser builds cannot host: their [`ServerHandle`] is uninhabited, so the
/// `Server` variant exists on every target but is never constructed there.
pub enum MultiplayerRuntime {
    Server(ServerHandle),
    Client(ClientHandle),
}

impl From<ServerHandle> for MultiplayerRuntime {
    fn from(handle: ServerHandle) -> Self {
        Self::Server(handle)
    }
}

impl From<ClientHandle> for MultiplayerRuntime {
    fn from(handle: ClientHandle) -> Self {
        Self::Client(handle)
    }
}

impl MultiplayerRuntime {
    /// Stop the transport now. Calling this more than once is harmless.
    pub fn shutdown(&mut self) {
        match self {
            Self::Server(handle) => handle.shutdown(),
            Self::Client(handle) => handle.shutdown(),
        }
    }

    /// Retain the authenticated host session/seat roster when this mission's
    /// transport shuts down; clients need no flag.
    pub(super) fn preserve_session_for_next_mission(&mut self) {
        match self {
            Self::Server(handle) => {
                handle.preserve_session_for_next_mission();
            }
            Self::Client(_) => {}
        }
    }
}

impl Drop for MultiplayerRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Build the ranked authorization capability of the attached runtime.
pub(super) fn attached_ranked_port(
    runtime: Option<&MultiplayerRuntime>,
    channels: &EngineNetChannels,
) -> Result<RankedMultiplayerPort, String> {
    let (
        role,
        local_seat,
        lifecycle,
        authenticated_seats,
        preflight_lobby,
        authenticated_host_public_key,
        local_public_key,
    ) = match runtime {
        Some(MultiplayerRuntime::Server(handle)) => (
            RankedMultiplayerRole::Host,
            handle.ranked_local_seat()?,
            handle.ranked_lifecycle(),
            handle.ranked_authenticated_seats(),
            handle.ranked_preflight_lobby(),
            Some(handle.ranked_host_public_key()),
            Some(handle.ranked_host_public_key()),
        ),
        Some(MultiplayerRuntime::Client(handle)) => (
            RankedMultiplayerRole::Client,
            handle.ranked_local_seat()?,
            handle.ranked_lifecycle(),
            Vec::new(),
            None,
            handle.ranked_authenticated_host_public_key(),
            handle.ranked_local_public_key(),
        ),
        None => {
            return Err(
                "ranked multiplayer capability requires an attached authenticated runtime"
                    .to_string(),
            );
        }
    };
    if (role == RankedMultiplayerRole::Host) != (local_seat == PlayerId::HOST) {
        return Err("attached multiplayer runtime reported an invalid ranked seat role".into());
    }
    Ok(RankedMultiplayerPort {
        role,
        local_seat,
        lifecycle,
        outgoing: channels.outgoing.clone(),
        authorization_inbox: channels.leaderboard_authorization_inbox(),
        authenticated_seats,
        preflight_lobby,
        local_public_key,
        authenticated_host_public_key,
    })
}

/// Hand the prepared ranked setup (or the explicit `None` decline) to the
/// attached runtime.
pub(super) fn install_attached_ranked_session_setup(
    runtime: Option<&MultiplayerRuntime>,
    setup: Option<OfficialRankedSessionSetupV1>,
) -> Result<(), String> {
    match runtime {
        Some(MultiplayerRuntime::Server(handle)) => handle.install_ranked_session_setup(setup),
        Some(MultiplayerRuntime::Client(handle)) => handle.install_ranked_session_setup(setup),
        None => Err("ranked setup requires an attached authenticated runtime".to_string()),
    }
}
