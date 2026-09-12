//! Multiplayer transport — iroh (peer-to-peer QUIC) server / client.
//!
//! The wire-format types ([`robin_engine::multiplayer::NetMsg`], [`NetEvent`], [`NetOutbound`])
//! and protocol constants live in
//! [`robin_engine::multiplayer`] so [`robin_engine::engine_manager::EngineManager`]
//! can route mutations through the rollback-safe path. This module wraps
//! the engine channel bundle in [`NetChannels`] so the channels and their
//! platform-specific [`MultiplayerRuntime`] have one owner and one lifetime.

mod clock;

#[cfg(feature = "multiplayer")]
mod client_gameplay;
#[cfg(feature = "multiplayer")]
mod client_outgoing;
#[cfg(feature = "multiplayer")]
mod client_protocol;
#[cfg(feature = "multiplayer")]
mod content_transfer;
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

#[cfg(feature = "multiplayer")]
mod framing;
#[cfg(feature = "multiplayer")]
pub(crate) use framing::*;

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
    ClientHandle, HostedModContent, MultiplayerCampaignSession, ServerConfig, ServerHandle,
    connect_client, connect_client_in_campaign, start_server_in_campaign,
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

mod ranked_port;
#[cfg(all(test, feature = "multiplayer", not(target_arch = "wasm32")))]
use ranked_port::require_admitted_remote_ranked_claim;
pub(crate) use ranked_port::{
    RankedAuthorizationEvent, RankedMultiplayerPort, RankedMultiplayerRole,
};

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
    /// returned by [`start_server_in_campaign`] or [`connect_client`] before publishing the
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
        assert_eq!(NET_PROTOCOL_VERSION, 48);
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
        let session = _client
            .session_metadata()
            .expect("complete Welcome publication");
        assert_eq!(session.seat, PlayerId(1));
        assert_eq!(session.mission_id, "Dem_Lei_MP");
        assert_eq!(session.mission_seed, 42);
        assert_eq!(session.sim_config, expected_config);
        assert_eq!(session.speech_timing_locale.as_deref(), Some("en-US"));
        assert_eq!(session.session_id, _server.session_id());
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
        assert!(client.session_metadata().is_none());
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
        assert!(
            client.session_metadata().is_none(),
            "Welcome must still be gated"
        );

        client_out_tx
            .send(NetOutbound::ContentPrepared {
                full_mod_sha256: expected_hash,
            })
            .expect("verified content prepared without a seat");
        assert!(matches!(
            client_in_rx.recv_timeout(Duration::from_secs(2)),
            Ok(NetEvent::Note(note)) if note.contains("without joining a gameplay seat")
        ));
        assert!(client.session_metadata().is_none());
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
        while join.session_metadata().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            join.session_metadata()
                .expect("admitted Welcome")
                .mission_id,
            "TestMission"
        );
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
