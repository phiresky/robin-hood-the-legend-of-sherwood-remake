//! Restricted mission-end ranked authorization capabilities.
use super::*;

/// Capability lane held by a mission-end leaderboard controller.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum RankedMultiplayerRole {
    Host,
    Client,
}

pub(super) fn require_admitted_remote_ranked_claim(
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
    pub(super) role: RankedMultiplayerRole,
    pub(super) local_seat: PlayerId,
    pub(super) lifecycle: SharedRankedSessionLifecycle,
    pub(super) outgoing: Sender<NetOutbound>,
    pub(super) authorization_inbox: LeaderboardAuthorizationInbox,
    pub(super) authenticated_seats: Vec<(PlayerId, PublicKey32)>,
    pub(super) preflight_lobby: Option<RankedPreflightLobbyV1>,
    pub(super) local_public_key: Option<PublicKey32>,
    pub(super) authenticated_host_public_key: Option<PublicKey32>,
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
