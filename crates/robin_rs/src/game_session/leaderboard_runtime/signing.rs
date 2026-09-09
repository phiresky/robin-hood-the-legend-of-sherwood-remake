//! Post-mission host authorization and peer co-signing, independent of modal presentation.

use super::*;

/// Host-side coordinator for the only ranked multiplayer upload. Remote
/// peers receive a typed, locally checkable context followed by the fixed
/// purpose-bound request; the authenticated transport stamps the responding
/// seat and rejects request replay before this task sees it.
pub(super) struct MultiplayerHostSubmissionAuthorizer {
    port: crate::multiplayer::RankedMultiplayerPort,
    notification_participants: Vec<robin_run_protocol::PublicKey32>,
    campaign_controller: Option<robin_run_protocol::PublicKey32>,
}

impl MultiplayerHostSubmissionAuthorizer {
    pub(super) fn new(port: crate::multiplayer::RankedMultiplayerPort) -> Result<Self, String> {
        if port.role() != crate::multiplayer::RankedMultiplayerRole::Host
            || port.local_seat() != robin_engine::player_command::PlayerId::HOST
        {
            return Err(
                "multiplayer submission authorizer requires the authenticated host port".to_owned(),
            );
        }
        Ok(Self {
            port,
            notification_participants: Vec::new(),
            campaign_controller: None,
        })
    }

    fn host_public_key(&self) -> Result<robin_run_protocol::PublicKey32, String> {
        let lifecycle = self.port.lifecycle();
        Ok(lifecycle
            .lock()
            .map_err(|_| "ranked host lifecycle lock is poisoned".to_owned())?
            .ranked_session()
            .ok_or_else(|| {
                "ranked host lifecycle ended before submission acknowledgement".to_owned()
            })?
            .genesis()
            .claim
            .host_public_key)
    }
}

impl MissionEndSubmissionAuthorizer for MultiplayerHostSubmissionAuthorizer {
    fn begin(
        &mut self,
        request: SubmissionAuthorizationRequest,
    ) -> Result<Box<dyn SubmissionAuthorizationTask>, String> {
        let host_key = self.host_public_key()?;
        self.notification_participants = request
            .offer_request
            .participant_claims
            .iter()
            .filter(|claim| claim.public_key != host_key)
            .map(|claim| claim.public_key)
            .collect();
        self.campaign_controller = match request.offer_request.scope_request {
            ScopeRequestV1::IndividualLevel => None,
            ScopeRequestV1::CampaignGenesis => request
                .offer_request
                .participant_claims
                .iter()
                .find(|claim| claim.seat == 0)
                .map(|claim| claim.public_key),
            ScopeRequestV1::CampaignContinuation { .. } => request.campaign_controller_public_key,
        };
        Ok(Box::new(MultiplayerHostAuthorizationTask::begin(
            self.port.clone(),
            request,
        )?))
    }

    fn submission_accepted(
        &mut self,
        accepted: &robin_run_protocol::SubmissionAcceptedV1,
    ) -> Result<(), String> {
        for participant in self.notification_participants.iter().copied() {
            self.port
                .host_publish_submission_accepted(participant, accepted.clone())?;
        }
        Ok(())
    }

    fn owns_receipt_watch(&self) -> Result<bool, String> {
        match self.campaign_controller {
            None => Ok(true),
            Some(controller) => Ok(controller == self.host_public_key()?),
        }
    }
}

type PendingParticipants = BTreeMap<
    robin_run_protocol::PublicKey32,
    (
        robin_engine::player_command::PlayerId,
        robin_run_protocol::LeaderboardCoSignInstanceV1,
    ),
>;

/// Correlate against phase-owned requests before consuming any pending evidence.
/// Cryptographic validation remains the final closed submission gate.
fn accept_submission_response(
    pending: &mut PendingParticipants,
    signatures: &mut BTreeMap<robin_run_protocol::PublicKey32, ParticipantSignatureV1>,
    from: robin_engine::player_command::PlayerId,
    response: robin_engine::multiplayer::LeaderboardCoSignResponse,
) -> Result<(), String> {
    let key = robin_run_protocol::PublicKey32::from_bytes(response.signer_public_key);
    let Some((expected_seat, expected_instance)) = pending.get(&key).copied() else {
        return Err("unexpected or duplicate ranked participant co-sign response".to_owned());
    };
    if from != expected_seat || response.instance != expected_instance {
        return Err(
            "ranked participant co-sign response changed its authenticated seat or request"
                .to_owned(),
        );
    }
    pending.remove(&key);
    signatures.insert(
        key,
        ParticipantSignatureV1 {
            public_key: key,
            signature: Signature64::from_bytes(response.signature),
        },
    );

    Ok(())
}

enum MultiplayerHostAuthorizationPhase {
    AwaitingLocalContinuation {
        task: Box<dyn MissionEndTask<HostLocalSignature>>,
    },
    AwaitingContinuation {
        claim: robin_run_protocol::CampaignContinuationAuthorizationClaimV1,
        controller_key: robin_run_protocol::PublicKey32,
        request_instance: robin_run_protocol::LeaderboardCoSignInstanceV1,
    },
    AwaitingLocalSubmission {
        task: Box<dyn MissionEndTask<HostLocalSignature>>,
        envelope: robin_run_protocol::SubmissionEnvelopeV1,
    },
    AwaitingSubmission {
        signatures: BTreeMap<robin_run_protocol::PublicKey32, ParticipantSignatureV1>,
        envelope: robin_run_protocol::SubmissionEnvelopeV1,
        pending: PendingParticipants,
    },
    Complete {
        result: SignedSubmissionV1,
    },
    Finished {
        signed: Vec<robin_run_protocol::PublicKey32>,
    },
}

struct MultiplayerHostAuthorizationTask {
    port: HostAuthorizationPort,
    request: SubmissionAuthorizationRequest,
    expected: Vec<robin_run_protocol::PublicKey32>,
    phase: MultiplayerHostAuthorizationPhase,
}

// A closed adapter keeps deterministic tests on the actual task driver. Normal
// builds contain only the authenticated transport variant, not a second lane.
enum HostAuthorizationPort {
    Transport(crate::multiplayer::RankedMultiplayerPort),
    #[cfg(test)]
    Fixture(std::rc::Rc<std::cell::RefCell<tests::HostIo>>),
}

impl HostAuthorizationPort {
    fn host_publish_co_sign_operation(
        &self,
        seat: robin_engine::player_command::PlayerId,
        context: &crate::leaderboard_ranked_session::RankedCoSignContextV1,
    ) -> Result<robin_run_protocol::LeaderboardCoSignRequestV1, String> {
        match self {
            Self::Transport(port) => port.host_publish_co_sign_operation(seat, context),
            #[cfg(test)]
            Self::Fixture(io) => {
                let mut io = io.borrow_mut();
                io.publications.push(seat);
                if io.fail_publication == Some(io.publications.len()) {
                    return Err("injected publication failure".to_owned());
                }
                Ok(io
                    .published_request
                    .expect("publication fixture needs a request"))
            }
        }
    }

    fn try_recv_authorization_event(
        &self,
    ) -> Result<Option<crate::multiplayer::RankedAuthorizationEvent>, String> {
        match self {
            Self::Transport(port) => port.try_recv_authorization_event(),
            #[cfg(test)]
            Self::Fixture(io) => {
                let mut io = io.borrow_mut();
                io.polls += 1;
                Ok(io.events.pop_front())
            }
        }
    }
}

impl MultiplayerHostAuthorizationTask {
    fn begin(
        port: crate::multiplayer::RankedMultiplayerPort,
        request: SubmissionAuthorizationRequest,
    ) -> Result<Self, String> {
        request
            .validate_exact_context()
            .map_err(|error| error.to_string())?;
        let expected = request.expected_participants();
        let host_key = {
            let shared_lifecycle = port.lifecycle();
            let lifecycle = shared_lifecycle
                .lock()
                .map_err(|_| "ranked session lifecycle lock is poisoned".to_owned())?;
            lifecycle
                .ranked_session()
                .ok_or_else(|| "ranked host lifecycle is no longer eligible".to_owned())?
                .genesis()
                .claim
                .host_public_key
        };
        if expected.binary_search(&host_key).is_err() {
            return Err("ranked host identity is absent from the final participant set".to_owned());
        }
        let mut task = Self {
            port: HostAuthorizationPort::Transport(port),
            request,
            expected,
            phase: MultiplayerHostAuthorizationPhase::Finished { signed: Vec::new() },
        };
        match task
            .request
            .continuation_claim()
            .map_err(|error| error.to_string())?
        {
            Some(claim) if claim.campaign_controller_public_key == host_key => {
                task.phase = MultiplayerHostAuthorizationPhase::AwaitingLocalContinuation {
                    task: start_host_continuation_signature_task(
                        task.request.offer.clone(),
                        claim,
                    )?,
                };
            }
            Some(claim) => {
                let controller_key = claim.campaign_controller_public_key;
                let controller_seat = task.participant_seat(controller_key)?;
                let context =
                    crate::leaderboard_ranked_session::RankedCoSignContextV1::CampaignContinuation(
                        crate::leaderboard_ranked_session::RankedContinuationContextV1 {
                            offer_request: task.request.offer_request.clone(),
                            offer: task.request.offer.clone(),
                            continuation_claim: claim.clone(),
                        },
                    );
                let request = task
                    .port
                    .host_publish_co_sign_operation(controller_seat, &context)?;
                task.phase = MultiplayerHostAuthorizationPhase::AwaitingContinuation {
                    claim,
                    controller_key,
                    request_instance: request.instance,
                };
            }
            None => task.begin_submission(None)?,
        }
        Ok(task)
    }

    fn participant_seat(
        &self,
        key: robin_run_protocol::PublicKey32,
    ) -> Result<robin_engine::player_command::PlayerId, String> {
        let mut claims = self
            .request
            .offer_request
            .participant_claims
            .iter()
            .filter(|claim| claim.public_key == key);
        let claim = claims
            .next()
            .ok_or_else(|| "co-sign identity is absent from the authenticated roster".to_owned())?;
        if claims.next().is_some() {
            return Err("co-sign identity owns multiple authenticated seats".to_owned());
        }
        Ok(robin_engine::player_command::PlayerId(
            u8::try_from(claim.seat)
                .map_err(|_| "ranked participant seat exceeds the game wire range".to_owned())?,
        ))
    }

    fn begin_submission(
        &mut self,
        continuation: Option<CampaignContinuationAuthorizationV1>,
    ) -> Result<(), String> {
        let envelope = self.request.envelope(continuation);
        envelope.validate().map_err(|error| error.to_string())?;
        let task = start_host_submission_signature_task(envelope.clone())?;
        self.phase = MultiplayerHostAuthorizationPhase::AwaitingLocalSubmission { task, envelope };
        Ok(())
    }

    fn publish_submission(
        &mut self,
        envelope: robin_run_protocol::SubmissionEnvelopeV1,
        local: ParticipantSignatureV1,
    ) -> Result<(), String> {
        let signatures = BTreeMap::from([(local.public_key, local)]);
        let context = crate::leaderboard_ranked_session::RankedCoSignContextV1::Submission(
            crate::leaderboard_ranked_session::RankedSubmissionContextV1 {
                offer_request: self.request.offer_request.clone(),
                replay_session_transcript: self.request.replay_session_transcript.clone(),
                submission: envelope.clone(),
            },
        );
        let remote = self
            .request
            .offer_request
            .participant_claims
            .iter()
            .filter(|claim| claim.seat != 0)
            .map(|claim| (claim.public_key, claim.seat))
            .collect::<Vec<_>>();
        let mut pending = BTreeMap::new();
        for (key, seat) in remote {
            let seat =
                robin_engine::player_command::PlayerId(u8::try_from(seat).map_err(|_| {
                    "ranked participant seat exceeds the game wire range".to_owned()
                })?);
            let request = self.port.host_publish_co_sign_operation(seat, &context)?;
            if pending.insert(key, (seat, request.instance)).is_some() {
                return Err("ranked participant identity appears more than once".to_owned());
            }
        }
        self.phase = MultiplayerHostAuthorizationPhase::AwaitingSubmission {
            envelope,
            pending,
            signatures,
        };
        self.finish_if_complete()
    }

    fn poll_local_signature(&mut self) -> Result<(), String> {
        let result = match &mut self.phase {
            MultiplayerHostAuthorizationPhase::AwaitingLocalContinuation { task }
            | MultiplayerHostAuthorizationPhase::AwaitingLocalSubmission { task, .. } => {
                task.try_take()
            }
            _ => return Ok(()),
        };
        let Some(result) = result else {
            return Ok(());
        };
        match (
            std::mem::replace(
                &mut self.phase,
                MultiplayerHostAuthorizationPhase::Finished { signed: Vec::new() },
            ),
            result?,
        ) {
            (
                MultiplayerHostAuthorizationPhase::AwaitingLocalContinuation { .. },
                HostLocalSignature::Continuation(authorization),
            ) => self.begin_submission(Some(authorization)),
            (
                MultiplayerHostAuthorizationPhase::AwaitingLocalSubmission { envelope, .. },
                HostLocalSignature::Submission(signature),
            ) => self.publish_submission(envelope, signature),
            _ => Err("durable host signer returned the wrong closed ranked operation".to_owned()),
        }
    }

    fn finish_if_complete(&mut self) -> Result<(), String> {
        let MultiplayerHostAuthorizationPhase::AwaitingSubmission {
            envelope,
            pending,
            signatures,
        } = &self.phase
        else {
            return Ok(());
        };
        if !pending.is_empty() {
            return Ok(());
        }
        let signed = SignedSubmissionV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission: envelope.clone(),
            algorithm: SignatureAlgorithmV1::Ed25519,
            participant_signatures: signatures.values().cloned().collect(),
        };
        crate::leaderboard_mission_end::validate_authorized_submission(&self.request, &signed)
            .map_err(|error| error.to_string())?;
        self.phase = MultiplayerHostAuthorizationPhase::Complete { result: signed };
        Ok(())
    }

    fn accept_response(
        &mut self,
        from: robin_engine::player_command::PlayerId,
        response: robin_engine::multiplayer::LeaderboardCoSignResponse,
    ) -> Result<(), String> {
        let key = robin_run_protocol::PublicKey32::from_bytes(response.signer_public_key);
        let continuation_seat = match &self.phase {
            MultiplayerHostAuthorizationPhase::AwaitingContinuation { controller_key, .. } => {
                Some(self.participant_seat(*controller_key)?)
            }
            _ => None,
        };
        match &mut self.phase {
            MultiplayerHostAuthorizationPhase::AwaitingContinuation {
                claim,
                controller_key,
                request_instance,
            } => {
                if Some(from) != continuation_seat
                    || key != *controller_key
                    || response.instance != *request_instance
                {
                    return Err("campaign continuation co-sign response changed its authenticated target or request".to_owned());
                }
                let authorization = CampaignContinuationAuthorizationV1 {
                    claim: claim.clone(),
                    algorithm: SignatureAlgorithmV1::Ed25519,
                    signature: Signature64::from_bytes(response.signature),
                };
                self.begin_submission(Some(authorization))
            }
            MultiplayerHostAuthorizationPhase::AwaitingSubmission {
                pending,
                signatures,
                ..
            } => {
                accept_submission_response(pending, signatures, from, response)?;
                self.finish_if_complete()
            }
            MultiplayerHostAuthorizationPhase::AwaitingLocalContinuation { .. }
            | MultiplayerHostAuthorizationPhase::AwaitingLocalSubmission { .. } => Err(
                "ranked co-sign response arrived while the durable host signer was active"
                    .to_owned(),
            ),
            MultiplayerHostAuthorizationPhase::Complete { .. }
            | MultiplayerHostAuthorizationPhase::Finished { .. } => {
                Err("ranked co-sign response arrived after authorization completed".to_owned())
            }
        }
    }
}

impl MultiplayerHostAuthorizationPhase {
    fn cancel(&mut self) {
        *self = Self::Finished {
            signed: self.signed(),
        };
    }

    fn signed(&self) -> Vec<robin_run_protocol::PublicKey32> {
        match self {
            Self::AwaitingSubmission { signatures, .. } => signatures.keys().copied().collect(),
            Self::Complete { result } => result
                .participant_signatures
                .iter()
                .map(|signature| signature.public_key)
                .collect(),
            Self::Finished { signed } => signed.clone(),
            _ => Vec::new(),
        }
    }

    fn take_completed(&mut self) -> Option<SignedSubmissionV1> {
        if !matches!(self, Self::Complete { .. }) {
            return None;
        }
        let signed = self.signed();
        let Self::Complete { result } = std::mem::replace(self, Self::Finished { signed }) else {
            unreachable!("completion phase was checked before consuming its result");
        };
        Some(result)
    }
}

impl MultiplayerHostAuthorizationTask {
    fn take_completed(&mut self) -> Option<SignedSubmissionV1> {
        self.phase.take_completed()
    }

    fn fail(&mut self, error: String) -> Option<Result<SignedSubmissionV1, String>> {
        // Dropping the phase discards its local result receiver and pending evidence.
        // A detached browser signing future may still finish durable identity work;
        // it cannot publish that result through this retired authorization owner.
        // A terminal task must never consume another transport event on a later poll.
        self.phase.cancel();
        Some(Err(error))
    }
}

impl MissionEndTask<SignedSubmissionV1> for MultiplayerHostAuthorizationTask {
    fn try_take(&mut self) -> Option<Result<SignedSubmissionV1, String>> {
        if matches!(
            self.phase,
            MultiplayerHostAuthorizationPhase::Finished { .. }
        ) {
            return None;
        }
        if let Some(result) = self.take_completed() {
            return Some(Ok(result));
        }
        if let Err(error) = self.poll_local_signature() {
            return self.fail(error);
        }
        for _ in 0..64 {
            let event = match self.port.try_recv_authorization_event() {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(error) => return self.fail(error),
            };
            match event {
                crate::multiplayer::RankedAuthorizationEvent::CoSignResponse { from, response } => {
                    if let Err(error) = self.accept_response(from, response) {
                        return self.fail(error);
                    }
                }
                _ => {
                    return self
                        .fail("ranked host received a client-only authorization event".to_owned());
                }
            }
        }
        self.take_completed().map(Ok)
    }
}

impl SubmissionAuthorizationTask for MultiplayerHostAuthorizationTask {
    fn progress(&self) -> ParticipantSigningProgress {
        ParticipantSigningProgress {
            expected: self.expected.clone(),
            signed: self.phase.signed(),
        }
    }
}

enum HostLocalSignature {
    Continuation(CampaignContinuationAuthorizationV1),
    Submission(ParticipantSignatureV1),
}

#[cfg(not(target_arch = "wasm32"))]
struct ImmediateHostSignatureTask(Option<Result<HostLocalSignature, String>>);

#[cfg(not(target_arch = "wasm32"))]
impl MissionEndTask<HostLocalSignature> for ImmediateHostSignatureTask {
    fn try_take(&mut self) -> Option<Result<HostLocalSignature, String>> {
        self.0.take()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start_host_continuation_signature_task(
    offer: robin_run_protocol::SubmissionOfferV1,
    claim: robin_run_protocol::CampaignContinuationAuthorizationClaimV1,
) -> Result<Box<dyn MissionEndTask<HostLocalSignature>>, String> {
    Ok(Box::new(ImmediateHostSignatureTask(Some(
        crate::leaderboard_signing::sign_campaign_continuation(&offer, claim)
            .map(HostLocalSignature::Continuation)
            .map_err(|error| error.to_string()),
    ))))
}

#[cfg(not(target_arch = "wasm32"))]
fn start_host_submission_signature_task(
    envelope: robin_run_protocol::SubmissionEnvelopeV1,
) -> Result<Box<dyn MissionEndTask<HostLocalSignature>>, String> {
    Ok(Box::new(ImmediateHostSignatureTask(Some(
        crate::leaderboard_signing::sign_submission_claim(&envelope)
            .map(HostLocalSignature::Submission)
            .map_err(|error| error.to_string()),
    ))))
}

#[cfg(target_arch = "wasm32")]
struct BrowserHostSignatureTask(async_channel::Receiver<Result<HostLocalSignature, String>>);

#[cfg(target_arch = "wasm32")]
impl MissionEndTask<HostLocalSignature> for BrowserHostSignatureTask {
    fn try_take(&mut self) -> Option<Result<HostLocalSignature, String>> {
        match self.0.try_recv() {
            Ok(result) => Some(result),
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => {
                Some(Err("browser host signer stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn start_host_continuation_signature_task(
    offer: robin_run_protocol::SubmissionOfferV1,
    claim: robin_run_protocol::CampaignContinuationAuthorizationClaimV1,
) -> Result<Box<dyn MissionEndTask<HostLocalSignature>>, String> {
    let (sender, receiver) = async_channel::bounded(1);
    wasm_bindgen_futures::spawn_local(async move {
        let result =
            crate::leaderboard_signing::browser_game_sign_campaign_continuation(&offer, &claim)
                .await
                .map(HostLocalSignature::Continuation)
                .map_err(|error| error.to_string());
        let _ = sender.send(result).await;
    });
    Ok(Box::new(BrowserHostSignatureTask(receiver)))
}

#[cfg(target_arch = "wasm32")]
fn start_host_submission_signature_task(
    envelope: robin_run_protocol::SubmissionEnvelopeV1,
) -> Result<Box<dyn MissionEndTask<HostLocalSignature>>, String> {
    let (sender, receiver) = async_channel::bounded(1);
    wasm_bindgen_futures::spawn_local(async move {
        let result = crate::leaderboard_signing::browser_game_sign_submission_claim(&envelope)
            .await
            .map(HostLocalSignature::Submission)
            .map_err(|error| error.to_string());
        let _ = sender.send(result).await;
    });
    Ok(Box::new(BrowserHostSignatureTask(receiver)))
}

pub(super) struct MultiplayerPeerCoSigner {
    port: crate::multiplayer::RankedMultiplayerPort,
    client: crate::leaderboard_ranked_session::RankedSessionClientV1,
    scope_request: ScopeRequestV1,
    requested_metrics: Vec<BoardMetricV1>,
    campaign_controller_public_key: Option<robin_run_protocol::PublicKey32>,
    mission_id: String,
    starting_campaign_bytes: Arc<[u8]>,
    consented: bool,
    replay_task: Option<Box<dyn MissionEndTask<Arc<[u8]>>>>,
    replay_bytes: Option<Arc<[u8]>>,
    local_replay: Option<robin_engine::replay::ReplayData>,
    armed_request: Option<robin_run_protocol::LeaderboardCoSignRequestV1>,
    signature_task: Option<Box<dyn MissionEndTask<ParticipantSignatureV1>>>,
    final_response_sent: bool,
    accepted: Option<robin_run_protocol::SubmissionAcceptedV1>,
    failure: Option<String>,
}

impl MultiplayerPeerCoSigner {
    pub(super) fn new(
        port: crate::multiplayer::RankedMultiplayerPort,
        signed: &SignedRankedMissionAdmission,
        mission_id: String,
        starting_campaign_bytes: Arc<[u8]>,
    ) -> Result<Self, String> {
        if port.role() != crate::multiplayer::RankedMultiplayerRole::Client
            || port.local_seat() == robin_engine::player_command::PlayerId::HOST
        {
            return Err("ranked peer co-signer requires an authenticated client port".to_owned());
        }
        let client = signed
            .lifecycle
            .lock()
            .map_err(|_| "ranked client lifecycle lock is poisoned".to_owned())?
            .ranked_client()
            .cloned()
            .ok_or_else(|| {
                "ranked client admission did not finish before mission runtime".to_owned()
            })?;
        client.validate().map_err(|error| error.to_string())?;
        if client.local_seat != u16::from(port.local_seat().0)
            || client.session_genesis.claim.ranked_session.mission_id != mission_id
        {
            return Err(
                "ranked client lifecycle differs from the active mission seat or mission"
                    .to_owned(),
            );
        }
        Ok(Self {
            port,
            client,
            scope_request: signed.scope_request.clone(),
            requested_metrics: signed.requested_metrics.clone(),
            campaign_controller_public_key: signed.campaign_controller_public_key,
            mission_id,
            starting_campaign_bytes,
            consented: false,
            replay_task: None,
            replay_bytes: None,
            local_replay: None,
            armed_request: None,
            signature_task: None,
            final_response_sent: false,
            accepted: None,
            failure: None,
        })
    }

    fn local_public_key(&self) -> robin_run_protocol::PublicKey32 {
        self.client.admission.local_public_key
    }

    /// Only the locally authenticated campaign controller owns receipt watching.
    /// Presentation does not need access to the peer's admission internals.
    pub(super) fn receipt_controller_public_key(&self) -> Option<robin_run_protocol::PublicKey32> {
        self.campaign_controller_public_key
            .filter(|controller| *controller == self.local_public_key())
    }

    fn prepare_replay(&mut self) -> Result<bool, String> {
        if self.replay_bytes.is_some() && self.local_replay.is_some() {
            return Ok(true);
        }
        if self.replay_task.is_none() {
            self.replay_task = Some(ActiveMissionReplayExporter.begin()?);
        }
        let Some(result) = self.replay_task.as_mut().and_then(|task| task.try_take()) else {
            return Ok(false);
        };
        self.replay_task = None;
        let bytes = result?;
        let replay = crate::replay_service::process().snapshot()?.parse_sync()?;
        self.replay_bytes = Some(bytes);
        self.local_replay = Some(replay);
        Ok(true)
    }

    fn validate_offer_request(&self, request: &SubmissionOfferRequestV1) -> Result<(), String> {
        request.validate().map_err(|error| error.to_string())?;
        if request.session_genesis != self.client.session_genesis
            || request.participant_claims != self.client.participant_claims
            || request.mission_id != self.mission_id
            || request.scope_request != self.scope_request
        {
            return Err(
                "host co-sign context differs from the client's admitted session, roster, mission, or campaign scope"
                    .to_owned(),
            );
        }
        Ok(())
    }

    fn arm_context(
        &mut self,
        context: crate::leaderboard_ranked_session::RankedCoSignContextV1,
    ) -> Result<(), String> {
        if self.armed_request.is_some() || self.signature_task.is_some() {
            return Err("host sent overlapping ranked co-sign operations".to_owned());
        }
        let request = match context {
            crate::leaderboard_ranked_session::RankedCoSignContextV1::CampaignContinuation(
                context,
            ) => {
                self.validate_offer_request(&context.offer_request)?;
                let local_key = self.local_public_key();
                if self.campaign_controller_public_key != Some(local_key) {
                    return Err(
                        "host targeted a non-controller peer for campaign continuation authorization"
                            .to_owned(),
                    );
                }
                let expected =
                    crate::leaderboard_ranked_session::RankedLocalContinuationEvidenceV1 {
                        offer_request: context.offer_request.clone(),
                        continuation_claim: self.local_continuation_claim(&context)?,
                        local_public_key: local_key,
                    };
                context
                    .validate_and_co_sign_request(&expected)
                    .map_err(|error| error.to_string())?
            }
            crate::leaderboard_ranked_session::RankedCoSignContextV1::Submission(context) => {
                self.validate_offer_request(&context.offer_request)?;
                let replay_bytes = self
                    .replay_bytes
                    .as_ref()
                    .ok_or_else(|| "canonical replay export is not ready".to_owned())?;
                let local_replay = self
                    .local_replay
                    .as_ref()
                    .ok_or_else(|| "local replay evidence is not ready".to_owned())?;
                let replay = crate::leaderboard_mission_end::canonical_replay_artifact(
                    replay_bytes,
                    &self.starting_campaign_bytes,
                    &self.mission_id,
                    &context.replay_session_transcript,
                )
                .map_err(|error| error.to_string())?;
                let campaign = robin_run_protocol::ArtifactRefV1 {
                    sha256: Digest32::digest_bytes(&self.starting_campaign_bytes),
                    byte_length: u64::try_from(self.starting_campaign_bytes.len()).map_err(
                        |_| "starting campaign length exceeds protocol bounds".to_owned(),
                    )?,
                    media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
                };
                let continuation_claim = self.local_submission_continuation_claim(
                    &context,
                    robin_run_protocol::SubmissionArtifactsV1 {
                        replay: replay.clone(),
                        starting_campaign: campaign.clone(),
                    },
                )?;
                let expected = crate::leaderboard_ranked_session::RankedLocalSubmissionEvidenceV1 {
                    offer_request: context.offer_request.clone(),
                    replay_session_transcript: context.replay_session_transcript.clone(),
                    artifacts: robin_run_protocol::SubmissionArtifactsV1 {
                        replay,
                        starting_campaign: campaign,
                    },
                    campaign_aggregation_consent: match self.scope_request {
                        ScopeRequestV1::IndividualLevel => {
                            robin_run_protocol::CampaignAggregationConsentV1::NotAuthorized
                        }
                        ScopeRequestV1::CampaignGenesis
                        | ScopeRequestV1::CampaignContinuation { .. } => robin_run_protocol::CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1,
                    },
                    campaign_continuation_claim: continuation_claim,
                    requested_metrics: self.requested_metrics.clone(),
                    local_public_key: self.local_public_key(),
                };
                context
                    .validate_and_co_sign_request(&expected, local_replay)
                    .map_err(|error| error.to_string())?
            }
        };
        self.port.client_arm_co_sign_request(request)?;
        self.armed_request = Some(request);
        Ok(())
    }

    fn local_continuation_claim(
        &self,
        context: &crate::leaderboard_ranked_session::RankedContinuationContextV1,
    ) -> Result<robin_run_protocol::CampaignContinuationAuthorizationClaimV1, String> {
        let ScopeRequestV1::CampaignContinuation {
            chain_id,
            predecessor_run_id,
        } = &self.scope_request
        else {
            return Err(
                "continuation context arrived outside a locally admitted continuation".to_owned(),
            );
        };
        let claim = &context.continuation_claim;
        if &claim.chain_id != chain_id
            || &claim.predecessor_run_id != predecessor_run_id
            || Some(claim.campaign_controller_public_key) != self.campaign_controller_public_key
        {
            return Err(
                "continuation context differs from the locally retained chain receipt".to_owned(),
            );
        }
        Ok(claim.clone())
    }

    fn local_submission_continuation_claim(
        &self,
        context: &crate::leaderboard_ranked_session::RankedSubmissionContextV1,
        artifacts: robin_run_protocol::SubmissionArtifactsV1,
    ) -> Result<Option<robin_run_protocol::CampaignContinuationAuthorizationClaimV1>, String> {
        let Some(authorization) = &context.submission.campaign_continuation_authorization else {
            if matches!(
                self.scope_request,
                ScopeRequestV1::CampaignContinuation { .. }
            ) {
                return Err("host omitted the locally required campaign continuation".to_owned());
            }
            return Ok(None);
        };
        let claim = &authorization.claim;
        let ScopeRequestV1::CampaignContinuation {
            chain_id,
            predecessor_run_id,
        } = &self.scope_request
        else {
            return Err("host inserted campaign continuation into another local scope".to_owned());
        };
        if &claim.chain_id != chain_id
            || &claim.predecessor_run_id != predecessor_run_id
            || Some(claim.campaign_controller_public_key) != self.campaign_controller_public_key
            || claim.next_artifacts != artifacts
        {
            return Err(
                "host continuation authorization differs from local chain/artifact evidence"
                    .to_owned(),
            );
        }
        Ok(Some(claim.clone()))
    }

    fn poll_inner(&mut self) -> Result<(), String> {
        if !self.prepare_replay()? {
            return Ok(());
        }
        if let Some(task) = self.signature_task.as_mut() {
            let Some(signature) = task.try_take() else {
                return Ok(());
            };
            self.signature_task = None;
            let signature = signature?;
            let request = self
                .armed_request
                .take()
                .ok_or_else(|| "co-signature completed without an armed request".to_owned())?;
            if signature.public_key != self.local_public_key() {
                return Err("durable signer returned another participant identity".to_owned());
            }
            self.port.client_respond_co_sign(
                robin_engine::multiplayer::LeaderboardCoSignResponse {
                    instance: request.instance,
                    signer_public_key: *signature.public_key.as_bytes(),
                    signature: *signature.signature.as_bytes(),
                },
            )?;
            self.final_response_sent = matches!(
                request.instance.purpose,
                robin_run_protocol::LeaderboardCoSignPurposeV1::Submission
            );
        }
        for _ in 0..64 {
            let Some(event) = self.port.try_recv_authorization_event()? else {
                break;
            };
            match event {
                crate::multiplayer::RankedAuthorizationEvent::CoSignContext(context) => {
                    self.arm_context(context)?;
                }
                crate::multiplayer::RankedAuthorizationEvent::CoSignRequest(request) => {
                    if self.armed_request.as_ref() != Some(&request) {
                        return Err("transport released a co-sign request other than the locally armed request".to_owned());
                    }
                    install_signer_if_idle(&mut self.signature_task, || {
                        start_peer_signature_task(request)
                    })?;
                }
                crate::multiplayer::RankedAuthorizationEvent::SubmissionAccepted(accepted) => {
                    accepted.validate().map_err(|error| error.to_string())?;
                    self.accepted = Some(accepted);
                }
                crate::multiplayer::RankedAuthorizationEvent::CoSignResponse { .. } => {
                    return Err("ranked client received a host-only co-sign response".to_owned());
                }
                crate::multiplayer::RankedAuthorizationEvent::OfficialSessionSetup(_)
                | crate::multiplayer::RankedAuthorizationEvent::ContinuationReceiptSelectionRequest(_)
                | crate::multiplayer::RankedAuthorizationEvent::ContinuationReceiptSelectionResponse { .. }
                | crate::multiplayer::RankedAuthorizationEvent::ContinuationPreflightClaim(_)
                | crate::multiplayer::RankedAuthorizationEvent::ContinuationPreflightSignature { .. } => {
                    return Err(
                        "pre-frame ranked authorization event arrived at the mission-end co-signer"
                            .to_owned(),
                    );
                }
            }
        }
        Ok(())
    }
}

impl MissionEndPeerCoSigner for MultiplayerPeerCoSigner {
    fn consent(&mut self) -> Result<(), String> {
        if self.consented {
            return Err("ranked peer consent was already recorded".to_owned());
        }
        self.consented = true;
        Ok(())
    }

    fn poll(&mut self) -> PeerCoSignPoll {
        if let Some(error) = &self.failure {
            return PeerCoSignPoll::Failed(error.clone());
        }
        if !self.consented {
            return PeerCoSignPoll::AwaitingConsent;
        }
        if let Err(error) = self.poll_inner() {
            self.signature_task = None;
            self.armed_request = None;
            self.failure = Some(error.clone());
            return PeerCoSignPoll::Failed(error);
        }
        if let Some(accepted) = &self.accepted {
            return PeerCoSignPoll::Accepted(accepted.clone());
        }
        let expected = vec![self.local_public_key()];
        if self.signature_task.is_some() {
            return PeerCoSignPoll::Signing(ParticipantSigningProgress {
                expected,
                signed: Vec::new(),
            });
        }
        if self.final_response_sent {
            return PeerCoSignPoll::ResponseSent(ParticipantSigningProgress {
                expected: expected.clone(),
                signed: expected,
            });
        }
        PeerCoSignPoll::AwaitingHost
    }

    fn has_pending_work(&self) -> bool {
        self.consented && self.failure.is_none() && self.accepted.is_none()
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct ImmediatePeerSignatureTask(Option<Result<ParticipantSignatureV1, String>>);

#[cfg(not(target_arch = "wasm32"))]
impl MissionEndTask<ParticipantSignatureV1> for ImmediatePeerSignatureTask {
    fn try_take(&mut self) -> Option<Result<ParticipantSignatureV1, String>> {
        self.0.take()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start_peer_signature_task(
    request: robin_run_protocol::LeaderboardCoSignRequestV1,
) -> Result<Box<dyn MissionEndTask<ParticipantSignatureV1>>, String> {
    Ok(Box::new(ImmediatePeerSignatureTask(Some(
        crate::leaderboard_signing::sign_multiplayer_leaderboard_request(&request)
            .map_err(|error| error.to_string()),
    ))))
}

#[cfg(target_arch = "wasm32")]
struct BrowserPeerSignatureTask(async_channel::Receiver<Result<ParticipantSignatureV1, String>>);

#[cfg(target_arch = "wasm32")]
impl MissionEndTask<ParticipantSignatureV1> for BrowserPeerSignatureTask {
    fn try_take(&mut self) -> Option<Result<ParticipantSignatureV1, String>> {
        match self.0.try_recv() {
            Ok(result) => Some(result),
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => {
                Some(Err("browser peer signer stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn start_peer_signature_task(
    request: robin_run_protocol::LeaderboardCoSignRequestV1,
) -> Result<Box<dyn MissionEndTask<ParticipantSignatureV1>>, String> {
    let (sender, receiver) = async_channel::bounded(1);
    wasm_bindgen_futures::spawn_local(async move {
        let result =
            crate::leaderboard_signing::browser_game_sign_multiplayer_leaderboard_request(&request)
                .await
                .map_err(|error| error.to_string());
        let _ = sender.send(result).await;
    });
    Ok(Box::new(BrowserPeerSignatureTask(receiver)))
}

/// Check before invoking the durable signer: duplicate requests must not replace
/// an in-flight task or start a second browser identity operation.
fn install_signer_if_idle<T>(
    slot: &mut Option<Box<dyn MissionEndTask<T>>>,
    start: impl FnOnce() -> Result<Box<dyn MissionEndTask<T>>, String>,
) -> Result<(), String> {
    if slot.is_some() {
        return Err(
            "host sent a duplicate request while the durable peer signer was active".to_owned(),
        );
    }
    *slot = Some(start()?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer as _;
    use robin_engine::player_command::PlayerId;
    use robin_run_protocol::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::{cell::RefCell, rc::Rc};

    #[derive(Default, serde::Serialize, serde::Deserialize)]
    pub(super) struct HostIo {
        #[serde(skip)]
        pub(super) events: VecDeque<crate::multiplayer::RankedAuthorizationEvent>,
        pub(super) polls: usize,
        pub(super) publications: Vec<PlayerId>,
        pub(super) fail_publication: Option<usize>,
        pub(super) published_request: Option<LeaderboardCoSignRequestV1>,
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct DelayedSigner {
        #[serde(skip)]
        result: Option<Result<HostLocalSignature, String>>,
        pending: bool,
    }

    impl MissionEndTask<HostLocalSignature> for DelayedSigner {
        fn try_take(&mut self) -> Option<Result<HostLocalSignature, String>> {
            if std::mem::take(&mut self.pending) {
                None
            } else {
                self.result.take()
            }
        }
    }

    fn authorization_fixture() -> (SubmissionAuthorizationRequest, ed25519_dalek::SigningKey) {
        let key = ed25519_dalek::SigningKey::from_bytes(&[0x44; 32]);
        (authorization_for_key(&key), key)
    }

    // One host key authors the admission and the final signature. Offer context
    // comes from that admission, including the authority-signed grant expiry;
    // tests mutate a returned lawful request only for their intended failure.
    fn authorization_for_key(key: &ed25519_dalek::SigningKey) -> SubmissionAuthorizationRequest {
        let campaign = bitcode::encode(&Campaign::default());
        let (mut admission, replay) =
            super::super::tests::signed_single_player_admission_for_key(&campaign, key);
        admission
            .materialize_terminal_from_replay("Dem_Lei_MP", campaign.clone().into(), &replay)
            .unwrap();
        let RankedMissionAdmission::Authorized(input) = admission else {
            panic!("fixture must be authorized")
        };
        let ranked = &input.offer_request.session_genesis.claim.ranked_session;
        let offer = SubmissionOfferV1 {
            schema_version: SCHEMA_VERSION_V1,
            upload_challenge_id: OpaqueId::new("driver-offer").unwrap(),
            upload_challenge_nonce: ChallengeNonce32::from_bytes([2; 32]),
            expires_at_unix_ms: input
                .offer_request
                .session_genesis
                .claim
                .fresh_run_preflight_grant
                .as_ref()
                .expect("single-player fixture carries a signed fresh-run grant")
                .claim
                .expires_at_unix_ms,
            max_concurrent_players: input.offer_request.max_concurrent_players,
            participant_instance_count: input.offer_request.participant_instance_count,
            participant_claims: input.offer_request.participant_claims.clone(),
            session_genesis: input.offer_request.session_genesis.clone(),
            mission_id: ranked.mission_id.clone(),
            competition_manifest_sha256: ranked.competition_manifest_sha256,
            build_manifest_sha256: ranked.build_manifest_sha256,
            content_manifest_sha256: ranked.content_manifest_sha256,
            rules_config_sha256: ranked.rules_config_sha256,
            ruleset_manifest_sha256: ranked.ruleset_manifest_sha256,
            starting_state: InitialStateExpectationV1::IndividualLevel {
                template_id: OpaqueId::new("driver-template").unwrap(),
                campaign_state_requirement: CanonicalCampaignStateRequirementV1 {
                    edition: OfficialContentEditionV1::Demo,
                    kind: CanonicalCampaignStateKindV1::IndividualTemplate,
                    rules_config_sha256: ranked.rules_config_sha256,
                },
                campaign_sha256: Digest32::digest_bytes(&campaign),
                starting_campaign_byte_length: campaign.len() as u64,
            },
            allowed_metrics: input.requested_metrics.clone(),
        };
        let request = SubmissionAuthorizationRequest {
            offer_request: input.offer_request,
            offer,
            replay_session_transcript: input.replay_session_transcript,
            artifacts: SubmissionArtifactsV1 {
                replay: ReplayArtifactV1 {
                    artifact: ArtifactRefV1 {
                        sha256: Digest32::from_bytes([3; 32]),
                        byte_length: 100,
                        media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
                    },
                    replay_schema_version: CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
                },
                starting_campaign: ArtifactRefV1 {
                    sha256: Digest32::digest_bytes(&campaign),
                    byte_length: campaign.len() as u64,
                    media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
                },
            },
            requested_metrics: input.requested_metrics,
            campaign_controller_public_key: None,
        };
        request.validate_exact_context().unwrap();
        let envelope = request.envelope(None);
        envelope.validate().unwrap();
        crate::leaderboard_mission_end::validate_authorized_submission(
            &request,
            &SignedSubmissionV1 {
                schema_version: SCHEMA_VERSION_V1,
                submission: envelope,
                algorithm: SignatureAlgorithmV1::Ed25519,
                participant_signatures: vec![signature(&request, key)],
            },
        )
        .expect("fixture must satisfy the real final authorization validator");
        request
    }

    fn signature(
        request: &SubmissionAuthorizationRequest,
        key: &ed25519_dalek::SigningKey,
    ) -> ParticipantSignatureV1 {
        ParticipantSignatureV1 {
            public_key: PublicKey32::from_bytes(key.verifying_key().to_bytes()),
            signature: Signature64::from_bytes(
                key.sign(&request.envelope(None).signing_bytes().unwrap())
                    .to_bytes(),
            ),
        }
    }

    fn host_task(
        request: SubmissionAuthorizationRequest,
        phase: MultiplayerHostAuthorizationPhase,
    ) -> (MultiplayerHostAuthorizationTask, Rc<RefCell<HostIo>>) {
        let io = Rc::new(RefCell::new(HostIo {
            published_request: Some(request.envelope(None).co_sign_request().unwrap()),
            ..HostIo::default()
        }));
        (
            MultiplayerHostAuthorizationTask {
                expected: request.expected_participants(),
                request,
                phase,
                port: HostAuthorizationPort::Fixture(io.clone()),
            },
            io,
        )
    }

    fn response_event(
        request: &SubmissionAuthorizationRequest,
        key: &ed25519_dalek::SigningKey,
    ) -> crate::multiplayer::RankedAuthorizationEvent {
        let signature = signature(request, key);
        let participant = request
            .offer_request
            .participant_claims
            .iter()
            .find(|claim| claim.public_key == signature.public_key)
            .expect("response fixture key must belong to the admitted roster");
        crate::multiplayer::RankedAuthorizationEvent::CoSignResponse {
            from: PlayerId(u8::try_from(participant.seat).unwrap()),
            response: robin_engine::multiplayer::LeaderboardCoSignResponse {
                instance: request.envelope(None).co_sign_request().unwrap().instance,
                signer_public_key: *signature.public_key.as_bytes(),
                signature: *signature.signature.as_bytes(),
            },
        }
    }

    fn awaiting_response(
        request: SubmissionAuthorizationRequest,
    ) -> (MultiplayerHostAuthorizationTask, Rc<RefCell<HostIo>>) {
        request.validate_exact_context().unwrap();
        assert_eq!(request.offer_request.participant_claims.len(), 1);
        let envelope = request.envelope(None);
        let instance = envelope.co_sign_request().unwrap().instance;
        let claim = &request.offer_request.participant_claims[0];
        let pending = BTreeMap::from([(
            claim.public_key,
            (PlayerId(u8::try_from(claim.seat).unwrap()), instance),
        )]);
        host_task(
            request,
            MultiplayerHostAuthorizationPhase::AwaitingSubmission {
                envelope,
                signatures: BTreeMap::new(),
                pending,
            },
        )
    }

    #[test]
    fn authorization_builder_binds_host_identity_and_signed_expiry() {
        for seed in [0x44, 0x55] {
            let key = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
            let request = authorization_for_key(&key);
            assert_eq!(
                request.expected_participants(),
                vec![PublicKey32::from_bytes(key.verifying_key().to_bytes())]
            );
            let mut wrong_expiry = request.clone();
            wrong_expiry.offer.expires_at_unix_ms += 1;
            assert!(
                wrong_expiry
                    .validate_exact_context()
                    .unwrap_err()
                    .to_string()
                    .contains("run_preflight_grant_expiry")
            );
        }
    }

    #[test]
    fn host_delayed_wrong_local_kind_and_signer_error_are_terminal_without_consuming_events() {
        for wrong_kind in [false, true] {
            let (request, key) = authorization_fixture();
            let result = if wrong_kind {
                Ok(HostLocalSignature::Submission(signature(&request, &key)))
            } else {
                Err("signer failed".to_owned())
            };
            let event = response_event(&request, &key);
            let (mut task, io) = host_task(
                request,
                MultiplayerHostAuthorizationPhase::AwaitingLocalContinuation {
                    task: Box::new(DelayedSigner {
                        pending: true,
                        result: Some(result),
                    }),
                },
            );
            assert!(task.try_take().is_none());
            io.borrow_mut().events.push_back(event);
            let polls = io.borrow().polls;
            assert!(task.try_take().unwrap().is_err());
            assert!(task.try_take().is_none());
            assert_eq!(io.borrow().polls, polls);
            assert_eq!(io.borrow().events.len(), 1);
        }
    }

    #[test]
    fn host_final_response_and_queued_duplicate_retire_success_in_same_poll() {
        for duplicate in [false, true] {
            let (request, key) = authorization_fixture();
            let event = response_event(&request, &key);
            let duplicate_event = response_event(&request, &key);
            let (mut task, io) = awaiting_response(request);
            io.borrow_mut().events.push_back(event);
            if duplicate {
                io.borrow_mut().events.push_back(duplicate_event);
            }
            let result = task.try_take().unwrap();
            if duplicate {
                assert!(
                    result
                        .unwrap_err()
                        .contains("after authorization completed")
                );
            } else {
                let signed = result.unwrap();
                crate::leaderboard_mission_end::validate_authorized_submission(
                    &task.request,
                    &signed,
                )
                .unwrap();
            }
            let polls = io.borrow().polls;
            assert!(task.try_take().is_none());
            assert_eq!(io.borrow().polls, polls);
        }
    }

    #[test]
    fn host_wrong_response_binding_is_terminal_and_preserves_later_inbox_events() {
        for substitution in 0..4 {
            let (request, key) = authorization_fixture();
            let mut wrong = response_event(&request, &key);
            let crate::multiplayer::RankedAuthorizationEvent::CoSignResponse { from, response } =
                &mut wrong
            else {
                unreachable!()
            };
            match substitution {
                0 => *from = PlayerId(1),
                1 => response.signer_public_key = [99; 32],
                2 => response.instance.replay_session_id = Digest32::from_bytes([99; 32]),
                3 => response.instance.submission_offer_sha256 = Digest32::from_bytes([99; 32]),
                _ => unreachable!(),
            }
            let valid = response_event(&request, &key);
            let (mut task, io) = awaiting_response(request);
            io.borrow_mut().events.extend([wrong, valid]);
            assert!(
                task.try_take()
                    .unwrap()
                    .unwrap_err()
                    .contains(if substitution == 1 {
                        "unexpected"
                    } else {
                        "authenticated seat or request"
                    })
            );
            assert!(task.try_take().is_none());
            assert_eq!(io.borrow().events.len(), 1);
            assert_eq!(io.borrow().polls, 1);
        }
    }

    #[test]
    fn host_wrong_local_identity_fails_final_validation_and_stays_terminal() {
        let (request, key) = authorization_fixture();
        let envelope = request.envelope(None);
        let event = response_event(&request, &key);
        let other_key = ed25519_dalek::SigningKey::from_bytes(&[0x55; 32]);
        let local = signature(&request, &other_key);
        let (mut task, io) = host_task(
            request,
            MultiplayerHostAuthorizationPhase::AwaitingLocalSubmission {
                envelope,
                task: Box::new(DelayedSigner {
                    pending: false,
                    result: Some(Ok(HostLocalSignature::Submission(local))),
                }),
            },
        );
        io.borrow_mut().events.push_back(event);
        assert!(task.try_take().unwrap().is_err());
        assert!(task.try_take().is_none());
        assert_eq!(io.borrow().polls, 0);
        assert_eq!(io.borrow().events.len(), 1);
    }

    #[test]
    fn host_partial_publication_failure_is_never_retried() {
        let (request, key) = authorization_fixture();
        let local = signature(&request, &key);
        let envelope = request.envelope(None);
        let (mut task, io) = host_task(
            request,
            MultiplayerHostAuthorizationPhase::AwaitingLocalSubmission {
                envelope,
                task: Box::new(DelayedSigner {
                    pending: false,
                    result: Some(Ok(HostLocalSignature::Submission(local))),
                }),
            },
        );
        // Inject only the publication itinerary, after the validated fixture was
        // built. Fake I/O does not replace production crypto/context validation;
        // this test isolates failure between two already-authorized operations.
        for seat in [1, 2] {
            let mut claim = task.request.offer_request.participant_claims[0].clone();
            claim.seat = seat;
            claim.public_key = PublicKey32::from_bytes([seat as u8; 32]);
            task.request.offer_request.participant_claims.push(claim);
        }
        io.borrow_mut().fail_publication = Some(2);
        assert!(
            task.try_take()
                .unwrap()
                .unwrap_err()
                .contains("publication failure")
        );
        assert_eq!(io.borrow().publications, [PlayerId(1), PlayerId(2)]);
        assert!(task.try_take().is_none());
        assert_eq!(io.borrow().publications.len(), 2);
        assert_eq!(io.borrow().polls, 0);
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct PendingSigner {
        #[serde(skip)]
        dropped: Arc<AtomicBool>,
    }

    impl Drop for PendingSigner {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    impl MissionEndTask<HostLocalSignature> for PendingSigner {
        fn try_take(&mut self) -> Option<Result<HostLocalSignature, String>> {
            None
        }
    }

    #[test]
    fn duplicate_peer_request_does_not_start_or_replace_signer() {
        let dropped = Arc::new(AtomicBool::new(false));
        let mut slot: Option<Box<dyn MissionEndTask<HostLocalSignature>>> = None;
        install_signer_if_idle(&mut slot, || {
            Ok(Box::new(PendingSigner {
                dropped: dropped.clone(),
            }))
        })
        .unwrap();
        assert!(
            install_signer_if_idle(&mut slot, || {
                panic!("duplicate request must be rejected before invoking durable signer")
            })
            .is_err()
        );
        assert!(!dropped.load(Ordering::SeqCst));
        assert!(slot.as_mut().unwrap().try_take().is_none());
        drop(slot);
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn phase_owns_pending_signer_and_cancellation_drops_it() {
        let dropped = Arc::new(AtomicBool::new(false));
        let mut phase = MultiplayerHostAuthorizationPhase::AwaitingLocalContinuation {
            task: Box::new(PendingSigner {
                dropped: dropped.clone(),
            }),
        };
        let MultiplayerHostAuthorizationPhase::AwaitingLocalContinuation { task } = &mut phase
        else {
            unreachable!()
        };
        assert!(task.try_take().is_none());
        assert!(!dropped.load(Ordering::SeqCst));
        assert!(phase.take_completed().is_none());
        phase.cancel();
        assert!(dropped.load(Ordering::SeqCst));
        assert!(phase.take_completed().is_none());
        phase.cancel();
        assert!(matches!(
            phase,
            MultiplayerHostAuthorizationPhase::Finished { .. }
        ));
    }

    #[test]
    fn response_correlation_rejects_substitution_without_consuming_request() {
        let (request, key) = authorization_fixture();
        let crate::multiplayer::RankedAuthorizationEvent::CoSignResponse { from, response } =
            response_event(&request, &key)
        else {
            unreachable!()
        };
        let pending = BTreeMap::from([(
            PublicKey32::from_bytes(response.signer_public_key),
            (from, response.instance),
        )]);
        for substitution in 0..4 {
            let mut pending = pending.clone();
            let mut signatures = BTreeMap::new();
            let mut changed = response.clone();
            let seat = if substitution == 0 { PlayerId(2) } else { from };
            match substitution {
                1 => changed.signer_public_key = [9; 32],
                2 => changed.instance.replay_session_id = Digest32::from_bytes([9; 32]),
                3 => changed.instance.submission_offer_sha256 = Digest32::from_bytes([9; 32]),
                _ => {}
            }
            assert!(
                accept_submission_response(&mut pending, &mut signatures, seat, changed).is_err()
            );
            assert_eq!(pending.len(), 1);
            assert!(signatures.is_empty());
        }
        let mut pending = pending;
        let mut signatures = BTreeMap::new();
        accept_submission_response(&mut pending, &mut signatures, from, response.clone()).unwrap();
        assert!(pending.is_empty());
        assert_eq!(signatures.len(), 1);
        assert!(accept_submission_response(&mut pending, &mut signatures, from, response).is_err());
        assert_eq!(signatures.len(), 1);
    }
}
