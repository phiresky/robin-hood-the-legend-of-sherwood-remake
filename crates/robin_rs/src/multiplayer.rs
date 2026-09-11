//! Multiplayer transport — iroh (peer-to-peer QUIC) server / client.
//!
//! The wire-format types ([`robin_engine::multiplayer::NetMsg`], [`NetEvent`], [`NetOutbound`])
//! and protocol constants live in
//! [`robin_engine::multiplayer`] so [`robin_engine::engine_manager::EngineManager`]
//! can route mutations through the rollback-safe path. This module wraps
//! the engine channel bundle in [`NetChannels`] so the channels and their
//! platform-specific [`MultiplayerRuntime`] have one owner and one lifetime.

#[cfg(feature = "multiplayer")]
mod client_gameplay;
#[cfg(feature = "multiplayer")]
mod client_protocol;
#[cfg(feature = "multiplayer")]
pub use client_protocol::ClientSessionMetadata;

#[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
pub(crate) use robin_engine::multiplayer::INPUT_DELAY_FRAMES;
use robin_engine::multiplayer::LeaderboardAuthorizationInbox;
use robin_engine::multiplayer::LeaderboardCoSignResponse;
#[cfg(any(test, feature = "multiplayer"))]
pub(crate) use robin_engine::multiplayer::MultiplayerSessionId;
use robin_engine::multiplayer::NetChannels as EngineNetChannels;
#[cfg(all(test, feature = "multiplayer", not(target_arch = "wasm32")))]
use robin_engine::multiplayer::new_frame_cursor;
pub(crate) use robin_engine::multiplayer::{
    FrameCursor, InitialSnapshot, NetEvent, NetOutbound, RankedCoSignContextDocument,
    RankedContinuationPreflightClaimDocument, RankedContinuationPreflightSignatureDocument,
    RankedContinuationReceiptSelectionDocument, RankedContinuationReceiptSelectionRequestDocument,
    RankedOfficialSessionSetupDocument, RankedSubmissionAcceptedDocument, STATE_HASH_INTERVAL,
};
#[cfg(feature = "multiplayer")]
pub(crate) use robin_engine::multiplayer::{NET_PROTOCOL_VERSION, NetMsg, decode_msg, encode_msg};
#[cfg(feature = "multiplayer")]
pub(crate) use robin_engine::multiplayer::{RankedBrowseOnlyReason, RankedJoinUnavailableReason};
#[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
pub(crate) use robin_engine::multiplayer::{
    RankedJoinAccepted, RankedJoinClaimDocument, RankedParticipantRosterDocument,
    RankedSessionGenesisDocument,
};
#[cfg(feature = "multiplayer")]
pub(crate) use robin_engine::multiplayer::{
    RankedJoinAttestationDocument, RankedJoinChallenge, RankedJoinResponse,
    RankedSessionConfigDocument,
};
use robin_engine::player_command::PlayerId;
use robin_run_protocol::{
    CampaignContinuationPreflightRequestClaimV1, LeaderboardCoSignRequestV1, ParticipantClaimV1,
    ParticipantSignatureV1, PublicKey32, SubmissionAcceptedV1, Validate,
};
use std::ops::Deref;
use std::sync::mpsc::{Receiver, Sender};

use crate::leaderboard_ranked_session::{
    CampaignContinuationReceiptSelectionRequestV1, CampaignContinuationReceiptSelectionResponseV1,
    CampaignContinuationReceiptSelectionV1, OfficialRankedSessionSetupV1,
    OfficialRankedSessionWireSetupV1, RankedCoSignContextV1, RankedPreflightLobbyV1,
    SharedRankedSessionLifecycle,
};

/// A class byte precedes each frame length so an impossible client snapshot
/// or oversized control message is rejected before allocating its body.
#[cfg(feature = "multiplayer")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum NetFrameClass {
    Control = 0,
    Input = 1,
    Snapshot = 2,
    Content = 3,
}

#[cfg(feature = "multiplayer")]
pub(crate) const MAX_SERVER_CONTROL_FRAME_BYTES: usize = 64 * 1024;
#[cfg(all(feature = "multiplayer", any(test, not(target_arch = "wasm32"))))]
pub(crate) const MAX_CLIENT_CONTROL_FRAME_BYTES: usize = 32 * 1024;
#[cfg(feature = "multiplayer")]
pub(crate) const MAX_INPUT_FRAME_BYTES: usize = 256 * 1024;
#[cfg(feature = "multiplayer")]
pub(crate) const MAX_SNAPSHOT_FRAME_BYTES: usize =
    robin_engine::multiplayer::MAX_SNAPSHOT_FRAME_BYTES;
#[cfg(all(feature = "multiplayer", any(test, not(target_arch = "wasm32"))))]
pub(crate) const MAX_HELLO_FRAME_BYTES: usize = 24 * 1024;
#[cfg(feature = "multiplayer")]
pub(crate) const MAX_CONTENT_FRAME_BYTES: usize =
    robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT + 16 * 1024;

#[cfg(feature = "multiplayer")]
impl NetFrameClass {
    pub(crate) fn from_byte(value: u8) -> Result<Self, String> {
        match value {
            0 => Ok(Self::Control),
            1 => Ok(Self::Input),
            2 => Ok(Self::Snapshot),
            3 => Ok(Self::Content),
            _ => Err(format!("unknown multiplayer frame class {value}")),
        }
    }

    pub(crate) const fn absolute_limit(self) -> usize {
        match self {
            Self::Control => MAX_SERVER_CONTROL_FRAME_BYTES,
            Self::Input => MAX_INPUT_FRAME_BYTES,
            Self::Snapshot => MAX_SNAPSHOT_FRAME_BYTES,
            Self::Content => MAX_CONTENT_FRAME_BYTES,
        }
    }
}

#[cfg(feature = "multiplayer")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InboundFramePolicy {
    // Browser production transport is client-only; shared framing tests still
    // exercise every direction on both targets.
    #[cfg(any(test, not(target_arch = "wasm32")))]
    ClientHello,
    #[cfg(any(test, not(target_arch = "wasm32")))]
    ClientToServer,
    ServerToClient,
}

#[cfg(feature = "multiplayer")]
impl InboundFramePolicy {
    pub(crate) const fn limit(self, class: NetFrameClass) -> Option<usize> {
        match (self, class) {
            #[cfg(any(test, not(target_arch = "wasm32")))]
            (Self::ClientHello, NetFrameClass::Control) => Some(MAX_HELLO_FRAME_BYTES),
            #[cfg(any(test, not(target_arch = "wasm32")))]
            (Self::ClientToServer, NetFrameClass::Control) => Some(MAX_CLIENT_CONTROL_FRAME_BYTES),
            #[cfg(any(test, not(target_arch = "wasm32")))]
            (Self::ClientToServer, NetFrameClass::Input) => Some(MAX_INPUT_FRAME_BYTES),
            (Self::ServerToClient, NetFrameClass::Control) => Some(MAX_SERVER_CONTROL_FRAME_BYTES),
            (Self::ServerToClient, NetFrameClass::Input) => Some(MAX_INPUT_FRAME_BYTES),
            (Self::ServerToClient, NetFrameClass::Snapshot) => Some(MAX_SNAPSHOT_FRAME_BYTES),
            (Self::ServerToClient, NetFrameClass::Content) => Some(MAX_CONTENT_FRAME_BYTES),
            #[cfg(any(test, not(target_arch = "wasm32")))]
            (
                Self::ClientHello,
                NetFrameClass::Input | NetFrameClass::Snapshot | NetFrameClass::Content,
            )
            | (Self::ClientToServer, NetFrameClass::Snapshot | NetFrameClass::Content) => None,
        }
    }
}

#[cfg(feature = "multiplayer")]
pub(crate) const fn net_frame_class(message: &NetMsg) -> NetFrameClass {
    match message {
        NetMsg::InitialSnapshot { .. } | NetMsg::PrepareSnapshotTransition { .. } => {
            NetFrameClass::Snapshot
        }
        NetMsg::ContentChunk { .. } => NetFrameClass::Content,
        NetMsg::Input { .. } | NetMsg::BroadcastInput { .. } => NetFrameClass::Input,
        NetMsg::Hello { .. }
        | NetMsg::Welcome { .. }
        | NetMsg::Reject { .. }
        | NetMsg::ContentOffer { .. }
        | NetMsg::ContentRequest { .. }
        | NetMsg::ContentReject { .. }
        | NetMsg::ContentReady { .. }
        | NetMsg::ContentPrepared { .. }
        | NetMsg::Note(_)
        | NetMsg::StateHash { .. }
        | NetMsg::ReadyToSim { .. }
        | NetMsg::BeginSim { .. }
        | NetMsg::ModalProposal { .. }
        | NetMsg::ModalDecision { .. }
        | NetMsg::ReconnectRequired { .. }
        | NetMsg::SnapshotTransitionReady { .. }
        | NetMsg::CommitSnapshotTransition { .. }
        | NetMsg::RankedJoinChallenge(_)
        | NetMsg::RankedJoinResponse(_)
        | NetMsg::RankedJoinAccepted(_)
        | NetMsg::RankedParticipantRoster(_)
        | NetMsg::RankedBrowseOnly { .. }
        | NetMsg::RankedCoSignContext(_)
        | NetMsg::RankedSubmissionAccepted(_)
        | NetMsg::RankedOfficialSessionSetup(_)
        | NetMsg::RankedContinuationReceiptSelectionRequest(_)
        | NetMsg::RankedContinuationReceiptSelection(_)
        | NetMsg::RankedContinuationPreflightClaim(_)
        | NetMsg::RankedContinuationPreflightSignature(_)
        | NetMsg::LeaderboardCoSignRequest(_)
        | NetMsg::LeaderboardCoSignResponse(_) => NetFrameClass::Control,
    }
}

#[cfg(feature = "multiplayer")]
mod ranked_client;
#[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
use ranked_client::decode_ranked_participant_roster;
#[cfg(all(test, feature = "multiplayer", not(target_arch = "wasm32")))]
use ranked_client::{ClientLeaderboardCoSignState, ClientRankedJoinState};
#[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
pub(crate) use ranked_client::{
    MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION, verify_leaderboard_cosign_response,
};
#[cfg(feature = "multiplayer")]
pub(crate) use ranked_client::{SharedClientLeaderboardCoSignState, SharedClientRankedJoinState};

pub mod content_identity;
#[cfg(feature = "multiplayer")]
pub mod identity;
#[cfg(feature = "multiplayer")]
pub mod join_ticket;
#[cfg(feature = "multiplayer")]
pub use join_ticket::MAX_MULTIPLAYER_PLAYERS;
#[cfg(not(feature = "multiplayer"))]
pub const MAX_MULTIPLAYER_PLAYERS: u32 = 4;
#[cfg(feature = "multiplayer")]
pub mod matchmaking;
#[cfg(not(feature = "multiplayer"))]
#[path = "multiplayer/matchmaking_disabled.rs"]
pub mod matchmaking;
#[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
pub mod rendezvous;

#[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
mod native;

#[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
pub use native::{
    ClientHandle, HostedModContent, MultiplayerCampaignSession, ServerHandle, connect_client,
    connect_client_in_campaign, start_server, start_server_in_campaign, start_server_with_content,
};

#[cfg(all(feature = "multiplayer", target_arch = "wasm32"))]
mod wasm;

#[cfg(all(feature = "multiplayer", target_arch = "wasm32"))]
pub use wasm::{ClientHandle, connect_client};

/// Owns every worker and platform resource for one multiplayer transport.
///
/// Dropping the runtime cancels its workers, closes the iroh endpoint
/// (ending every peer connection), and joins native threads.
///
/// The original game's networking behavior
/// Original-game network shutdown closes an active session and releases the
/// DirectPlay object. This runtime preserves that resource-owning RAII
/// behavior for the port's iroh transport.
#[cfg(feature = "multiplayer")]
pub enum MultiplayerRuntime {
    #[cfg(not(target_arch = "wasm32"))]
    Server(ServerHandle),
    Client(ClientHandle),
}

#[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
impl From<ServerHandle> for MultiplayerRuntime {
    fn from(handle: ServerHandle) -> Self {
        Self::Server(handle)
    }
}

#[cfg(feature = "multiplayer")]
impl From<ClientHandle> for MultiplayerRuntime {
    fn from(handle: ClientHandle) -> Self {
        Self::Client(handle)
    }
}

#[cfg(feature = "multiplayer")]
impl MultiplayerRuntime {
    /// Stop the transport now. Calling this more than once is harmless.
    pub fn shutdown(&mut self) {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Server(handle) => handle.shutdown(),
            Self::Client(handle) => handle.shutdown(),
        }
    }
}

#[cfg(feature = "multiplayer")]
impl Drop for MultiplayerRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Capability lane held by a mission-end leaderboard controller.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum RankedMultiplayerRole {
    Host,
    Client,
}

fn require_admitted_remote_ranked_claim(
    participant_claims: &[ParticipantClaimV1],
    to: PlayerId,
) -> Result<&ParticipantClaimV1, String> {
    if to == PlayerId::HOST {
        return Err("ranked authorization for the host must remain local".to_string());
    }
    let mut target_claims = participant_claims
        .iter()
        .filter(|claim| claim.seat == u16::from(to.0));
    let target_claim = target_claims
        .next()
        .ok_or_else(|| "ranked target is not an admitted participant seat".to_string())?;
    if target_claims.next().is_some() {
        return Err("ranked target seat has duplicate participant claims".to_string());
    }
    if target_claim.public_key.is_zero() {
        return Err("ranked target has a zero durable identity".to_string());
    }
    Ok(target_claim)
}

/// Only the closed authorization events a mission-end controller may consume.
/// Context bytes are decoded and validated as the closed protocol enum before
/// leaving the port.
#[derive(Clone, Debug)]
pub(crate) enum RankedAuthorizationEvent {
    OfficialSessionSetup(OfficialRankedSessionWireSetupV1),
    ContinuationReceiptSelectionRequest(CampaignContinuationReceiptSelectionRequestV1),
    ContinuationReceiptSelectionResponse {
        from: PlayerId,
        response: CampaignContinuationReceiptSelectionResponseV1,
    },
    ContinuationPreflightClaim(CampaignContinuationPreflightRequestClaimV1),
    ContinuationPreflightSignature {
        from: PlayerId,
        signature: ParticipantSignatureV1,
    },
    CoSignContext(RankedCoSignContextV1),
    SubmissionAccepted(SubmissionAcceptedV1),
    CoSignRequest(LeaderboardCoSignRequestV1),
    CoSignResponse {
        from: PlayerId,
        response: LeaderboardCoSignResponse,
    },
}

/// Cloneable, capability-restricted multiplayer access for ranked mission-end
/// authorization. It retains no runtime owner, raw transport sender, or
/// mutable [`NetChannels`] reference.
#[derive(Clone)]
pub(crate) struct RankedMultiplayerPort {
    role: RankedMultiplayerRole,
    local_seat: PlayerId,
    lifecycle: SharedRankedSessionLifecycle,
    outgoing: Sender<NetOutbound>,
    authorization_inbox: LeaderboardAuthorizationInbox,
    authenticated_seats: Vec<(PlayerId, PublicKey32)>,
    preflight_lobby: Option<RankedPreflightLobbyV1>,
    local_public_key: Option<PublicKey32>,
    authenticated_host_public_key: Option<PublicKey32>,
}

impl RankedMultiplayerPort {
    fn require_role(&self, expected: RankedMultiplayerRole) -> Result<(), String> {
        if self.role != expected {
            return Err(format!(
                "ranked multiplayer {expected:?} capability is unavailable to {:?}",
                self.role
            ));
        }
        Ok(())
    }

    pub(crate) fn role(&self) -> RankedMultiplayerRole {
        self.role
    }

    pub(crate) fn local_seat(&self) -> PlayerId {
        self.local_seat
    }

    pub(crate) fn lifecycle(&self) -> SharedRankedSessionLifecycle {
        std::sync::Arc::clone(&self.lifecycle)
    }

    pub(crate) fn authenticated_ranked_identity_pair(
        &self,
    ) -> Result<(PublicKey32, PublicKey32), String> {
        let host = self.authenticated_host_public_key.ok_or_else(|| {
            "ranked multiplayer has no authenticated host durable identity".to_string()
        })?;
        let local = self
            .local_public_key
            .ok_or_else(|| "ranked multiplayer has no local durable identity".to_string())?;
        Ok((host, local))
    }

    /// Bind an authorization response's claimed durable identity to the
    /// authenticated transport seat that delivered it. Checking seat and key
    /// independently would let two colluding seats exchange claimed keys.
    pub(crate) fn validate_authenticated_remote_identity(
        &self,
        seat: PlayerId,
        claimed_public_key: PublicKey32,
    ) -> Result<(), String> {
        self.require_role(RankedMultiplayerRole::Host)?;
        if seat == PlayerId::HOST || claimed_public_key.is_zero() {
            return Err("ranked authorization response has an invalid remote identity".to_string());
        }
        let mut matching_seats = self
            .authenticated_seats
            .iter()
            .filter(|(authenticated_seat, _)| *authenticated_seat == seat);
        let (_, authenticated_public_key) = matching_seats.next().ok_or_else(|| {
            "ranked authorization response came from an unauthenticated seat".to_string()
        })?;
        if matching_seats.next().is_some() {
            return Err(
                "ranked authorization response seat has duplicate authenticated identities"
                    .to_string(),
            );
        }
        if *authenticated_public_key != claimed_public_key {
            return Err(
                "ranked authorization response identity differs from its authenticated seat"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// Exact host-only durable lobby tuple used by every fresh/continuation
    /// authority request. It is unavailable until every configured seat has
    /// completed authenticated transport setup.
    pub(crate) fn host_preflight_lobby(&self) -> Result<RankedPreflightLobbyV1, String> {
        self.require_role(RankedMultiplayerRole::Host)?;
        self.preflight_lobby.clone().ok_or_else(|| {
            "ranked preflight requires every configured durable lobby identity".to_string()
        })
    }

    /// Broadcast the authority-admitted setup without serializing the host's
    /// notion of trusted time. Each peer must reconstruct local setup through
    /// `OfficialRankedSessionWireSetupV1::prepare_for_authenticated_peer`.
    pub(crate) fn host_publish_official_session_setup(
        &self,
        setup: &OfficialRankedSessionSetupV1,
    ) -> Result<(), String> {
        self.require_role(RankedMultiplayerRole::Host)?;
        let setup = OfficialRankedSessionWireSetupV1::from_local_setup(setup)
            .map_err(|error| format!("prepare official ranked wire setup: {error}"))?;
        let bytes = crate::leaderboard_ranked_session::encode_ranked_wire_document(&setup)
            .map_err(|error| format!("encode official ranked wire setup: {error}"))?;
        let setup = RankedOfficialSessionSetupDocument::new(bytes)
            .map_err(|error| format!("wrap official ranked wire setup: {error}"))?;
        self.outgoing
            .send(NetOutbound::RankedOfficialSessionSetup(setup))
            .map_err(|_| "ranked multiplayer authorization channel is closed".to_string())
    }

    /// Ask every authenticated remote seat to compare its local active chain
    /// receipt with this exact host-derived lobby/ranked tuple. Only the
    /// immutable controller is allowed to answer at the response boundary.
    pub(crate) fn host_publish_continuation_receipt_selection_request(
        &self,
        request: &CampaignContinuationReceiptSelectionRequestV1,
    ) -> Result<(), String> {
        self.require_role(RankedMultiplayerRole::Host)?;
        request
            .validate()
            .map_err(|error| format!("invalid continuation receipt selection request: {error}"))?;
        if Some(&request.lobby) != self.preflight_lobby.as_ref() {
            return Err(
                "continuation receipt selection request differs from authenticated lobby"
                    .to_string(),
            );
        }
        let bytes = crate::leaderboard_ranked_session::encode_ranked_wire_document(request)
            .map_err(|error| format!("encode continuation receipt selection request: {error}"))?;
        let request = RankedContinuationReceiptSelectionRequestDocument::new(bytes)
            .map_err(|error| format!("wrap continuation receipt selection request: {error}"))?;
        self.outgoing
            .send(NetOutbound::RankedContinuationReceiptSelectionRequest(
                request,
            ))
            .map_err(|_| "ranked multiplayer authorization channel is closed".to_string())
    }

    /// Controller response to an exact selection request. The host transport
    /// binds the response to this client's authenticated durable seat.
    pub(crate) fn client_respond_continuation_receipt_selection(
        &self,
        selection: &CampaignContinuationReceiptSelectionV1,
    ) -> Result<(), String> {
        self.require_role(RankedMultiplayerRole::Client)?;
        selection
            .validate()
            .map_err(|error| format!("invalid continuation receipt selection: {error}"))?;
        self.client_respond_continuation_receipt_selection_response(
            &CampaignContinuationReceiptSelectionResponseV1::Selected {
                selection: selection.clone(),
            },
        )
    }

    pub(crate) fn client_respond_no_matching_continuation_receipt(
        &self,
        request: CampaignContinuationReceiptSelectionRequestV1,
        responder_public_key: PublicKey32,
    ) -> Result<(), String> {
        self.client_respond_continuation_receipt_selection_response(
            &CampaignContinuationReceiptSelectionResponseV1::NoMatchingReceipt {
                request,
                responder_public_key,
            },
        )
    }

    fn client_respond_continuation_receipt_selection_response(
        &self,
        response: &CampaignContinuationReceiptSelectionResponseV1,
    ) -> Result<(), String> {
        self.require_role(RankedMultiplayerRole::Client)?;
        response
            .validate()
            .map_err(|error| format!("invalid continuation receipt selection response: {error}"))?;
        let local_public_key = self.local_public_key.ok_or_else(|| {
            "ranked multiplayer has no local durable identity for receipt selection".to_string()
        })?;
        if response.responder_public_key() != local_public_key {
            return Err(
                "continuation receipt selection claims another authenticated identity".to_string(),
            );
        }
        let bytes = crate::leaderboard_ranked_session::encode_ranked_wire_document(response)
            .map_err(|error| format!("encode continuation receipt selection: {error}"))?;
        let selection = RankedContinuationReceiptSelectionDocument::new(bytes)
            .map_err(|error| format!("wrap continuation receipt selection: {error}"))?;
        self.outgoing
            .send(NetOutbound::RankedContinuationReceiptSelection(selection))
            .map_err(|_| "ranked multiplayer authorization channel is closed".to_string())
    }

    /// Publish the exact host-signed continuation preflight claim to the one
    /// authenticated seat whose durable key is the immutable campaign
    /// controller. The controller seat is derived here, never supplied by a
    /// presentation/runtime caller.
    pub(crate) fn host_publish_continuation_preflight_claim(
        &self,
        claim: &CampaignContinuationPreflightRequestClaimV1,
    ) -> Result<PlayerId, String> {
        self.require_role(RankedMultiplayerRole::Host)?;
        claim
            .validate()
            .map_err(|error| format!("invalid continuation preflight claim: {error}"))?;
        let mut seats = self
            .authenticated_seats
            .iter()
            .filter(|(_, key)| *key == claim.campaign_controller_public_key)
            .map(|(seat, _)| *seat);
        let to = seats.next().ok_or_else(|| {
            "continuation controller is not an authenticated multiplayer seat".to_string()
        })?;
        if seats.next().is_some() {
            return Err("continuation controller key owns multiple multiplayer seats".to_string());
        }
        if to == PlayerId::HOST {
            return Err("local host controller preflight must be signed without transport".into());
        }
        let bytes = crate::leaderboard_ranked_session::encode_ranked_wire_document(claim)
            .map_err(|error| format!("encode continuation preflight claim: {error}"))?;
        let claim = RankedContinuationPreflightClaimDocument::new(bytes)
            .map_err(|error| format!("wrap continuation preflight claim: {error}"))?;
        self.outgoing
            .send(NetOutbound::RankedContinuationPreflightClaim { to, claim })
            .map_err(|_| "ranked multiplayer authorization channel is closed".to_string())?;
        Ok(to)
    }

    /// Return the controller's domain-bound signature for the exact claim
    /// delivered through the authorization inbox.
    pub(crate) fn client_respond_continuation_preflight(
        &self,
        signature: ParticipantSignatureV1,
    ) -> Result<(), String> {
        self.require_role(RankedMultiplayerRole::Client)?;
        if signature.public_key.is_zero() || signature.signature.is_zero() {
            return Err("continuation preflight response contains zero key material".into());
        }
        let local_public_key = self.local_public_key.ok_or_else(|| {
            "ranked multiplayer has no local durable identity for continuation preflight"
                .to_string()
        })?;
        if signature.public_key != local_public_key {
            return Err(
                "continuation preflight response claims another authenticated identity".into(),
            );
        }
        let bytes = crate::leaderboard_ranked_session::encode_ranked_wire_document(&signature)
            .map_err(|error| format!("encode continuation preflight signature: {error}"))?;
        let signature = RankedContinuationPreflightSignatureDocument::new(bytes)
            .map_err(|error| format!("wrap continuation preflight signature: {error}"))?;
        self.outgoing
            .send(NetOutbound::RankedContinuationPreflightSignature(signature))
            .map_err(|_| "ranked multiplayer authorization channel is closed".to_string())
    }

    /// Host-only publication of one complete closed co-sign operation. The
    /// exact request is derived from the validated typed context here, then the
    /// context and request are queued in that order. Callers cannot substitute
    /// an unrelated request or publish arbitrary signing bytes.
    pub(crate) fn host_publish_co_sign_operation(
        &self,
        to: PlayerId,
        context: &RankedCoSignContextV1,
    ) -> Result<LeaderboardCoSignRequestV1, String> {
        self.require_role(RankedMultiplayerRole::Host)?;
        if to == PlayerId::HOST {
            return Err("ranked co-sign context for the host must remain local".to_string());
        }
        context
            .validate()
            .map_err(|error| format!("invalid ranked co-sign context: {error}"))?;
        let offer_request = match context {
            RankedCoSignContextV1::CampaignContinuation(context) => &context.offer_request,
            RankedCoSignContextV1::Submission(context) => &context.offer_request,
        };
        let lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| "ranked session lifecycle lock is poisoned".to_string())?;
        let session = lifecycle.ranked_session().ok_or_else(|| {
            "host cannot publish a co-sign operation outside an admitted ranked session".to_string()
        })?;
        let retained_participant_claims = session.participant_claims();
        require_admitted_remote_ranked_claim(&retained_participant_claims, to)?;
        if session.genesis() != &offer_request.session_genesis
            || retained_participant_claims != offer_request.participant_claims
        {
            return Err(
                "ranked co-sign context roster does not equal the retained final roster"
                    .to_string(),
            );
        }
        drop(lifecycle);
        let request = match context {
            RankedCoSignContextV1::CampaignContinuation(context) => context
                .continuation_claim
                .co_sign_request(&context.offer)
                .map_err(|error| format!("derive ranked continuation co-sign request: {error}"))?,
            RankedCoSignContextV1::Submission(context) => context
                .submission
                .co_sign_request()
                .map_err(|error| format!("derive ranked submission co-sign request: {error}"))?,
        };
        request
            .validate()
            .map_err(|error| format!("invalid derived leaderboard co-sign request: {error}"))?;
        let bytes = crate::leaderboard_ranked_session::encode_ranked_wire_document(context)
            .map_err(|error| format!("encode ranked co-sign context: {error}"))?;
        let context = RankedCoSignContextDocument::new(bytes)
            .map_err(|error| format!("encode ranked co-sign context: {error}"))?;
        self.outgoing
            .send(NetOutbound::RankedCoSignContext { to, context })
            .map_err(|_| "ranked multiplayer authorization channel is closed".to_string())?;
        self.outgoing
            .send(NetOutbound::LeaderboardCoSignRequest { to, request })
            .map_err(|_| "ranked multiplayer authorization channel is closed".to_string())?;
        Ok(request)
    }

    /// Host-only notification that the leaderboard service accepted a final
    /// submission into its verification queue. The destination is derived
    /// from the retained authenticated participant roster, so a caller cannot
    /// substitute a different seat for the campaign controller's durable key.
    pub(crate) fn host_publish_submission_accepted(
        &self,
        controller_public_key: PublicKey32,
        accepted: SubmissionAcceptedV1,
    ) -> Result<PlayerId, String> {
        self.require_role(RankedMultiplayerRole::Host)?;
        if controller_public_key.is_zero() {
            return Err("ranked submission controller public key is zero".to_string());
        }
        accepted
            .validate()
            .map_err(|error| format!("invalid ranked submission acknowledgement: {error}"))?;
        let lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| "ranked session lifecycle lock is poisoned".to_string())?;
        let session = lifecycle.ranked_session().ok_or_else(|| {
            "host cannot publish a submission acknowledgement outside an admitted ranked session"
                .to_string()
        })?;
        let mut matching_seats = session
            .participant_claims()
            .into_iter()
            .filter(|claim| claim.public_key == controller_public_key)
            .map(|claim| claim.seat);
        let seat = matching_seats.next().ok_or_else(|| {
            "ranked submission controller is not bound to an admitted participant".to_string()
        })?;
        if matching_seats.next().is_some() {
            return Err(
                "ranked submission controller key is bound to multiple participant seats"
                    .to_string(),
            );
        }
        if seat == u16::from(PlayerId::HOST.0) {
            return Err(
                "ranked submission acknowledgement for the host must remain local".to_string(),
            );
        }
        let to = PlayerId(u8::try_from(seat).map_err(|_| {
            "ranked submission controller seat exceeds multiplayer seat range".to_string()
        })?);
        let bytes = crate::leaderboard_ranked_session::encode_ranked_wire_document(&accepted)
            .map_err(|error| format!("encode ranked submission acknowledgement: {error}"))?;
        let accepted = RankedSubmissionAcceptedDocument::new(bytes)
            .map_err(|error| format!("encode ranked submission acknowledgement: {error}"))?;
        self.outgoing
            .send(NetOutbound::RankedSubmissionAccepted { to, accepted })
            .map_err(|_| "ranked multiplayer authorization channel is closed".to_string())?;
        drop(lifecycle);
        Ok(to)
    }

    /// Client-only arm of the exact purpose-bound request derived after local
    /// evidence validates a received typed context.
    pub(crate) fn client_arm_co_sign_request(
        &self,
        request: LeaderboardCoSignRequestV1,
    ) -> Result<(), String> {
        self.require_role(RankedMultiplayerRole::Client)?;
        request
            .validate()
            .map_err(|error| format!("invalid leaderboard co-sign request: {error}"))?;
        self.outgoing
            .send(NetOutbound::ArmLeaderboardCoSignRequest { request })
            .map_err(|_| "ranked multiplayer authorization channel is closed".to_string())
    }

    /// Client-only response to a request already released by the transport's
    /// exact request equality gate.
    pub(crate) fn client_respond_co_sign(
        &self,
        response: LeaderboardCoSignResponse,
    ) -> Result<(), String> {
        self.require_role(RankedMultiplayerRole::Client)?;
        if response.signer_public_key == [0; 32] || response.signature == [0; 64] {
            return Err("leaderboard co-sign response contains zero key material".to_string());
        }
        response
            .instance
            .validate()
            .map_err(|error| format!("invalid leaderboard co-sign response: {error}"))?;
        self.outgoing
            .send(NetOutbound::LeaderboardCoSignResponse(response))
            .map_err(|_| "ranked multiplayer authorization channel is closed".to_string())
    }

    /// Non-blocking access to the bounded ranked authorization inbox. Any role
    /// violation fails closed rather than exposing a transport event to the
    /// wrong authority.
    pub(crate) fn try_recv_authorization_event(
        &self,
    ) -> Result<Option<RankedAuthorizationEvent>, String> {
        let event = self
            .authorization_inbox
            .lock()
            .map_err(|_| "multiplayer ranked authorization inbox lock is poisoned".to_string())?
            .pop_front();
        let Some(event) = event else {
            return Ok(None);
        };
        match (self.role, event) {
            (RankedMultiplayerRole::Client, NetEvent::RankedOfficialSessionSetup(document)) => {
                let setup =
                    crate::leaderboard_ranked_session::decode_canonical_ranked_wire_document(
                        document.as_bytes(),
                    )
                    .map_err(|error| format!("invalid official ranked session setup: {error}"))?;
                Ok(Some(RankedAuthorizationEvent::OfficialSessionSetup(setup)))
            }
            (
                RankedMultiplayerRole::Client,
                NetEvent::RankedContinuationReceiptSelectionRequest(document),
            ) => {
                let request = crate::leaderboard_ranked_session::decode_ranked_wire_document(
                    document.as_bytes(),
                )
                .map_err(|error| {
                    format!("invalid continuation receipt selection request: {error}")
                })?;
                Ok(Some(
                    RankedAuthorizationEvent::ContinuationReceiptSelectionRequest(request),
                ))
            }
            (
                RankedMultiplayerRole::Host,
                NetEvent::RankedContinuationReceiptSelection { from, selection },
            ) => {
                let response = crate::leaderboard_ranked_session::decode_ranked_wire_document(
                    selection.as_bytes(),
                )
                .map_err(|error| format!("invalid continuation receipt selection: {error}"))?;
                Ok(Some(
                    RankedAuthorizationEvent::ContinuationReceiptSelectionResponse {
                        from,
                        response,
                    },
                ))
            }
            (
                RankedMultiplayerRole::Client,
                NetEvent::RankedContinuationPreflightClaim(document),
            ) => {
                let claim = crate::leaderboard_ranked_session::decode_ranked_wire_document(
                    document.as_bytes(),
                )
                .map_err(|error| format!("invalid continuation preflight claim: {error}"))?;
                Ok(Some(RankedAuthorizationEvent::ContinuationPreflightClaim(
                    claim,
                )))
            }
            (
                RankedMultiplayerRole::Host,
                NetEvent::RankedContinuationPreflightSignature { from, signature },
            ) => {
                let signature: ParticipantSignatureV1 =
                    crate::leaderboard_ranked_session::decode_canonical_ranked_wire_document(
                        signature.as_bytes(),
                    )
                    .map_err(|error| {
                        format!("invalid continuation preflight signature: {error}")
                    })?;
                if signature.public_key.is_zero() || signature.signature.is_zero() {
                    return Err(
                        "continuation preflight signature contains zero key material".to_string(),
                    );
                }
                Ok(Some(
                    RankedAuthorizationEvent::ContinuationPreflightSignature { from, signature },
                ))
            }
            (RankedMultiplayerRole::Client, NetEvent::RankedCoSignContext(document)) => {
                let context = crate::leaderboard_ranked_session::decode_ranked_wire_document(
                    document.as_bytes(),
                )
                .map_err(|error| format!("invalid ranked co-sign context: {error}"))?;
                Ok(Some(RankedAuthorizationEvent::CoSignContext(context)))
            }
            (RankedMultiplayerRole::Client, NetEvent::RankedSubmissionAccepted(document)) => {
                let accepted = crate::leaderboard_ranked_session::decode_ranked_wire_document(
                    document.as_bytes(),
                )
                .map_err(|error| format!("invalid ranked submission acknowledgement: {error}"))?;
                Ok(Some(RankedAuthorizationEvent::SubmissionAccepted(accepted)))
            }
            (RankedMultiplayerRole::Client, NetEvent::LeaderboardCoSignRequest(request)) => {
                request
                    .validate()
                    .map_err(|error| format!("invalid leaderboard co-sign request: {error}"))?;
                Ok(Some(RankedAuthorizationEvent::CoSignRequest(request)))
            }
            (
                RankedMultiplayerRole::Host,
                NetEvent::LeaderboardCoSignResponse { from, response },
            ) => {
                if from == PlayerId::HOST
                    || response.signer_public_key == [0; 32]
                    || response.signature == [0; 64]
                {
                    return Err("invalid authenticated leaderboard co-sign response".to_string());
                }
                response.instance.validate().map_err(|error| {
                    format!("invalid authenticated leaderboard co-sign response: {error}")
                })?;
                Ok(Some(RankedAuthorizationEvent::CoSignResponse {
                    from,
                    response,
                }))
            }
            (role, _) => Err(format!(
                "ranked multiplayer authorization inbox contained an event invalid for {role:?}"
            )),
        }
    }
}

/// Game-loop channels coupled to the runtime that services them.
///
/// Field order is intentional: the engine channel senders are dropped before
/// the runtime, then runtime shutdown joins workers after their channel ends
/// have closed.
pub struct NetChannels {
    channels: EngineNetChannels,
    #[cfg(feature = "multiplayer")]
    runtime: Option<MultiplayerRuntime>,
}

impl NetChannels {
    /// Build an unattached channel bundle. The caller must attach the runtime
    /// returned by [`start_server`] or [`connect_client`] before publishing the
    /// bundle to the game loop.
    pub fn new() -> (
        Self,
        Sender<NetEvent>,
        Receiver<NetOutbound>,
        FrameCursor,
        InitialSnapshot,
    ) {
        let (channels, incoming_tx, outgoing_rx, frame_cursor, initial_snapshot) =
            EngineNetChannels::new();
        (
            Self {
                channels,
                #[cfg(feature = "multiplayer")]
                runtime: None,
            },
            incoming_tx,
            outgoing_rx,
            frame_cursor,
            initial_snapshot,
        )
    }

    /// Couple the channel bundle to its transport owner.
    #[cfg(feature = "multiplayer")]
    pub fn attach_runtime(&mut self, runtime: impl Into<MultiplayerRuntime>) {
        assert!(
            self.runtime.is_none(),
            "multiplayer channels already have an attached runtime"
        );
        self.runtime = Some(runtime.into());
    }

    /// Create a mission-end ranked authorization capability without lending
    /// the runtime owner or this non-clone channel bundle.
    #[cfg(feature = "multiplayer")]
    pub(crate) fn ranked_port(&self) -> Result<RankedMultiplayerPort, String> {
        let (
            role,
            local_seat,
            lifecycle,
            authenticated_seats,
            preflight_lobby,
            authenticated_host_public_key,
            local_public_key,
        ) = match self.runtime.as_ref() {
            #[cfg(not(target_arch = "wasm32"))]
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
            outgoing: self.channels.outgoing.clone(),
            authorization_inbox: self.channels.leaderboard_authorization_inbox(),
            authenticated_seats,
            preflight_lobby,
            local_public_key,
            authenticated_host_public_key,
        })
    }

    #[cfg(not(feature = "multiplayer"))]
    pub(crate) fn ranked_port(&self) -> Result<RankedMultiplayerPort, String> {
        Err(
            "ranked multiplayer capability is unavailable without the multiplayer feature"
                .to_owned(),
        )
    }

    /// Resolve ranked bootstrap once exact official mission inputs are
    /// prepared. The attached authenticated runtime owns construction of its
    /// role-specific state. `None` explicitly and irreversibly declines ranking.
    #[cfg(feature = "multiplayer")]
    pub(crate) fn install_ranked_session_setup(
        &self,
        setup: Option<OfficialRankedSessionSetupV1>,
    ) -> Result<(), String> {
        match self.runtime.as_ref() {
            #[cfg(not(target_arch = "wasm32"))]
            Some(MultiplayerRuntime::Server(handle)) => handle.install_ranked_session_setup(setup),
            Some(MultiplayerRuntime::Client(handle)) => handle.install_ranked_session_setup(setup),
            None => Err("ranked setup requires an attached authenticated runtime".to_string()),
        }
    }

    #[cfg(not(feature = "multiplayer"))]
    pub(crate) fn install_ranked_session_setup(
        &self,
        _setup: Option<OfficialRankedSessionSetupV1>,
    ) -> Result<(), String> {
        Err("ranked multiplayer setup is unavailable without the multiplayer feature".to_owned())
    }

    /// Explicitly stop and detach the transport. Drop performs the same work.
    pub fn shutdown(&mut self) {
        #[cfg(feature = "multiplayer")]
        if let Some(mut runtime) = self.runtime.take() {
            runtime.shutdown();
        }
    }

    /// Retain the authenticated host session/seat roster when this mission's
    /// transport shuts down. Clients require no explicit flag: their durable
    /// browser owner or process-held native key reclaims the retained seat.
    pub(crate) fn preserve_session_for_next_mission(&mut self) {
        #[cfg(feature = "multiplayer")]
        match self.runtime.as_mut() {
            #[cfg(not(target_arch = "wasm32"))]
            Some(MultiplayerRuntime::Server(handle)) => {
                handle.preserve_session_for_next_mission();
            }
            Some(MultiplayerRuntime::Client(_)) | None => {}
        }
    }
}

impl Deref for NetChannels {
    type Target = EngineNetChannels;

    fn deref(&self) -> &Self::Target {
        &self.channels
    }
}

#[cfg(all(test, feature = "multiplayer", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::distributed_mod::DistributedModPackage;
    use crate::leaderboard_ranked_session::{
        RankedSessionHost, RankedSessionLifecycle, encode_ranked_wire_document,
        sign_named_seat_join,
    };
    use crate::multiplayer::native::{
        HostedModContent, connect_client, connect_client_with_key, start_server_with_key,
        start_server_with_key_and_content,
    };
    use robin_engine::multiplayer::LeaderboardCoSignResponse;
    use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
    use robin_run_protocol::{
        Digest32, LeaderboardCoSignInstanceV1, LeaderboardCoSignPurposeV1,
        LeaderboardCoSignRequestV1, OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId,
        ParticipantPublicDisclosureV1, PublicKey32, RankedSessionConfigV1, ResourceLocaleRootV1,
        SCHEMA_VERSION_V1, SimulationSeed64, SpeechTimingAuthorityV1, SubmissionAcceptedV1,
        SubmissionLifecycleV1,
    };
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn ranked_signing_key(key: &iroh::SecretKey) -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&key.to_bytes())
    }

    fn start_owned_server() -> (NetChannels, String) {
        let (mut channels, incoming_tx, outgoing_rx, frame_cursor, initial_snapshot) =
            NetChannels::new();
        let handle = start_server_with_key(
            iroh::SecretKey::generate(),
            "host".into(),
            "Dem_Lei_MP".into(),
            42,
            robin_engine::engine::SimConfig::default(),
            Some("en-US".into()),
            incoming_tx,
            outgoing_rx,
            frame_cursor,
            initial_snapshot,
            1,
        )
        .expect("start server on an ephemeral iroh identity");
        let connect_string = handle.connect_string();
        channels.attach_runtime(handle);
        (channels, connect_string)
    }

    fn leaderboard_request(
        purpose: LeaderboardCoSignPurposeV1,
        byte: u8,
    ) -> LeaderboardCoSignRequestV1 {
        LeaderboardCoSignRequestV1 {
            instance: LeaderboardCoSignInstanceV1 {
                purpose,
                replay_session_id: Digest32::from_bytes([byte; 32]),
                submission_offer_sha256: Digest32::from_bytes([byte.wrapping_add(1); 32]),
            },
            run_digest: Digest32::from_bytes([byte.wrapping_add(2); 32]),
        }
    }

    fn signed_response(
        request: &LeaderboardCoSignRequestV1,
        key: &iroh::SecretKey,
    ) -> LeaderboardCoSignResponse {
        let payload = request.signing_bytes().unwrap();
        LeaderboardCoSignResponse {
            instance: request.instance,
            signer_public_key: *key.public().as_bytes(),
            signature: key.sign(&payload).to_bytes(),
        }
    }

    fn ranked_config() -> RankedSessionConfigV1 {
        RankedSessionConfigV1 {
            custom_rules_config: None,
            custom_canonical_campaign: None,
            schema_version: SCHEMA_VERSION_V1,
            mission_id: "Dem_Lei_MP".to_string(),
            content_edition: OfficialContentEditionV1::Demo,
            content_subject: OfficialContentSubjectV1::FieldMission {
                mission_id: "Dem_Lei_MP".to_string(),
            },
            simulation_seed: SimulationSeed64::new(7),
            starting_campaign_sha256: Digest32::from_bytes([1; 32]),
            starting_campaign_byte_length: 1,
            prepared_inputs_projection_sha256: Digest32::from_bytes([2; 32]),
            prepared_mission_inputs_seal_sha256: Digest32::from_bytes([3; 32]),
            build_manifest_sha256: Digest32::from_bytes([4; 32]),
            content_manifest_sha256: Digest32::from_bytes([5; 32]),
            campaign_content_manifest_sha256: None,
            rules_config_sha256: Digest32::from_bytes([6; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([7; 32]),
            competition_manifest_sha256: None,
            spellforge_content_sha256: None,
            resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
            speech_timing: SpeechTimingAuthorityV1::BaseInstallation,
        }
    }

    fn submission_accepted() -> SubmissionAcceptedV1 {
        SubmissionAcceptedV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission_id: OpaqueId::new("submission-accepted-test").unwrap(),
            state: SubmissionLifecycleV1::Queued,
            retry_after_ms: 250,
        }
    }

    struct RankedJoinFixture {
        expected: RankedSessionConfigDocument,
        challenge: RankedJoinChallenge,
        response: RankedJoinResponse,
        accepted: RankedJoinAccepted,
        host: RankedSessionHost,
        guest_key: iroh::SecretKey,
    }

    fn ranked_join_fixture() -> RankedJoinFixture {
        let config = ranked_config();
        let expected =
            RankedSessionConfigDocument::new(encode_ranked_wire_document(&config).unwrap())
                .unwrap();
        let host_key = iroh::SecretKey::generate();
        let guest_key = iroh::SecretKey::generate();
        let transport_endpoint = *guest_key.public().as_bytes();
        let mut host =
            RankedSessionHost::new(&ranked_signing_key(&host_key), NET_PROTOCOL_VERSION, config)
                .unwrap();
        let claim = host
            .prepare_join(
                1,
                PublicKey32::from_bytes(*guest_key.public().as_bytes()),
                PublicKey32::from_bytes(transport_endpoint),
                PublicKey32::from_bytes(*host_key.public().as_bytes()),
            )
            .unwrap();
        let attestation =
            sign_named_seat_join(&ranked_signing_key(&guest_key), claim.clone()).unwrap();
        host.admit_join(
            attestation.clone(),
            transport_endpoint,
            ParticipantPublicDisclosureV1::NamedProfile,
        )
        .unwrap();
        let session_genesis =
            RankedSessionGenesisDocument::new(encode_ranked_wire_document(host.genesis()).unwrap())
                .unwrap();
        let join_claim =
            RankedJoinClaimDocument::new(encode_ranked_wire_document(&claim).unwrap()).unwrap();
        let join_attestation =
            RankedJoinAttestationDocument::new(encode_ranked_wire_document(&attestation).unwrap())
                .unwrap();
        let participant_roster = RankedParticipantRosterDocument::new(
            encode_ranked_wire_document(&host.participant_claims()).unwrap(),
        )
        .unwrap();
        RankedJoinFixture {
            expected,
            challenge: RankedJoinChallenge {
                session_genesis: session_genesis.clone(),
                join_claim,
            },
            response: RankedJoinResponse::Attestation(join_attestation.clone()),
            accepted: RankedJoinAccepted {
                session_genesis,
                join_attestation,
                participant_roster,
            },
            host,
            guest_key,
        }
    }

    #[test]
    fn cosign_messages_are_bounded_control_frames_in_each_live_direction() {
        let request = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 30);
        let response = signed_response(&request, &iroh::SecretKey::generate());
        for message in [
            NetMsg::LeaderboardCoSignRequest(request),
            NetMsg::LeaderboardCoSignResponse(response),
        ] {
            assert_eq!(net_frame_class(&message), NetFrameClass::Control);
            let encoded = encode_msg(&message);
            assert!(encoded.len() <= MAX_CLIENT_CONTROL_FRAME_BYTES);
            assert!(encoded.len() <= MAX_SERVER_CONTROL_FRAME_BYTES);
            assert_eq!(
                InboundFramePolicy::ClientToServer.limit(NetFrameClass::Control),
                Some(MAX_CLIENT_CONTROL_FRAME_BYTES)
            );
            assert_eq!(
                InboundFramePolicy::ServerToClient.limit(NetFrameClass::Control),
                Some(MAX_SERVER_CONTROL_FRAME_BYTES)
            );
        }
    }

    #[test]
    fn ranked_wire_messages_are_bounded_control_frames_without_protocol_bump() {
        let fixture = ranked_join_fixture();
        let context = RankedCoSignContextDocument::new(br#"{"kind":"test"}"#.to_vec()).unwrap();
        let accepted = RankedSubmissionAcceptedDocument::new(
            encode_ranked_wire_document(&submission_accepted()).unwrap(),
        )
        .unwrap();
        let roster = fixture.accepted.participant_roster.clone();
        let messages = vec![
            NetMsg::RankedJoinChallenge(fixture.challenge),
            NetMsg::RankedJoinResponse(fixture.response),
            NetMsg::RankedJoinAccepted(fixture.accepted),
            NetMsg::RankedParticipantRoster(roster),
            NetMsg::RankedBrowseOnly {
                reason: RankedBrowseOnlyReason::PeerAttestationRejected,
            },
            NetMsg::RankedCoSignContext(context),
            NetMsg::RankedSubmissionAccepted(accepted),
        ];
        assert_eq!(NET_PROTOCOL_VERSION, 46);
        for message in messages {
            assert_eq!(net_frame_class(&message), NetFrameClass::Control);
            let encoded = encode_msg(&message);
            assert!(encoded.len() <= MAX_SERVER_CONTROL_FRAME_BYTES);
            assert!(encoded.len() <= MAX_CLIENT_CONTROL_FRAME_BYTES);
            assert!(
                matches!(decode_msg(&encoded), Ok(decoded) if std::mem::discriminant(&decoded) == std::mem::discriminant(&message))
            );
        }
    }

    #[test]
    fn ranked_document_and_hello_bounds_reject_empty_oversize_and_zero_identity() {
        assert!(RankedSessionGenesisDocument::new(Vec::new()).is_err());
        assert!(
            RankedCoSignContextDocument::new(vec![
                b'x';
                robin_engine::multiplayer::MAX_RANKED_WIRE_DOCUMENT_BYTES
                    + 1
            ])
            .is_err()
        );
        let zero_identity = NetMsg::Hello {
            protocol_version: NET_PROTOCOL_VERSION,
            nickname: "alice".to_string(),
            browser_auth: None,
            ranked_public_key: Some([0; 32]),
        };
        assert!(decode_msg(&encode_msg(&zero_identity)).is_err());
    }

    #[test]
    fn ranked_join_gate_handles_both_arrival_orders_and_requires_exact_ack() {
        let fixture = ranked_join_fixture();
        let state = ClientRankedJoinState::default();
        assert_eq!(
            state
                .arm_expected_session(fixture.expected.clone())
                .unwrap(),
            None
        );
        assert_eq!(
            state
                .receive_wire_challenge(fixture.challenge.clone())
                .unwrap(),
            Some(fixture.challenge.clone())
        );
        assert!(!state.is_accepted().unwrap());
        state.authorize_response(&fixture.response).unwrap();
        assert!(!state.is_accepted().unwrap());

        let mut wrong_ack = fixture.accepted.clone();
        wrong_ack.session_genesis = ranked_join_fixture().accepted.session_genesis;
        assert!(state.receive_wire_acceptance(wrong_ack).is_err());
        let mut missing_local = fixture.accepted.clone();
        let host_only = fixture
            .host
            .participant_claims()
            .into_iter()
            .filter(|claim| claim.seat == 0)
            .collect::<Vec<_>>();
        missing_local.participant_roster =
            RankedParticipantRosterDocument::new(encode_ranked_wire_document(&host_only).unwrap())
                .unwrap();
        assert!(state.receive_wire_acceptance(missing_local).is_err());
        assert_eq!(
            state
                .receive_wire_acceptance(fixture.accepted.clone())
                .unwrap(),
            fixture.accepted
        );
        assert!(state.is_accepted().unwrap());
        assert!(
            state
                .receive_wire_acceptance(fixture.accepted.clone())
                .is_err()
        );

        let wire_first = ranked_join_fixture();
        let state = ClientRankedJoinState::default();
        assert_eq!(
            state
                .receive_wire_challenge(wire_first.challenge.clone())
                .unwrap(),
            None
        );
        assert_eq!(
            state.arm_expected_session(wire_first.expected).unwrap(),
            Some(wire_first.challenge)
        );
    }

    #[test]
    fn ranked_roster_updates_are_complete_and_strictly_monotonic() {
        let mut fixture = ranked_join_fixture();
        let initial_roster = fixture.accepted.participant_roster.clone();
        let state = ClientRankedJoinState::default();
        state
            .arm_expected_session(fixture.expected.clone())
            .unwrap();
        state
            .receive_wire_challenge(fixture.challenge.clone())
            .unwrap();
        state.authorize_response(&fixture.response).unwrap();
        state
            .receive_wire_acceptance(fixture.accepted.clone())
            .unwrap();

        let second_guest = iroh::SecretKey::generate();
        let second_claim = fixture
            .host
            .prepare_join(
                2,
                PublicKey32::from_bytes(*second_guest.public().as_bytes()),
                PublicKey32::from_bytes(*second_guest.public().as_bytes()),
                fixture.host.genesis().claim.host_public_key,
            )
            .unwrap();
        let second_attestation =
            sign_named_seat_join(&ranked_signing_key(&second_guest), second_claim).unwrap();
        fixture
            .host
            .admit_join(
                second_attestation,
                *second_guest.public().as_bytes(),
                ParticipantPublicDisclosureV1::NamedProfile,
            )
            .unwrap();
        let expanded = RankedParticipantRosterDocument::new(
            encode_ranked_wire_document(&fixture.host.participant_claims()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            state.receive_wire_roster_update(expanded.clone()).unwrap(),
            expanded
        );
        assert!(state.receive_wire_roster_update(expanded).is_err());
        assert!(state.receive_wire_roster_update(initial_roster).is_err());
    }

    #[test]
    fn ranked_join_gate_rejects_mismatch_replay_and_supports_fresh_reconnect() {
        let mut fixture = ranked_join_fixture();
        let state = ClientRankedJoinState::default();
        state
            .arm_expected_session(fixture.expected.clone())
            .unwrap();

        let mut mismatch = ranked_config();
        mismatch.simulation_seed = SimulationSeed64::new(99);
        let mismatch =
            RankedSessionConfigDocument::new(encode_ranked_wire_document(&mismatch).unwrap())
                .unwrap();
        let mismatch_state = ClientRankedJoinState::default();
        mismatch_state.arm_expected_session(mismatch).unwrap();
        assert!(
            mismatch_state
                .receive_wire_challenge(fixture.challenge.clone())
                .unwrap_err()
                .contains("does not match")
        );

        state
            .receive_wire_challenge(fixture.challenge.clone())
            .unwrap();
        assert!(
            state
                .receive_wire_challenge(fixture.challenge.clone())
                .unwrap_err()
                .contains("replayed or replaced")
        );
        state.authorize_response(&fixture.response).unwrap();
        state
            .receive_wire_acceptance(fixture.accepted.clone())
            .unwrap();

        let second_guest = iroh::SecretKey::generate();
        let second_claim = fixture
            .host
            .prepare_join(
                2,
                PublicKey32::from_bytes(*second_guest.public().as_bytes()),
                PublicKey32::from_bytes(*second_guest.public().as_bytes()),
                fixture.host.genesis().claim.host_public_key,
            )
            .unwrap();
        let second_attestation =
            sign_named_seat_join(&ranked_signing_key(&second_guest), second_claim).unwrap();
        fixture
            .host
            .admit_join(
                second_attestation,
                *second_guest.public().as_bytes(),
                ParticipantPublicDisclosureV1::NamedProfile,
            )
            .unwrap();
        let expanded_roster = RankedParticipantRosterDocument::new(
            encode_ranked_wire_document(&fixture.host.participant_claims()).unwrap(),
        )
        .unwrap();

        fixture.host.observe_disconnect(1).unwrap();
        let reconnect_transport = iroh::SecretKey::generate();
        let reconnect_claim = fixture
            .host
            .prepare_reconnect(
                1,
                PublicKey32::from_bytes(*fixture.guest_key.public().as_bytes()),
                PublicKey32::from_bytes(*reconnect_transport.public().as_bytes()),
                fixture.host.genesis().claim.host_public_key,
            )
            .unwrap();
        let reconnect_attestation = sign_named_seat_join(
            &ranked_signing_key(&fixture.guest_key),
            reconnect_claim.clone(),
        )
        .unwrap();
        let reconnect_challenge = RankedJoinChallenge {
            session_genesis: fixture.accepted.session_genesis.clone(),
            join_claim: RankedJoinClaimDocument::new(
                encode_ranked_wire_document(&reconnect_claim).unwrap(),
            )
            .unwrap(),
        };
        let reconnect_response = RankedJoinResponse::Attestation(
            RankedJoinAttestationDocument::new(
                encode_ranked_wire_document(&reconnect_attestation).unwrap(),
            )
            .unwrap(),
        );
        state.begin_reconnect().unwrap();
        assert!(
            state
                .receive_wire_challenge(fixture.challenge)
                .unwrap_err()
                .contains("earlier stream")
        );
        assert_eq!(
            state
                .receive_wire_challenge(reconnect_challenge.clone())
                .unwrap(),
            Some(reconnect_challenge.clone())
        );
        state.authorize_response(&reconnect_response).unwrap();
        state.begin_reconnect().unwrap();
        assert_eq!(
            state
                .receive_wire_challenge(reconnect_challenge.clone())
                .unwrap(),
            Some(reconnect_challenge)
        );
        state.authorize_response(&reconnect_response).unwrap();
        let RankedJoinResponse::Attestation(reconnect_attestation_document) = reconnect_response
        else {
            unreachable!()
        };
        let reconnect_accepted = RankedJoinAccepted {
            session_genesis: fixture.accepted.session_genesis,
            join_attestation: reconnect_attestation_document,
            participant_roster: fixture.accepted.participant_roster,
        };
        let mut substituted_roster = reconnect_accepted.clone();
        substituted_roster.participant_roster = expanded_roster;
        assert!(
            state
                .receive_wire_acceptance(substituted_roster)
                .unwrap_err()
                .contains("changed the immutable participant roster")
        );
        state.receive_wire_acceptance(reconnect_accepted).unwrap();
        assert!(state.is_accepted().unwrap());
    }

    #[test]
    fn ranked_browse_only_downgrade_is_irreversible() {
        let fixture = ranked_join_fixture();
        let state = ClientRankedJoinState::default();
        assert!(
            state
                .mark_browse_only(RankedBrowseOnlyReason::PeerIdentityUnavailable)
                .unwrap()
        );
        assert!(
            !state
                .mark_browse_only(RankedBrowseOnlyReason::RankedProtocolViolation)
                .unwrap()
        );
        assert!(state.arm_expected_session(fixture.expected).is_err());
        assert!(state.receive_wire_challenge(fixture.challenge).is_err());
        assert!(state.begin_reconnect().is_err());
    }

    #[test]
    fn ranked_port_is_cloneable_role_restricted_typed_and_bounded() {
        let (unattached, _incoming, _outgoing, _cursor, _snapshot) = NetChannels::new();
        assert!(unattached.ranked_port().is_err());

        let (channels, _connect_string) = start_owned_server();
        let host = channels.ranked_port().unwrap();
        assert_eq!(host.role(), RankedMultiplayerRole::Host);
        assert_eq!(host.local_seat(), PlayerId::HOST);
        assert!(
            host.lifecycle()
                .lock()
                .unwrap()
                .is_awaiting_prepared_inputs()
        );
        assert_eq!(host.clone().local_seat(), PlayerId::HOST);

        let request = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 91);
        assert!(host.client_arm_co_sign_request(request).is_err());

        let response = signed_response(&request, &iroh::SecretKey::generate());
        channels
            .defer_leaderboard_cosign_event(NetEvent::LeaderboardCoSignResponse {
                from: PlayerId(1),
                response: response.clone(),
            })
            .unwrap();
        assert!(matches!(
            host.try_recv_authorization_event().unwrap(),
            Some(RankedAuthorizationEvent::CoSignResponse { from, response: actual })
                if from == PlayerId(1) && actual == response
        ));

        channels
            .defer_leaderboard_cosign_event(NetEvent::RankedCoSignContext(
                RankedCoSignContextDocument::new(b"{}".to_vec()).unwrap(),
            ))
            .unwrap();
        assert!(host.try_recv_authorization_event().is_err());
    }

    #[test]
    fn ranked_port_binds_authorization_identity_to_authenticated_seat() {
        let fixture = ranked_join_fixture();
        let guest_public_key = PublicKey32::from_bytes(*fixture.guest_key.public().as_bytes());
        let other_public_key = PublicKey32::from_bytes([0x55; 32]);
        let lifecycle = std::sync::Arc::new(std::sync::Mutex::new(RankedSessionLifecycle::ranked(
            fixture.host,
        )));
        let host = RankedMultiplayerPort {
            role: RankedMultiplayerRole::Host,
            local_seat: PlayerId::HOST,
            lifecycle: std::sync::Arc::clone(&lifecycle),
            outgoing: channel().0,
            authorization_inbox: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::new(),
            )),
            authenticated_seats: vec![(PlayerId(1), guest_public_key)],
            preflight_lobby: None,
            local_public_key: None,
            authenticated_host_public_key: None,
        };

        host.validate_authenticated_remote_identity(PlayerId(1), guest_public_key)
            .unwrap();
        assert!(
            host.validate_authenticated_remote_identity(PlayerId(1), other_public_key)
                .is_err()
        );
        assert!(
            host.validate_authenticated_remote_identity(PlayerId(2), guest_public_key)
                .is_err()
        );
        assert!(
            host.validate_authenticated_remote_identity(PlayerId::HOST, guest_public_key)
                .is_err()
        );

        let (client_outgoing, _client_outgoing_rx) = channel();
        let client = RankedMultiplayerPort {
            role: RankedMultiplayerRole::Client,
            local_seat: PlayerId(1),
            lifecycle,
            outgoing: client_outgoing,
            authorization_inbox: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::new(),
            )),
            authenticated_seats: Vec::new(),
            preflight_lobby: None,
            local_public_key: Some(guest_public_key),
            authenticated_host_public_key: None,
        };
        let signature = robin_run_protocol::Signature64::from_bytes([0x66; 64]);
        assert!(
            client
                .client_respond_continuation_preflight(ParticipantSignatureV1 {
                    public_key: other_public_key,
                    signature,
                })
                .is_err()
        );
        client
            .client_respond_continuation_preflight(ParticipantSignatureV1 {
                public_key: guest_public_key,
                signature,
            })
            .unwrap();
    }

    #[test]
    fn ranked_submission_acceptance_targets_exact_admitted_controller() {
        let fixture = ranked_join_fixture();
        let controller_public_key = PublicKey32::from_bytes(*fixture.guest_key.public().as_bytes());
        let lifecycle = std::sync::Arc::new(std::sync::Mutex::new(RankedSessionLifecycle::ranked(
            fixture.host,
        )));
        let (outgoing, outgoing_rx) = channel();
        let inbox = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        let host = RankedMultiplayerPort {
            role: RankedMultiplayerRole::Host,
            local_seat: PlayerId::HOST,
            lifecycle: std::sync::Arc::clone(&lifecycle),
            outgoing,
            authorization_inbox: std::sync::Arc::clone(&inbox),
            authenticated_seats: Vec::new(),
            preflight_lobby: None,
            local_public_key: None,
            authenticated_host_public_key: None,
        };
        let accepted = submission_accepted();
        assert_eq!(
            host.host_publish_submission_accepted(controller_public_key, accepted.clone())
                .unwrap(),
            PlayerId(1)
        );
        let wire_document = match outgoing_rx.recv().unwrap() {
            NetOutbound::RankedSubmissionAccepted { to, accepted } => {
                assert_eq!(to, PlayerId(1));
                accepted
            }
            other => panic!("unexpected ranked outbound: {other:?}"),
        };

        inbox
            .lock()
            .unwrap()
            .push_back(NetEvent::RankedSubmissionAccepted(wire_document));
        let client = RankedMultiplayerPort {
            role: RankedMultiplayerRole::Client,
            local_seat: PlayerId(1),
            lifecycle,
            outgoing: channel().0,
            authorization_inbox: inbox,
            authenticated_seats: Vec::new(),
            preflight_lobby: None,
            local_public_key: None,
            authenticated_host_public_key: None,
        };
        assert!(matches!(
            client.try_recv_authorization_event().unwrap(),
            Some(RankedAuthorizationEvent::SubmissionAccepted(actual)) if actual == accepted
        ));
        assert!(
            host.host_publish_submission_accepted(PublicKey32::default(), submission_accepted())
                .is_err()
        );
        assert!(
            host.host_publish_submission_accepted(
                PublicKey32::from_bytes([0x55; 32]),
                submission_accepted(),
            )
            .is_err()
        );
    }

    #[test]
    fn ranked_cosign_port_rejects_unadmitted_or_ambiguous_targets() {
        let fixture = ranked_join_fixture();
        let claims = fixture.host.participant_claims();
        assert_eq!(
            require_admitted_remote_ranked_claim(&claims, PlayerId(1))
                .unwrap()
                .seat,
            1
        );
        assert!(require_admitted_remote_ranked_claim(&claims, PlayerId::HOST).is_err());
        assert!(require_admitted_remote_ranked_claim(&claims, PlayerId(2)).is_err());

        let mut duplicate = claims.clone();
        duplicate.push(claims.iter().find(|claim| claim.seat == 1).unwrap().clone());
        assert!(require_admitted_remote_ranked_claim(&duplicate, PlayerId(1)).is_err());

        let mut zero_identity = claims;
        zero_identity
            .iter_mut()
            .find(|claim| claim.seat == 1)
            .unwrap()
            .public_key = PublicKey32::default();
        assert!(require_admitted_remote_ranked_claim(&zero_identity, PlayerId(1)).is_err());
    }

    #[test]
    fn client_cosign_gate_handles_both_arrival_orders_and_consumes_once() {
        let state = ClientLeaderboardCoSignState::default();
        let continuation =
            leaderboard_request(LeaderboardCoSignPurposeV1::CampaignContinuation, 40);
        assert_eq!(state.receive_wire_request(continuation).unwrap(), None);
        assert_eq!(state.arm_request(continuation).unwrap(), Some(continuation));
        state
            .authorize_response(&signed_response(
                &continuation,
                &iroh::SecretKey::generate(),
            ))
            .unwrap();
        assert!(
            state
                .receive_wire_request(continuation)
                .unwrap_err()
                .contains("duplicate")
        );

        let submission = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 41);
        assert_eq!(state.arm_request(submission).unwrap(), None);
        assert_eq!(
            state.receive_wire_request(submission).unwrap(),
            Some(submission)
        );
        let response = signed_response(&submission, &iroh::SecretKey::generate());
        state.authorize_response(&response).unwrap();
        assert!(
            state
                .authorize_response(&response)
                .unwrap_err()
                .contains("duplicate")
        );
    }

    #[test]
    fn client_cosign_gate_rejects_wrong_session_offer_digest_and_purpose() {
        let expected = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 50);
        let mut wrong_offer = expected;
        wrong_offer.instance.submission_offer_sha256 = Digest32::from_bytes([97; 32]);
        let mut wrong_digest = expected;
        wrong_digest.run_digest = Digest32::from_bytes([99; 32]);
        let mut wrong_purpose = expected;
        wrong_purpose.instance.purpose = LeaderboardCoSignPurposeV1::CampaignContinuation;
        for wrong in [wrong_offer, wrong_digest, wrong_purpose] {
            let state = ClientLeaderboardCoSignState::default();
            state.arm_request(expected).unwrap();
            let error = state.receive_wire_request(wrong).unwrap_err();
            assert!(error.contains("does not equal"));
        }

        let mut wrong_session = expected;
        wrong_session.instance.replay_session_id = Digest32::from_bytes([98; 32]);
        let state = ClientLeaderboardCoSignState::default();
        state.arm_request(expected).unwrap();
        assert!(
            state
                .receive_wire_request(wrong_session)
                .unwrap_err()
                .contains("does not equal")
        );
        assert_eq!(
            state.receive_wire_request(expected).unwrap(),
            Some(expected)
        );
    }

    #[test]
    fn exact_signature_domain_rejects_cross_purpose_and_digest_substitution() {
        let key = iroh::SecretKey::generate();
        let request = leaderboard_request(LeaderboardCoSignPurposeV1::CampaignContinuation, 60);
        let signed = signed_response(&request, &key);
        verify_leaderboard_cosign_response(&request, &signed).unwrap();

        let mut wrong_digest = request;
        wrong_digest.run_digest = Digest32::from_bytes([61; 32]);
        let mut wrong_digest_response = signed.clone();
        wrong_digest_response.instance = wrong_digest.instance;
        assert!(verify_leaderboard_cosign_response(&wrong_digest, &wrong_digest_response).is_err());

        let mut wrong_purpose = request;
        wrong_purpose.instance.purpose = LeaderboardCoSignPurposeV1::Submission;
        let mut wrong_purpose_response = signed;
        wrong_purpose_response.instance = wrong_purpose.instance;
        assert!(
            verify_leaderboard_cosign_response(&wrong_purpose, &wrong_purpose_response).is_err()
        );
    }

    #[test]
    fn dropping_owned_runtime_joins_workers() {
        let (channels, _connect_string) = start_owned_server();
        let (done_tx, done_rx) = channel();

        let shutdown_thread = std::thread::spawn(move || {
            drop(channels);
            done_tx.send(()).expect("report completed shutdown");
        });

        done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("runtime drop must close the endpoint and join its workers");
        shutdown_thread
            .join()
            .expect("runtime shutdown test worker must not panic");
    }

    #[test]
    fn server_client_input_roundtrip() {
        // Server side.
        let (server_in_tx, server_in_rx) = channel::<NetEvent>();
        let (_server_out_tx, server_out_rx) = channel::<NetOutbound>();
        let server_cursor = new_frame_cursor();
        let server_snapshot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let expected_config = robin_engine::engine::SimConfig {
            amount_of_speaking: 9,
            ..Default::default()
        };
        let _server = start_server_with_key(
            iroh::SecretKey::generate(),
            "host".into(),
            "Dem_Lei_MP".into(),
            42,
            expected_config,
            Some("en-US".into()),
            server_in_tx,
            server_out_rx,
            server_cursor,
            server_snapshot,
            2,
        )
        .expect("start_server");
        _server
            .install_ranked_session_setup(None)
            .expect("select unranked multiplayer for the transport smoke test");
        let addr = _server.connect_string();

        // Client side.
        let (client_in_tx, client_in_rx) = channel::<NetEvent>();
        let (client_out_tx, client_out_rx) = channel::<NetOutbound>();
        let _client = connect_client(&addr, "alice".into(), client_in_tx, client_out_rx)
            .expect("connect_client");
        assert_eq!(_client.mission_id().as_deref(), Some("Dem_Lei_MP"));
        assert_eq!(_client.mission_seed(), Some(42));
        assert_eq!(_client.mission_sim_config(), Some(expected_config));
        assert_eq!(_client.speech_timing_locale().as_deref(), Some("en-US"));
        let session = _client
            .session_metadata()
            .expect("complete Welcome publication");
        assert_eq!(session.seat, PlayerId(1));
        assert_eq!(session.mission_id, "Dem_Lei_MP");
        assert_eq!(session.mission_seed, 42);
        assert_eq!(session.sim_config, expected_config);
        assert_eq!(session.speech_timing_locale.as_deref(), Some("en-US"));
        assert_eq!(Some(session.session_id), _client.session_id());
        assert!(session.admitted_content.is_none());

        let assigned = loop {
            match client_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::AssignedLocalSeat(p)) => break p,
                Ok(NetEvent::Note(_)) => continue,
                Ok(other) => panic!("unexpected pre-handshake event {other:?}"),
                Err(e) => panic!("timeout waiting for AssignedLocalSeat: {e}"),
            }
        };
        assert_eq!(assigned, PlayerId(1));

        let mut saw_join = false;
        for _ in 0..16 {
            match server_in_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(NetEvent::Input { input, .. }) => {
                    if let PlayerCommand::ConnectSeat {
                        player_id,
                        ref nickname,
                        ..
                    } = input.command
                        && player_id == PlayerId(1)
                        && nickname == "alice"
                    {
                        saw_join = true;
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert!(
            saw_join,
            "server should have folded a ConnectSeat for the new client"
        );

        client_out_tx
            .send(NetOutbound::Input {
                origin_frame: 0,
                command: PlayerCommand::CrouchDown,
            })
            .unwrap();

        let (server_input, server_target) = loop {
            match server_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::Input {
                    input,
                    target_frame,
                    ..
                }) if matches!(input.command, PlayerCommand::CrouchDown) => {
                    break (input, target_frame);
                }
                Ok(_) => continue,
                Err(e) => panic!("timeout waiting for server-side input echo: {e}"),
            }
        };
        assert_eq!(server_input.player_id, PlayerId(1));
        assert_eq!(server_target, INPUT_DELAY_FRAMES);

        let client_seen = loop {
            match client_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::Input { input, .. })
                    if matches!(input.command, PlayerCommand::CrouchDown) =>
                {
                    break input;
                }
                Ok(_) => continue,
                Err(e) => panic!("timeout waiting for client-side input echo: {e}"),
            }
        };
        assert_eq!(client_seen.player_id, PlayerId(1));

        let _ = (PlayerInput::new(PlayerId(0), PlayerCommand::CrouchDown),);
    }

    #[test]
    fn unranked_transport_never_exposes_a_cosign_request() {
        let (server_in_tx, _server_in_rx) = channel::<NetEvent>();
        let (server_out_tx, server_out_rx) = channel::<NetOutbound>();
        let server_cursor = new_frame_cursor();
        let server_snapshot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let _server = start_server_with_key(
            iroh::SecretKey::generate(),
            "host".into(),
            "Dem_Lei_MP".into(),
            42,
            robin_engine::engine::SimConfig::default(),
            Some("en-US".into()),
            server_in_tx,
            server_out_rx,
            server_cursor,
            server_snapshot,
            2,
        )
        .expect("start server");
        _server
            .install_ranked_session_setup(None)
            .expect("select browse-only multiplayer for the rejection test");
        let (client_in_tx, client_in_rx) = channel::<NetEvent>();
        let (client_out_tx, client_out_rx) = channel::<NetOutbound>();
        let _client = connect_client(
            _server.connect_string(),
            "alice".into(),
            client_in_tx,
            client_out_rx,
        )
        .expect("connect client");
        _client
            .install_ranked_session_setup(None)
            .expect("select browse-only client state for the rejection test");

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            assert!(std::time::Instant::now() < deadline, "seat did not connect");
            if matches!(
                client_in_rx.recv_timeout(Duration::from_millis(50)),
                Ok(NetEvent::AssignedLocalSeat(PlayerId(1)))
            ) {
                break;
            }
        }

        let request = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 80);
        server_out_tx
            .send(NetOutbound::LeaderboardCoSignRequest {
                to: PlayerId(1),
                request,
            })
            .unwrap();
        client_out_tx
            .send(NetOutbound::ArmLeaderboardCoSignRequest { request })
            .unwrap();
        assert!(
            !matches!(
                client_in_rx.recv_timeout(Duration::from_millis(250)),
                Ok(NetEvent::LeaderboardCoSignRequest(_))
            ),
            "browse-only peers must never receive a leaderboard signing capability"
        );
    }

    #[test]
    fn host_content_preflight_uses_no_seat_then_exact_resume_reconnects() {
        let package = test_distributed_mod();
        let encoded = package.package.encode().expect("encode test full mod");
        let expected_hash = package.package.manifest.full_mod_sha256;
        let hosted = HostedModContent::from_encoded(encoded.clone()).expect("hosted content");

        let (server_in_tx, server_in_rx) = channel::<NetEvent>();
        let (_server_out_tx, server_out_rx) = channel::<NetOutbound>();
        let server_key = iroh::SecretKey::generate();
        let server_cursor = new_frame_cursor();
        let server_snapshot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let _server = start_server_with_key_and_content(
            server_key,
            "host".into(),
            "TestMission".into(),
            42,
            robin_engine::engine::SimConfig::default(),
            server_in_tx,
            server_out_rx,
            server_cursor,
            server_snapshot,
            2,
            Some(hosted),
        )
        .expect("start content server");

        let client_key = iroh::SecretKey::generate();
        let (client_in_tx, client_in_rx) = channel::<NetEvent>();
        let (client_out_tx, client_out_rx) = channel::<NetOutbound>();
        let mut client = connect_client_with_key(
            client_key.clone(),
            _server.connect_string(),
            "alice".into(),
            client_in_tx,
            client_out_rx,
        )
        .expect("connect through content prelude");
        let offer = client
            .content_offer()
            .expect("content offer before Welcome");
        assert_eq!(offer.full_mod_sha256, expected_hash);
        assert!(client.mission_id().is_none());
        assert!(matches!(
            client_in_rx.recv_timeout(Duration::from_secs(2)),
            Ok(NetEvent::ContentOffer(seen)) if seen == offer
        ));

        client_out_tx
            .send(NetOutbound::ContentRequest {
                full_mod_sha256: expected_hash,
                resume_offset: 0,
            })
            .expect("accept exact content");
        let mut downloaded = Vec::new();
        while downloaded.len() < encoded.len() {
            match client_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::ContentChunk {
                    full_mod_sha256,
                    offset,
                    total_bytes,
                    bytes,
                }) => {
                    assert_eq!(full_mod_sha256, expected_hash);
                    assert_eq!(offset as usize, downloaded.len());
                    assert_eq!(total_bytes as usize, encoded.len());
                    downloaded.extend_from_slice(&bytes);
                }
                Ok(other) => panic!("unexpected pre-admission event {other:?}"),
                Err(error) => panic!("content transfer timed out: {error}"),
            }
        }
        assert_eq!(downloaded, encoded);
        DistributedModPackage::decode(&downloaded).expect("downloaded exact package validates");
        assert!(client.mission_id().is_none(), "Welcome must still be gated");

        client_out_tx
            .send(NetOutbound::ContentPrepared {
                full_mod_sha256: expected_hash,
            })
            .expect("verified content prepared without a seat");
        assert!(matches!(
            client_in_rx.recv_timeout(Duration::from_secs(2)),
            Ok(NetEvent::Note(note)) if note.contains("without joining a gameplay seat")
        ));
        assert!(client.mission_id().is_none());
        client.shutdown();

        assert!(
            server_in_rx
                .recv_timeout(Duration::from_millis(200))
                .is_err(),
            "content preflight must not allocate or announce a gameplay seat"
        );

        let (join_in_tx, join_in_rx) = channel::<NetEvent>();
        let (join_out_tx, join_out_rx) = channel::<NetOutbound>();
        let join = connect_client_with_key(
            client_key,
            _server.connect_string(),
            "alice".into(),
            join_in_tx,
            join_out_rx,
        )
        .expect("reconnect after exact content preflight");
        assert!(matches!(
            join_in_rx.recv_timeout(Duration::from_secs(2)),
            Ok(NetEvent::ContentOffer(seen)) if seen == offer
        ));
        join_out_tx
            .send(NetOutbound::ContentRequest {
                full_mod_sha256: expected_hash,
                resume_offset: encoded.len() as u64,
            })
            .expect("resume from exact complete package");
        join_out_tx
            .send(NetOutbound::ContentReady {
                full_mod_sha256: expected_hash,
            })
            .expect("exact mounted content ready");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while join.mission_id().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(join.mission_id().as_deref(), Some("TestMission"));
        assert!(matches!(
            join_in_rx.recv_timeout(Duration::from_secs(2)),
            Ok(NetEvent::AssignedLocalSeat(PlayerId(1))) | Ok(NetEvent::MissionConfig { .. })
        ));
    }

    fn test_distributed_mod() -> crate::distributed_mod::ValidatedDistributedMod {
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

        let cursor = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(cursor);
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
        let mission_archive = zip.finish().unwrap().into_inner();
        DistributedModPackage::build(
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
            mission_archive,
            None,
        )
        .expect("build test full mod")
    }

    #[test]
    fn server_releases_begin_only_after_snapshot_and_both_ready_messages() {
        let (server_in_tx, server_in_rx) = channel::<NetEvent>();
        let (server_out_tx, server_out_rx) = channel::<NetOutbound>();
        let server_cursor = new_frame_cursor();
        let server_snapshot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let _server = start_server_with_key(
            iroh::SecretKey::generate(),
            "host".into(),
            "Dem_Lei_MP".into(),
            42,
            robin_engine::engine::SimConfig::default(),
            Some("en-US".into()),
            server_in_tx,
            server_out_rx,
            server_cursor,
            server_snapshot,
            2,
        )
        .expect("start_server");
        let (client_in_tx, client_in_rx) = channel::<NetEvent>();
        let (client_out_tx, client_out_rx) = channel::<NetOutbound>();
        let _client = connect_client(
            _server.connect_string(),
            "alice".into(),
            client_in_tx,
            client_out_rx,
        )
        .expect("connect_client");

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "client did not receive seat assignment"
            );
            if matches!(
                client_in_rx.recv_timeout(Duration::from_millis(50)),
                Ok(NetEvent::AssignedLocalSeat(PlayerId(1)))
            ) {
                break;
            }
        }

        server_out_tx
            .send(NetOutbound::InitialSnapshot {
                frame: 0,
                engine_bytes: vec![1, 2, 3, 4],
            })
            .expect("publish snapshot");
        let snapshot_deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            assert!(
                std::time::Instant::now() < snapshot_deadline,
                "joining peer did not receive initial snapshot"
            );
            match client_in_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(NetEvent::InitialSnapshot {
                    frame: 0,
                    engine_bytes,
                }) => {
                    assert_eq!(engine_bytes, [1, 2, 3, 4]);
                    break;
                }
                Ok(NetEvent::BeginSim { .. }) => {
                    panic!("BeginSim arrived before the joining peer was ready")
                }
                Ok(_) | Err(_) => {}
            }
        }

        client_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .expect("peer ready");
        let no_begin_deadline = std::time::Instant::now() + Duration::from_millis(150);
        while std::time::Instant::now() < no_begin_deadline {
            if matches!(
                client_in_rx.recv_timeout(Duration::from_millis(20)),
                Ok(NetEvent::BeginSim { .. })
            ) {
                panic!("BeginSim arrived before the delayed host readiness");
            }
        }

        server_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .expect("host ready");
        let server_begin = loop {
            match server_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                }) => break (frame, start_epoch_ms),
                Ok(_) => continue,
                Err(error) => panic!("host did not receive BeginSim: {error}"),
            }
        };
        let client_begin = loop {
            match client_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                }) => break (frame, start_epoch_ms),
                Ok(_) => continue,
                Err(error) => panic!("peer did not receive BeginSim: {error}"),
            }
        };

        assert_eq!(server_begin, client_begin);
        assert_eq!(server_begin.0, 0);
    }
}
