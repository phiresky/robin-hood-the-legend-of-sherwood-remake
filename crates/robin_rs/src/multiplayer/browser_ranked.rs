//! Browser ranked admission for the shared client session.
//!
//! The durable browser identity lives behind the stable shell's isolated JS
//! signer, so the named-seat claim is signed asynchronously, and the locally
//! prepared ranked setup is awaited when the host challenge arrives.
//! [`BrowserAdmissionPhase`] additionally gates `ReadyToSim` until ranking is
//! resolved and rejects Welcomes that race ahead of admission.
//!
//! Compiled into native test builds as well, so the shared session policy tests
//! exercise this adapter without a browser.

use super::client_outgoing::ClientPublicationAuthority;
use super::client_session::{
    ClientRankedAdmission, ClientTimer, RankedResponses, SessionLinks, with_timeout,
};
use super::ranked_client::{BrowserAdmissionPhase, ClientRankedJoinState, ranked_lifecycle_lock};
use super::{
    MessageError, MultiplayerError, NetEvent, NetMsg, RankedBrowseOnlyReason,
    RankedJoinAttestationDocument, RankedJoinChallenge, RankedJoinResponse,
    RankedJoinUnavailableReason, RankedSessionConfigDocument, SharedClientRankedJoinState,
};
use crate::leaderboard_ranked_session::{
    OfficialRankedSessionSetupV1, RankedSessionClientAdmissionV1, SharedRankedSessionLifecycle,
};
use iroh::EndpointId;
use robin_engine::multiplayer::{
    BrowserPeerAuth, RankedCoSignContextDocument, RankedJoinAccepted,
    RankedParticipantRosterDocument, RankedSubmissionAcceptedDocument,
};
use robin_engine::player_command::PlayerId;
use robin_run_protocol::{
    CanonicalDocument as _, NamedSeatJoinClaimV1, PublicKey32, ReplaySessionGenesisV1,
};
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::time::Duration;

// Mission authority is fetched and compared while level resources load. Slow
// browsers and cold HTTP caches routinely exceed a few seconds; retain a
// bounded failure path without racing normal bootstrap into browse-only.
const RANKED_SETUP_TIMEOUT: Duration = Duration::from_secs(120);

fn lifecycle_poisoned() -> MultiplayerError {
    MultiplayerError::LocalState("browser ranked lifecycle lock is poisoned".into())
}

fn closed(message: &'static str) -> MultiplayerError {
    MultiplayerError::ChannelClosed(message.into())
}

/// Browser-only ranking state that must survive a dropped relay stream. The
/// shared gate owns the exact documents; the cells retain transport facts
/// needed to reject a Welcome that races ahead of admission or changes seats
/// on reconnect.
pub(super) struct BrowserRankedAdmission {
    lifecycle: SharedRankedSessionLifecycle,
    setup_rx: async_channel::Receiver<Option<OfficialRankedSessionSetupV1>>,
    browser_auth: BrowserPeerAuth,
    transport_endpoint: EndpointId,
    authenticated_host_endpoint: EndpointId,
    join: SharedClientRankedJoinState,
    prepared_setup: RefCell<Option<OfficialRankedSessionSetupV1>>,
    admission: Cell<BrowserAdmissionPhase>,
    welcomed_seat: Cell<Option<PlayerId>>,
    admitted_seat: Cell<Option<PlayerId>>,
    last_admitted_claim: RefCell<Option<NamedSeatJoinClaimV1>>,
}

impl BrowserRankedAdmission {
    pub(super) fn new(
        lifecycle: SharedRankedSessionLifecycle,
        setup_rx: async_channel::Receiver<Option<OfficialRankedSessionSetupV1>>,
        browser_auth: BrowserPeerAuth,
        transport_endpoint: EndpointId,
        authenticated_host_endpoint: EndpointId,
    ) -> Self {
        Self {
            lifecycle,
            setup_rx,
            browser_auth,
            transport_endpoint,
            authenticated_host_endpoint,
            join: Arc::new(Default::default()),
            prepared_setup: RefCell::new(None),
            admission: Cell::new(BrowserAdmissionPhase::default()),
            welcomed_seat: Cell::new(None),
            admitted_seat: Cell::new(None),
            last_admitted_claim: RefCell::new(None),
        }
    }

    /// `ReadyToSim` is held back until ranking is admitted or browse-only.
    pub(super) fn admission_resolved(&self) -> bool {
        self.admission.get().resolved()
    }

    /// Decide the response to a host challenge and the admission phase that
    /// follows once it is queued.
    async fn answer_challenge<Tm: ClientTimer>(
        &self,
        challenge: RankedJoinChallenge,
        incoming_tx: &std::sync::mpsc::Sender<NetEvent>,
    ) -> Result<(RankedJoinResponse, BrowserAdmissionPhase), MultiplayerError> {
        if self.admission.get().pending() {
            return Err(MultiplayerError::Ranked(
                "host replayed or replaced a pending ranked join challenge".into(),
            ));
        }

        // Stage the independently authenticated host value first. On the initial
        // stream this permits an exact local mismatch to be consumed by the closed
        // Unavailable response. Reconnects retain the expected configuration and
        // release only a fresh, matching challenge immediately.
        let already_released = self.join.receive_wire_challenge(challenge.clone())?;

        let prepared = self.prepared_setup.borrow().clone();
        let local_setup = if let Some(setup) = prepared {
            setup
        } else {
            let setup =
                match with_timeout::<Tm, _>(RANKED_SETUP_TIMEOUT, self.setup_rx.recv()).await {
                    Ok(Ok(setup)) => setup,
                    Ok(Err(_)) | Err(()) => None,
                };
            let Some(setup) = setup else {
                downgrade_ranked_lifecycle(
                    &self.lifecycle,
                    "browser ranked setup was explicitly unavailable",
                )?;
                return Ok(unavailable(
                    RankedJoinUnavailableReason::LocalRankedSessionUnavailable,
                ));
            };
            let admission = RankedSessionClientAdmissionV1::new_official(
                setup.clone(),
                PublicKey32::from_bytes(*self.authenticated_host_endpoint.as_bytes()),
                PublicKey32::from_bytes(self.browser_auth.durable_public_key),
                PublicKey32::from_bytes(*self.transport_endpoint.as_bytes()),
            )
            .map_err(|error| {
                MultiplayerError::ranked_document(
                    "prepare authenticated browser ranked admission",
                    error,
                )
            })?;
            self.lifecycle
                .lock()
                .map_err(|_| lifecycle_poisoned())?
                .install_client_admission(admission)
                .map_err(|error| {
                    MultiplayerError::ranked_document(
                        "install authenticated browser ranked admission",
                        error,
                    )
                })?;
            *self.prepared_setup.borrow_mut() = Some(setup.clone());
            setup
        };
        let local_config = &local_setup.ranked_session;

        let released = if let Some(released) = already_released {
            released
        } else {
            let local_document =
                match crate::leaderboard_ranked_session::encode_ranked_wire_document(local_config)
                    .map_err(|error| {
                        MultiplayerError::ranked_document(
                            "encode browser ranked-session configuration",
                            error,
                        )
                    })
                    .and_then(|bytes| {
                        RankedSessionConfigDocument::new(bytes).map_err(|error| {
                            MultiplayerError::ranked_document(
                                "wrap browser ranked-session configuration",
                                MessageError(error.to_owned()),
                            )
                        })
                    }) {
                    Ok(document) => document,
                    Err(error) => {
                        tracing::warn!(%error, "browser ranked setup could not be encoded");
                        downgrade_ranked_lifecycle(
                            &self.lifecycle,
                            "browser ranked setup could not be canonically encoded",
                        )?;
                        return Ok(unavailable(
                            RankedJoinUnavailableReason::LocalRankedSessionUnavailable,
                        ));
                    }
                };
            match self.join.arm_expected_session(local_document) {
                Ok(Some(released)) => released,
                Ok(None) => {
                    return Err(MultiplayerError::LocalState(
                        "ranked join setup did not release its already-staged challenge".into(),
                    ));
                }
                Err(error) => {
                    tracing::warn!(%error, "browser ranked challenge did not match local setup");
                    downgrade_ranked_lifecycle(
                        &self.lifecycle,
                        "host ranked challenge did not match the prepared browser session",
                    )?;
                    return Ok(unavailable(
                        RankedJoinUnavailableReason::LocalRankedSessionMismatch,
                    ));
                }
            }
        };

        let claim = match validate_browser_ranked_challenge(
            &released,
            &local_setup,
            self.authenticated_host_endpoint,
            self.transport_endpoint,
            &self.browser_auth,
            self.welcomed_seat.get(),
            self.last_admitted_claim.borrow().as_ref(),
        ) {
            Ok(claim) => claim,
            Err(error) => {
                tracing::warn!(%error, "browser ranked challenge failed endpoint binding");
                downgrade_ranked_lifecycle(
                    &self.lifecycle,
                    "host ranked challenge failed browser identity binding",
                )?;
                return Ok(unavailable(
                    RankedJoinUnavailableReason::LocalRankedSessionMismatch,
                ));
            }
        };

        incoming_tx
            .send(NetEvent::RankedJoinChallenge(released))
            .map_err(|_| closed("browser ranked challenge channel is closed"))?;
        #[cfg(target_arch = "wasm32")]
        let signed = crate::leaderboard_signing::browser_game_sign_named_seat_join(&claim)
            .await
            .map_err(|error| {
                MultiplayerError::ranked_document("sign browser ranked join claim", error)
            });
        // Native test builds compile this adapter only to exercise the shared
        // session handler; there is no isolated browser signer outside a browser.
        #[cfg(not(target_arch = "wasm32"))]
        let signed: Result<
            robin_run_protocol::NamedSeatJoinAttestationV1,
            MultiplayerError,
        > = Err(MultiplayerError::Unavailable(
            format!(
                "isolated browser ranked admission signer exists only in browser builds (seat {})",
                claim.seat
            )
            .into(),
        ));
        let attestation = match signed {
            Ok(attestation) => attestation,
            Err(error) => {
                tracing::warn!(%error, "isolated browser ranked admission signer unavailable");
                downgrade_ranked_lifecycle(
                    &self.lifecycle,
                    "isolated browser ranked admission signer was unavailable",
                )?;
                return Ok(unavailable(
                    RankedJoinUnavailableReason::AttestationSigningFailed,
                ));
            }
        };
        let attestation_bytes = crate::leaderboard_ranked_session::encode_ranked_wire_document(
            &attestation,
        )
        .map_err(|error| {
            MultiplayerError::ranked_document("encode browser ranked join attestation", error)
        })?;
        let attestation_document =
            RankedJoinAttestationDocument::new(attestation_bytes).map_err(|error| {
                MultiplayerError::ranked_document(
                    "wrap browser ranked join attestation",
                    MessageError(error.to_owned()),
                )
            })?;
        Ok((
            RankedJoinResponse::Attestation(attestation_document),
            BrowserAdmissionPhase::AwaitingRankedDecision,
        ))
    }
}

fn unavailable(reason: RankedJoinUnavailableReason) -> (RankedJoinResponse, BrowserAdmissionPhase) {
    (
        RankedJoinResponse::Unavailable(reason),
        BrowserAdmissionPhase::AwaitingBrowseDecision,
    )
}

fn downgrade_ranked_lifecycle(
    lifecycle: &SharedRankedSessionLifecycle,
    reason: &'static str,
) -> Result<(), MultiplayerError> {
    lifecycle
        .lock()
        .map_err(|_| lifecycle_poisoned())?
        .downgrade(reason);
    Ok(())
}

fn ranked_browse_only_reason(reason: RankedBrowseOnlyReason) -> &'static str {
    match reason {
        RankedBrowseOnlyReason::HostRankedSessionUnavailable => {
            "host ranked session was unavailable"
        }
        RankedBrowseOnlyReason::PeerIdentityUnavailable => "a peer ranked identity was unavailable",
        RankedBrowseOnlyReason::PeerRankedSessionMismatch => "a peer ranked session did not match",
        RankedBrowseOnlyReason::PeerAttestationRejected => "a peer ranked attestation was rejected",
        RankedBrowseOnlyReason::RankedTransportInterrupted => {
            "ranked multiplayer transport was interrupted"
        }
        RankedBrowseOnlyReason::RankedProtocolViolation => "ranked multiplayer protocol violation",
    }
}

fn validate_browser_ranked_challenge(
    challenge: &RankedJoinChallenge,
    expected_setup: &OfficialRankedSessionSetupV1,
    authenticated_host_endpoint: EndpointId,
    authenticated_transport_endpoint: EndpointId,
    browser_auth: &BrowserPeerAuth,
    welcomed_seat: Option<PlayerId>,
    previous_claim: Option<&NamedSeatJoinClaimV1>,
) -> Result<NamedSeatJoinClaimV1, MultiplayerError> {
    let genesis: ReplaySessionGenesisV1 =
        crate::leaderboard_ranked_session::decode_ranked_wire_document(
            challenge.session_genesis.as_bytes(),
        )
        .map_err(|error| {
            MultiplayerError::ranked_document("decode browser ranked session genesis", error)
        })?;
    crate::leaderboard_ranked_session::validate_official_session_genesis(
        &genesis,
        *authenticated_host_endpoint.as_bytes(),
        expected_setup,
    )
    .map_err(|error| {
        MultiplayerError::ranked_document("validate browser ranked session genesis", error)
    })?;
    let claim: NamedSeatJoinClaimV1 =
        crate::leaderboard_ranked_session::decode_ranked_wire_document(
            challenge.join_claim.as_bytes(),
        )
        .map_err(|error| {
            MultiplayerError::ranked_document("decode browser ranked join claim", error)
        })?;
    let expected_genesis = genesis.canonical_digest().map_err(|error| {
        MultiplayerError::ranked_document("digest browser ranked session genesis", error)
    })?;
    let expected_public_key = PublicKey32::from_bytes(browser_auth.durable_public_key);
    let expected_host_endpoint = PublicKey32::from_bytes(*authenticated_host_endpoint.as_bytes());
    let expected_transport_endpoint =
        PublicKey32::from_bytes(*authenticated_transport_endpoint.as_bytes());
    let expected_config = &expected_setup.ranked_session;
    let expected_seat = welcomed_seat.ok_or_else(|| {
        MultiplayerError::Ranked(
            "ranked challenge arrived before authoritative Welcome seat".into(),
        )
    })?;
    if claim.session_genesis_sha256 != expected_genesis
        || claim.public_key != expected_public_key
        || claim.transport_endpoint_id != expected_transport_endpoint
        || claim.host_endpoint_id != expected_host_endpoint
        || claim.host_endpoint_id != genesis.claim.host_public_key
        || claim.replay_session_id != genesis.claim.replay_session_id
        || claim.host_nonce != genesis.claim.host_nonce
        || claim.mission_id != expected_config.mission_id
        || claim.content_manifest_sha256 != expected_config.content_manifest_sha256
        || claim.rules_config_sha256 != expected_config.rules_config_sha256
        || claim.ruleset_manifest_sha256 != expected_config.ruleset_manifest_sha256
        || claim.competition_manifest_sha256 != expected_config.competition_manifest_sha256
        || claim.seat == 0
        || u8::try_from(claim.seat).ok().map(PlayerId) != Some(expected_seat)
    {
        return Err(MultiplayerError::Identity(
            "ranked join claim does not match the durable browser identity, authenticated endpoints, or exact prepared session"
                .into(),
        ));
    }
    match previous_claim {
        None if claim.connection_epoch != 0 => {
            return Err(MultiplayerError::Ranked(
                "initial ranked browser admission has a nonzero connection epoch".into(),
            ));
        }
        Some(previous)
            if claim.seat != previous.seat
                || claim.public_key != previous.public_key
                || claim.participant_instance_id != previous.participant_instance_id
                || previous.connection_epoch.checked_add(1) != Some(claim.connection_epoch)
                || claim.join_event_ordinal <= previous.join_event_ordinal =>
        {
            return Err(MultiplayerError::Ranked(
                "ranked browser reconnect changed its admitted participant or did not advance its lifecycle"
                    .into(),
            ));
        }
        None | Some(_) => {}
    }
    Ok(claim)
}

impl ClientRankedAdmission for BrowserRankedAdmission {
    const LABEL: &'static str = "browser";
    const RESPONSE_QUEUE_CAPACITY: Option<usize> = Some(1);

    fn lifecycle(&self) -> &SharedRankedSessionLifecycle {
        &self.lifecycle
    }

    fn join_state(&self) -> &ClientRankedJoinState {
        &self.join
    }

    fn durable_public_key(&self) -> Option<PublicKey32> {
        Some(PublicKey32::from_bytes(
            self.browser_auth.durable_public_key,
        ))
    }

    fn authenticated_host_public_key(&self) -> PublicKey32 {
        PublicKey32::from_bytes(*self.authenticated_host_endpoint.as_bytes())
    }

    fn welcomed(&self, seat: PlayerId) {
        self.welcomed_seat.set(Some(seat));
    }

    fn simulation_release_unresolved(&self) -> Result<bool, MultiplayerError> {
        Ok(!self.admission.get().resolved())
    }

    fn enter_browse_only(&self, reason: RankedBrowseOnlyReason) -> Result<(), MultiplayerError> {
        self.join.mark_browse_only(reason)?;
        self.admission.set(BrowserAdmissionPhase::BrowseOnly);
        Ok(())
    }

    fn downgrade(&self, reason: RankedBrowseOnlyReason, detail: String) {
        ranked_lifecycle_lock(&self.lifecycle).downgrade(detail.clone());
        tracing::warn!(?reason, %detail, "client ranked admission downgraded; gameplay remains available");
    }

    fn publication_authority(
        &self,
        requires_cosign: bool,
    ) -> Result<ClientPublicationAuthority, MultiplayerError> {
        Ok(ClientPublicationAuthority {
            co_sign_allowed: requires_cosign && self.join.is_accepted()?,
            durable_public_key: self.durable_public_key(),
        })
    }

    async fn on_challenge<Tm: ClientTimer>(
        &self,
        links: &SessionLinks<'_, Self>,
        challenge: RankedJoinChallenge,
    ) -> Result<(), MultiplayerError> {
        let (response, phase) = self
            .answer_challenge::<Tm>(challenge, links.incoming)
            .await?;
        links.responses.queue(&self.join, response)?;
        self.admission.set(phase);
        Ok(())
    }

    fn on_setup(&self, _responses: &RankedResponses, _setup: Option<OfficialRankedSessionSetupV1>) {
        unreachable!(
            "browser ranked setup is awaited by the challenge handler, never delivered to the session writer"
        )
    }

    fn on_accepted(
        &self,
        links: &SessionLinks<'_, Self>,
        accepted: RankedJoinAccepted,
    ) -> Result<(), MultiplayerError> {
        let accepted = self.join.receive_wire_acceptance(accepted)?;
        let genesis: ReplaySessionGenesisV1 =
            crate::leaderboard_ranked_session::decode_ranked_wire_document(
                accepted.session_genesis.as_bytes(),
            )
            .map_err(|error| {
                MultiplayerError::ranked_document("decode accepted browser ranked genesis", error)
            })?;
        let attestation: robin_run_protocol::NamedSeatJoinAttestationV1 =
            crate::leaderboard_ranked_session::decode_ranked_wire_document(
                accepted.join_attestation.as_bytes(),
            )
            .map_err(|error| {
                MultiplayerError::ranked_document(
                    "decode accepted browser ranked join attestation",
                    error,
                )
            })?;
        let roster: Vec<robin_run_protocol::ParticipantClaimV1> =
            serde_json::from_slice(accepted.participant_roster.as_bytes()).map_err(|error| {
                MultiplayerError::ranked_document("decode accepted browser ranked roster", error)
            })?;
        let admitted_seat = u8::try_from(attestation.claim.seat)
            .map(PlayerId)
            .map_err(|_| {
                MultiplayerError::Ranked(
                    "ranked browser seat does not fit the gameplay seat".into(),
                )
            })?;
        {
            let mut lifecycle = self.lifecycle.lock().map_err(|_| lifecycle_poisoned())?;
            if lifecycle.client_admission().is_some() {
                lifecycle
                    .accept_ranked_client(attestation.claim.seat, genesis, roster)
                    .map_err(|error| {
                        MultiplayerError::ranked_document(
                            "accept authenticated browser ranked client",
                            error,
                        )
                    })?;
            } else if lifecycle.ranked_client().is_some() {
                lifecycle
                    .update_ranked_client_roster(&genesis, roster)
                    .map_err(|error| {
                        MultiplayerError::ranked_document(
                            "accept browser ranked reconnect roster",
                            error,
                        )
                    })?;
            } else {
                return Err(MultiplayerError::Ranked(
                    "ranked join acceptance has no pending or admitted browser lifecycle".into(),
                ));
            }
        }
        self.admitted_seat.set(Some(admitted_seat));
        *self.last_admitted_claim.borrow_mut() = Some(attestation.claim);
        self.admission.set(BrowserAdmissionPhase::Ranked);
        links
            .incoming
            .send(NetEvent::RankedJoinAccepted(accepted))
            .map_err(|_| closed("browser ranked admission channel is closed"))
    }

    fn on_roster(
        &self,
        links: &SessionLinks<'_, Self>,
        document: RankedParticipantRosterDocument,
    ) -> Result<(), MultiplayerError> {
        let document = self.join.receive_wire_roster_update(document)?;
        let roster: Vec<robin_run_protocol::ParticipantClaimV1> =
            serde_json::from_slice(document.as_bytes()).map_err(|error| {
                MultiplayerError::ranked_document("decode browser ranked roster update", error)
            })?;
        let mut lifecycle = self.lifecycle.lock().map_err(|_| lifecycle_poisoned())?;
        let genesis = lifecycle
            .ranked_client()
            .map(|client| client.session_genesis.clone())
            .ok_or_else(|| {
                MultiplayerError::Ranked(
                    "ranked roster update has no accepted browser client lifecycle".into(),
                )
            })?;
        lifecycle
            .update_ranked_client_roster(&genesis, roster)
            .map_err(|error| {
                MultiplayerError::ranked_document("install browser ranked roster update", error)
            })?;
        drop(lifecycle);
        links
            .incoming
            .send(NetEvent::RankedParticipantRoster(document))
            .map_err(|_| closed("browser ranked roster channel is closed"))
    }

    fn on_browse_only(
        &self,
        links: &SessionLinks<'_, Self>,
        reason: RankedBrowseOnlyReason,
    ) -> Result<(), MultiplayerError> {
        if self.join.mark_browse_only(reason)? {
            downgrade_ranked_lifecycle(&self.lifecycle, ranked_browse_only_reason(reason))?;
            self.admission.set(BrowserAdmissionPhase::BrowseOnly);
            links
                .incoming
                .send(NetEvent::RankedBrowseOnly { reason })
                .map_err(|_| closed("browser ranked browse-only channel is closed"))?;
        }
        Ok(())
    }

    fn on_cosign_context(
        &self,
        links: &SessionLinks<'_, Self>,
        context: RankedCoSignContextDocument,
    ) -> Result<(), MultiplayerError> {
        if !self.join.is_accepted()? {
            return Err(MultiplayerError::RemoteProtocol(
                "host sent a ranked co-sign context outside an accepted ranked session".into(),
            ));
        }
        links
            .incoming
            .send(NetEvent::RankedCoSignContext(context))
            .map_err(|_| closed("browser ranked co-sign context channel is closed"))
    }

    fn on_submission_accepted(
        &self,
        _links: &SessionLinks<'_, Self>,
        accepted: RankedSubmissionAcceptedDocument,
    ) -> Result<(), MultiplayerError> {
        // TODO(10/F1): browsers never learned to relay submission
        // acknowledgements; native validates and publishes them.
        Err(MultiplayerError::RemoteProtocol(
            format!(
                "host sent invalid browser session message {:?}",
                NetMsg::RankedSubmissionAccepted(accepted)
            )
            .into(),
        ))
    }

    fn on_session_dropped(&self) -> Result<(), MultiplayerError> {
        if self.admission.get().can_reconnect() {
            self.join
                .begin_reconnect()
                .map_err(|error| error.context("could not begin authenticated ranked reconnect"))?;
            self.admission.set(BrowserAdmissionPhase::AwaitingChallenge);
        }
        Ok(())
    }

    fn reset_after_failed_handshake(&self) -> Result<(), MultiplayerError> {
        if self.admission.get().can_reconnect() {
            self.join.begin_reconnect()?;
            self.admission.set(BrowserAdmissionPhase::AwaitingChallenge);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leaderboard_ranked_session::RankedSessionLifecycle;
    use crate::multiplayer::client_session::tests::{
        assert_premature_begin_sim_downgrades, assert_premature_cosign_request_downgrades,
        begin_sim, handle,
    };

    fn admission() -> BrowserRankedAdmission {
        let (_setup_tx, setup_rx) = crate::multiplayer::client_session::ranked_setup_channel();
        BrowserRankedAdmission::new(
            Arc::new(std::sync::Mutex::new(
                RankedSessionLifecycle::awaiting_prepared_inputs(),
            )),
            setup_rx,
            BrowserPeerAuth {
                join_code: String::new(),
                durable_public_key: [2; 32],
                signature: vec![0; 64],
            },
            iroh::SecretKey::generate().public(),
            iroh::SecretKey::generate().public(),
        )
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn browser_premature_begin_sim_downgrades_to_browse_only() {
        let ranked = admission();
        assert!(!ranked.admission_resolved());
        assert_premature_begin_sim_downgrades(&ranked);
        assert!(
            ranked.admission_resolved(),
            "a held ReadyToSim is released once browse-only"
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn browser_premature_cosign_request_downgrades_to_browse_only() {
        assert_premature_cosign_request_downgrades(&admission());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn browser_resolved_begin_sim_is_released_directly() {
        let ranked = admission();
        ranked.admission.set(BrowserAdmissionPhase::BrowseOnly);
        let (tx, rx) = std::sync::mpsc::channel();
        let cosign = Arc::new(Default::default());
        handle(&ranked, &tx, &cosign, begin_sim()).unwrap();
        assert!(matches!(
            rx.try_recv().unwrap(),
            NetEvent::BeginSim {
                frame: 9,
                start_epoch_ms: 12
            }
        ));
        assert!(rx.try_recv().is_err());
        assert!(
            ranked_lifecycle_lock(&ranked.lifecycle)
                .browse_only_reason()
                .is_none(),
            "no downgrade once admission is resolved"
        );
    }
}
