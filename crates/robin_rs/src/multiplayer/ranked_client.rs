//! Client ranked-admission and co-sign trust state shared by native and browser transports.

use robin_engine::multiplayer::{
    LeaderboardCoSignResponse, RankedBrowseOnlyReason, RankedJoinAccepted,
    RankedJoinAttestationDocument, RankedJoinChallenge, RankedJoinClaimDocument,
    RankedJoinResponse, RankedParticipantRosterDocument, RankedSessionConfigDocument,
    RankedSessionGenesisDocument,
};
use robin_run_protocol::{
    CanonicalDocument, LeaderboardCoSignInstanceV1, LeaderboardCoSignRequestV1,
    NamedSeatJoinAttestationV1, NamedSeatJoinClaimV1, ParticipantClaimV1, RankedSessionConfigV1,
    ReplaySessionGenesisV1, Validate,
};

/// A multiplayer campaign can span every shipped mission. This bound covers
/// both purpose-specific requests for every allowed participant at every
/// mission end with ample headroom, while keeping a malicious or defective
/// host from growing client replay-protection state without limit.
pub(crate) const MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION: usize = 1024;

fn decode_ranked_document<T>(bytes: &[u8], description: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned + serde::Serialize + Validate,
{
    crate::leaderboard_ranked_session::decode_ranked_wire_document(bytes)
        .map_err(|error| format!("invalid {description}: {error}"))
}

fn validate_ranked_join_challenge(
    challenge: &RankedJoinChallenge,
) -> Result<(ReplaySessionGenesisV1, NamedSeatJoinClaimV1), String> {
    let genesis: ReplaySessionGenesisV1 = decode_ranked_document(
        challenge.session_genesis.as_bytes(),
        "ranked session genesis",
    )?;
    let claim: NamedSeatJoinClaimV1 =
        decode_ranked_document(challenge.join_claim.as_bytes(), "ranked join claim")?;
    let genesis_digest = genesis
        .canonical_digest()
        .map_err(|error| format!("invalid ranked session genesis digest: {error}"))?;
    if claim.session_genesis_sha256 != genesis_digest
        || claim.host_endpoint_id != genesis.claim.host_public_key
        || claim.replay_session_id != genesis.claim.replay_session_id
        || claim.host_nonce != genesis.claim.host_nonce
        || claim.mission_id != genesis.claim.ranked_session.mission_id
        || claim.content_manifest_sha256 != genesis.claim.ranked_session.content_manifest_sha256
        || claim.rules_config_sha256 != genesis.claim.ranked_session.rules_config_sha256
        || claim.ruleset_manifest_sha256 != genesis.claim.ranked_session.ruleset_manifest_sha256
        || claim.competition_manifest_sha256
            != genesis.claim.ranked_session.competition_manifest_sha256
    {
        return Err("ranked join claim does not match its signed session genesis".to_string());
    }
    Ok((genesis, claim))
}

fn challenge_matches_expected_session(
    challenge: &RankedJoinChallenge,
    expected: &RankedSessionConfigDocument,
) -> Result<(), String> {
    let expected_config: RankedSessionConfigV1 =
        decode_ranked_document(expected.as_bytes(), "expected ranked session configuration")?;
    let (genesis, _) = validate_ranked_join_challenge(challenge)?;
    if genesis.claim.ranked_session != expected_config {
        return Err(
            "ranked join challenge does not match the locally prepared session".to_string(),
        );
    }
    Ok(())
}

fn decode_ranked_participant_roster(
    document: &RankedParticipantRosterDocument,
    genesis: &ReplaySessionGenesisV1,
) -> Result<Vec<ParticipantClaimV1>, String> {
    let roster: Vec<ParticipantClaimV1> = serde_json::from_slice(document.as_bytes())
        .map_err(|error| format!("invalid ranked participant roster: {error}"))?;
    let canonical = crate::leaderboard_ranked_session::encode_ranked_wire_document(&roster)
        .map_err(|error| format!("invalid ranked participant roster: {error}"))?;
    if canonical != document.as_bytes() {
        return Err("ranked participant roster is not canonical JSON".to_string());
    }
    crate::leaderboard_ranked_session::validate_participant_roster(genesis, &roster)
        .map_err(|error| format!("invalid ranked participant roster: {error}"))?;
    Ok(roster)
}

fn roster_contains_exact_join(
    roster: &[ParticipantClaimV1],
    attestation: &NamedSeatJoinAttestationV1,
) -> bool {
    roster.iter().any(|participant| {
        participant.seat == attestation.claim.seat
            && participant.participant_instance_id == attestation.claim.participant_instance_id
            && participant.public_key == attestation.claim.public_key
            && participant.join_attestation.as_ref() == Some(attestation)
    })
}

#[derive(Clone, Debug)]
enum ClientRankedJoinPhase {
    Empty,
    Staged(RankedJoinChallenge),
    Delivered(RankedJoinChallenge),
    Responded {
        challenge: RankedJoinChallenge,
        attestation: RankedJoinAttestationDocument,
    },
    Accepted(RankedJoinAccepted),
    Unavailable,
    BrowseOnly(RankedBrowseOnlyReason),
}

#[derive(Clone, Debug)]
struct ClientRankedJoinInner {
    expected_session: Option<RankedSessionConfigDocument>,
    admitted_genesis: Option<RankedSessionGenesisDocument>,
    admitted_roster: Option<Vec<ParticipantClaimV1>>,
    last_accepted_join_claim: Option<NamedSeatJoinClaimV1>,
    phase: ClientRankedJoinPhase,
    retired_challenges: Vec<RankedJoinChallenge>,
}

/// Split-phase client trust gate for ranked multiplayer admission.
///
/// The signed host challenge and the local exact ranked configuration may
/// arrive in either order. The challenge is exposed once only after those two
/// independently sourced values match. A signed response still does not admit
/// the client: the exact host acknowledgement must also be received.
pub(crate) struct ClientRankedJoinState {
    inner: std::sync::Mutex<ClientRankedJoinInner>,
}

impl Default for ClientRankedJoinState {
    fn default() -> Self {
        Self {
            inner: std::sync::Mutex::new(ClientRankedJoinInner {
                expected_session: None,
                admitted_genesis: None,
                admitted_roster: None,
                last_accepted_join_claim: None,
                phase: ClientRankedJoinPhase::Empty,
                retired_challenges: Vec::new(),
            }),
        }
    }
}

pub(crate) type SharedClientRankedJoinState = std::sync::Arc<ClientRankedJoinState>;

impl ClientRankedJoinState {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ClientRankedJoinInner>, String> {
        self.inner
            .lock()
            .map_err(|_| "ranked join client state lock is poisoned".to_string())
    }

    /// Install the canonical local ranked-session configuration. Returns an
    /// already-staged challenge exactly once when it matches.
    pub(crate) fn arm_expected_session(
        &self,
        expected: RankedSessionConfigDocument,
    ) -> Result<Option<RankedJoinChallenge>, String> {
        let _: RankedSessionConfigV1 =
            decode_ranked_document(expected.as_bytes(), "expected ranked session configuration")?;
        let mut inner = self.lock()?;
        if inner.expected_session.is_some() {
            return Err("ranked join local session was armed more than once".to_string());
        }
        match &inner.phase {
            ClientRankedJoinPhase::Empty => {
                inner.expected_session = Some(expected);
                Ok(None)
            }
            ClientRankedJoinPhase::Staged(challenge) => {
                challenge_matches_expected_session(challenge, &expected)?;
                let challenge = challenge.clone();
                inner.expected_session = Some(expected);
                inner.phase = ClientRankedJoinPhase::Delivered(challenge.clone());
                Ok(Some(challenge))
            }
            ClientRankedJoinPhase::BrowseOnly(_) => {
                Err("ranked join cannot be armed after browse-only downgrade".to_string())
            }
            _ => Err("ranked join local session was armed more than once".to_string()),
        }
    }

    /// Stage a host challenge, or return it once when the local configuration
    /// was already armed and matches exactly.
    pub(crate) fn receive_wire_challenge(
        &self,
        challenge: RankedJoinChallenge,
    ) -> Result<Option<RankedJoinChallenge>, String> {
        validate_ranked_join_challenge(&challenge)?;
        let mut inner = self.lock()?;
        if inner.retired_challenges.contains(&challenge) {
            return Err("host replayed a ranked join challenge from an earlier stream".to_string());
        }
        if inner
            .admitted_genesis
            .as_ref()
            .is_some_and(|genesis| *genesis != challenge.session_genesis)
        {
            return Err("ranked reconnect changed the admitted session genesis".to_string());
        }
        match &inner.phase {
            ClientRankedJoinPhase::Empty => {
                if let Some(expected) = &inner.expected_session {
                    challenge_matches_expected_session(&challenge, expected)?;
                    inner.phase = ClientRankedJoinPhase::Delivered(challenge.clone());
                    Ok(Some(challenge))
                } else {
                    inner.phase = ClientRankedJoinPhase::Staged(challenge);
                    Ok(None)
                }
            }
            ClientRankedJoinPhase::BrowseOnly(_) => {
                Err("ranked join challenge arrived after browse-only downgrade".to_string())
            }
            _ => Err("host replayed or replaced a ranked join challenge".to_string()),
        }
    }

    /// Consume the delivered challenge with either its exact attestation or a
    /// typed inability to participate in ranking.
    pub(crate) fn authorize_response(&self, response: &RankedJoinResponse) -> Result<(), String> {
        let mut inner = self.lock()?;
        match (response, &inner.phase) {
            (
                RankedJoinResponse::Attestation(attestation_document),
                ClientRankedJoinPhase::Delivered(challenge),
            ) => {
                let attestation: NamedSeatJoinAttestationV1 = decode_ranked_document(
                    attestation_document.as_bytes(),
                    "ranked join attestation",
                )?;
                let (_, expected_claim) = validate_ranked_join_challenge(challenge)?;
                if attestation.claim != expected_claim {
                    return Err(
                        "ranked join attestation does not consume the delivered challenge"
                            .to_string(),
                    );
                }
                crate::leaderboard_ranked_session::verify_named_seat_join(
                    &attestation,
                    attestation.claim.transport_endpoint_id.as_bytes(),
                )
                .map_err(|error| format!("invalid ranked join attestation: {error}"))?;
                inner.phase = ClientRankedJoinPhase::Responded {
                    challenge: challenge.clone(),
                    attestation: attestation_document.clone(),
                };
                Ok(())
            }
            (RankedJoinResponse::Unavailable(_), ClientRankedJoinPhase::Staged(_))
            | (RankedJoinResponse::Unavailable(_), ClientRankedJoinPhase::Delivered(_)) => {
                inner.phase = ClientRankedJoinPhase::Unavailable;
                Ok(())
            }
            (_, ClientRankedJoinPhase::BrowseOnly(_)) => {
                Err("ranked join response attempted after browse-only downgrade".to_string())
            }
            _ => Err("ranked join response is duplicate or premature".to_string()),
        }
    }

    /// Admit the client only after the server echoes the exact signed genesis
    /// and attestation that were challenged and answered.
    pub(crate) fn receive_wire_acceptance(
        &self,
        accepted: RankedJoinAccepted,
    ) -> Result<RankedJoinAccepted, String> {
        let genesis: ReplaySessionGenesisV1 = decode_ranked_document(
            accepted.session_genesis.as_bytes(),
            "accepted ranked session genesis",
        )?;
        let accepted_attestation: NamedSeatJoinAttestationV1 = decode_ranked_document(
            accepted.join_attestation.as_bytes(),
            "accepted ranked join attestation",
        )?;
        let accepted_roster =
            decode_ranked_participant_roster(&accepted.participant_roster, &genesis)?;
        let mut inner = self.lock()?;
        let ClientRankedJoinPhase::Responded {
            challenge,
            attestation,
        } = &inner.phase
        else {
            return Err("ranked join acknowledgement is duplicate or premature".to_string());
        };
        if accepted.session_genesis != challenge.session_genesis
            || accepted.join_attestation != *attestation
        {
            return Err(
                "ranked join acknowledgement does not equal the challenged signed admission"
                    .to_string(),
            );
        }
        match (
            inner.admitted_roster.as_ref(),
            inner.last_accepted_join_claim.as_ref(),
        ) {
            (None, None) => {
                if accepted_attestation.claim.connection_epoch != 0
                    || !roster_contains_exact_join(&accepted_roster, &accepted_attestation)
                {
                    return Err(
                        "initial ranked acknowledgement does not contain the exact fresh admission"
                            .to_string(),
                    );
                }
            }
            (Some(previous_roster), Some(previous_claim)) => {
                let claim = &accepted_attestation.claim;
                if &accepted_roster != previous_roster {
                    return Err(
                        "ranked reconnect acknowledgement changed the immutable participant roster"
                            .to_string(),
                    );
                }
                if claim.seat != previous_claim.seat
                    || claim.participant_instance_id != previous_claim.participant_instance_id
                    || claim.public_key != previous_claim.public_key
                    || previous_claim.connection_epoch.checked_add(1)
                        != Some(claim.connection_epoch)
                    || claim.join_event_ordinal <= previous_claim.join_event_ordinal
                {
                    return Err(
                        "ranked reconnect acknowledgement changed its admitted participant or did not advance its lifecycle"
                            .to_string(),
                    );
                }
            }
            _ => {
                return Err(
                    "ranked join gate retained incomplete prior admission evidence".to_string(),
                );
            }
        }
        inner.admitted_genesis = Some(accepted.session_genesis.clone());
        inner.admitted_roster = Some(accepted_roster);
        inner.last_accepted_join_claim = Some(accepted_attestation.claim);
        inner.phase = ClientRankedJoinPhase::Accepted(accepted.clone());
        Ok(accepted)
    }

    /// Accept a complete roster broadcast only after this client is admitted.
    /// Every previous claim must remain byte-for-byte present and at least one
    /// new fresh participant must be added. Reconnects never mutate claims.
    pub(crate) fn receive_wire_roster_update(
        &self,
        document: RankedParticipantRosterDocument,
    ) -> Result<RankedParticipantRosterDocument, String> {
        let mut inner = self.lock()?;
        if !matches!(inner.phase, ClientRankedJoinPhase::Accepted(_)) {
            return Err("ranked participant roster arrived before join acceptance".to_string());
        }
        let genesis_document = inner
            .admitted_genesis
            .as_ref()
            .ok_or_else(|| "ranked participant roster has no admitted genesis".to_string())?;
        let genesis: ReplaySessionGenesisV1 =
            decode_ranked_document(genesis_document.as_bytes(), "admitted session genesis")?;
        let roster = decode_ranked_participant_roster(&document, &genesis)?;
        let previous = inner
            .admitted_roster
            .as_ref()
            .ok_or_else(|| "ranked participant roster has no admitted predecessor".to_string())?;
        if roster.len() <= previous.len() || previous.iter().any(|claim| !roster.contains(claim)) {
            return Err("ranked participant roster update is not monotonic".to_string());
        }
        inner.admitted_roster = Some(roster);
        Ok(document)
    }

    /// Begin a fresh authenticated transport cycle after the prior stream has
    /// ended. The locally prepared ranked configuration remains armed, while
    /// every old challenge, response, and acknowledgement is discarded. A
    /// fresh server claim (new endpoint/epoch) is therefore required.
    pub(crate) fn begin_reconnect(&self) -> Result<(), String> {
        let mut inner = self.lock()?;
        match &inner.phase {
            ClientRankedJoinPhase::BrowseOnly(_) | ClientRankedJoinPhase::Unavailable => {
                Err("ranked join cannot reconnect after an irreversible downgrade".to_string())
            }
            _ => {
                let retired = match &inner.phase {
                    ClientRankedJoinPhase::Accepted(accepted) => {
                        let attestation: NamedSeatJoinAttestationV1 = decode_ranked_document(
                            accepted.join_attestation.as_bytes(),
                            "accepted ranked join attestation",
                        )?;
                        let claim_bytes =
                            crate::leaderboard_ranked_session::encode_ranked_wire_document(
                                &attestation.claim,
                            )
                            .map_err(|error| {
                                format!("encode retired ranked join claim: {error}")
                            })?;
                        Some(RankedJoinChallenge {
                            session_genesis: accepted.session_genesis.clone(),
                            join_claim: RankedJoinClaimDocument::new(claim_bytes).map_err(
                                |error| format!("encode retired ranked join claim: {error}"),
                            )?,
                        })
                    }
                    // A challenge which never reached an authenticated host
                    // acknowledgement was not consumed. The host may issue
                    // the same epoch/ordinal again after the stream drops, so
                    // it must remain retryable. Only accepted challenges are
                    // permanently retired against replay.
                    ClientRankedJoinPhase::Empty
                    | ClientRankedJoinPhase::Staged(_)
                    | ClientRankedJoinPhase::Delivered(_)
                    | ClientRankedJoinPhase::Responded { .. } => None,
                    ClientRankedJoinPhase::Unavailable | ClientRankedJoinPhase::BrowseOnly(_) => {
                        unreachable!()
                    }
                };
                if let Some(retired) = retired {
                    if inner.retired_challenges.len() >= 1024 {
                        return Err(
                            "ranked join reconnect history exceeds the per-session limit"
                                .to_string(),
                        );
                    }
                    inner.retired_challenges.push(retired);
                }
                inner.phase = ClientRankedJoinPhase::Empty;
                Ok(())
            }
        }
    }

    /// Irreversibly downgrade this connection's ranking lane. Repeated events
    /// retain the first authoritative reason and do not emit again.
    pub(crate) fn mark_browse_only(&self, reason: RankedBrowseOnlyReason) -> Result<bool, String> {
        let mut inner = self.lock()?;
        if let ClientRankedJoinPhase::BrowseOnly(existing) = &inner.phase {
            let _ = existing;
            return Ok(false);
        }
        inner.phase = ClientRankedJoinPhase::BrowseOnly(reason);
        Ok(true)
    }

    pub(crate) fn is_accepted(&self) -> Result<bool, String> {
        Ok(matches!(
            self.lock()?.phase,
            ClientRankedJoinPhase::Accepted(_)
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClientLeaderboardCoSignEntry {
    /// Arrived on the authenticated host stream before local reconstruction.
    Staged(LeaderboardCoSignRequestV1),
    /// Independently reconstructed locally; waiting for the host request.
    Armed(LeaderboardCoSignRequestV1),
    /// Exact host/local match exposed to presentation and eligible to answer.
    Delivered(LeaderboardCoSignRequestV1),
    /// Successfully verified and queued once. Retained to reject replay.
    Responded(LeaderboardCoSignInstanceV1),
}

impl ClientLeaderboardCoSignEntry {
    fn instance(self) -> LeaderboardCoSignInstanceV1 {
        match self {
            Self::Staged(request) | Self::Armed(request) | Self::Delivered(request) => {
                request.instance
            }
            Self::Responded(instance) => instance,
        }
    }
}

#[derive(Default)]
struct ClientLeaderboardCoSignInner {
    entries: Vec<ClientLeaderboardCoSignEntry>,
}

/// Cross-reader/writer trust state shared by native and browser clients.
///
/// Network arrival and the game loop's local offer reconstruction occur on
/// independent schedules. A request is therefore staged without presentation
/// when it arrives early, and is released exactly once only after the locally
/// armed request is byte-for-byte/field-for-field identical. This is the
/// client-side wrong-session, wrong-offer, wrong-purpose, and wrong-digest
/// boundary; merely receiving a request from the current host is insufficient.
#[derive(Default)]
pub(crate) struct ClientLeaderboardCoSignState {
    inner: std::sync::Mutex<ClientLeaderboardCoSignInner>,
}

pub(crate) type SharedClientLeaderboardCoSignState = std::sync::Arc<ClientLeaderboardCoSignState>;

impl ClientLeaderboardCoSignState {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ClientLeaderboardCoSignInner>, String> {
        self.inner
            .lock()
            .map_err(|_| "leaderboard co-sign client state lock is poisoned".to_string())
    }

    fn validate_request(request: &LeaderboardCoSignRequestV1) -> Result<(), String> {
        request
            .validate()
            .map_err(|error| format!("invalid leaderboard co-sign request: {error}"))
    }

    fn reserve(inner: &ClientLeaderboardCoSignInner) -> Result<(), String> {
        if inner.entries.len() >= MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION {
            return Err(format!(
                "leaderboard co-sign request history exceeds the per-session limit of {}",
                MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION
            ));
        }
        Ok(())
    }

    /// Install the exact request independently derived from the validated
    /// offer/transcript. Returns it when an identical wire request arrived
    /// first and is now safe to present.
    pub(crate) fn arm_request(
        &self,
        request: LeaderboardCoSignRequestV1,
    ) -> Result<Option<LeaderboardCoSignRequestV1>, String> {
        Self::validate_request(&request)?;
        let mut inner = self.lock()?;
        if let Some(entry) = inner
            .entries
            .iter_mut()
            .find(|entry| entry.instance() == request.instance)
        {
            return match *entry {
                ClientLeaderboardCoSignEntry::Staged(staged) if staged == request => {
                    *entry = ClientLeaderboardCoSignEntry::Delivered(request);
                    Ok(Some(request))
                }
                ClientLeaderboardCoSignEntry::Staged(_) => Err(
                    "leaderboard co-sign host request does not equal the locally reconstructed request"
                        .to_string(),
                ),
                ClientLeaderboardCoSignEntry::Armed(_)
                | ClientLeaderboardCoSignEntry::Delivered(_)
                | ClientLeaderboardCoSignEntry::Responded(_) => Err(
                    "duplicate leaderboard co-sign request instance was armed locally".to_string(),
                ),
            };
        }
        if inner.entries.iter().any(|entry| {
            matches!(
                entry,
                ClientLeaderboardCoSignEntry::Staged(_)
                    | ClientLeaderboardCoSignEntry::Armed(_)
                    | ClientLeaderboardCoSignEntry::Delivered(_)
            )
        }) {
            return Err(
                "leaderboard co-sign host request does not equal the locally reconstructed request"
                    .to_string(),
            );
        }
        Self::reserve(&inner)?;
        inner
            .entries
            .push(ClientLeaderboardCoSignEntry::Armed(request));
        Ok(None)
    }

    /// Accept a request from the authenticated host stream. Returns it only
    /// after exact equality with an already armed local request.
    pub(crate) fn receive_wire_request(
        &self,
        request: LeaderboardCoSignRequestV1,
    ) -> Result<Option<LeaderboardCoSignRequestV1>, String> {
        Self::validate_request(&request)?;
        let mut inner = self.lock()?;
        if let Some(entry) = inner
            .entries
            .iter_mut()
            .find(|entry| entry.instance() == request.instance)
        {
            return match *entry {
                ClientLeaderboardCoSignEntry::Armed(armed) if armed == request => {
                    *entry = ClientLeaderboardCoSignEntry::Delivered(request);
                    Ok(Some(request))
                }
                ClientLeaderboardCoSignEntry::Armed(_) => Err(
                    "leaderboard co-sign host request does not equal the locally reconstructed request"
                        .to_string(),
                ),
                ClientLeaderboardCoSignEntry::Staged(_)
                | ClientLeaderboardCoSignEntry::Delivered(_)
                | ClientLeaderboardCoSignEntry::Responded(_) => Err(
                    "host replayed a duplicate leaderboard co-sign request instance".to_string(),
                ),
            };
        }
        if inner.entries.iter().any(|entry| {
            matches!(
                entry,
                ClientLeaderboardCoSignEntry::Staged(_)
                    | ClientLeaderboardCoSignEntry::Armed(_)
                    | ClientLeaderboardCoSignEntry::Delivered(_)
            )
        }) {
            return Err(
                "leaderboard co-sign host request does not equal the locally reconstructed request"
                    .to_string(),
            );
        }
        Self::reserve(&inner)?;
        inner
            .entries
            .push(ClientLeaderboardCoSignEntry::Staged(request));
        Ok(None)
    }

    /// Verify that a response signs the exact delivered request before it is
    /// allowed onto the wire, then consume that request exactly once.
    pub(crate) fn authorize_response(
        &self,
        response: &LeaderboardCoSignResponse,
    ) -> Result<(), String> {
        let mut inner = self.lock()?;
        let entry = inner
            .entries
            .iter_mut()
            .find(|entry| entry.instance() == response.instance)
            .ok_or_else(|| {
                "leaderboard co-sign response has no locally delivered request".to_string()
            })?;
        let ClientLeaderboardCoSignEntry::Delivered(request) = *entry else {
            return Err(
                "duplicate or premature leaderboard co-sign response was rejected".to_string(),
            );
        };
        verify_leaderboard_cosign_response(&request, response)?;
        *entry = ClientLeaderboardCoSignEntry::Responded(response.instance);
        Ok(())
    }
}

/// Validate a co-signature over the protocol crate's sole fixed, purpose-bound
/// Ed25519 payload. This helper intentionally cannot accept arbitrary bytes.
pub(crate) fn verify_leaderboard_cosign_response(
    request: &LeaderboardCoSignRequestV1,
    response: &LeaderboardCoSignResponse,
) -> Result<(), String> {
    request
        .validate()
        .map_err(|error| format!("invalid pending leaderboard co-sign request: {error}"))?;
    if response.instance != request.instance {
        return Err("leaderboard co-sign response instance does not match its request".to_string());
    }
    if response.signer_public_key == [0; 32] || response.signature == [0; 64] {
        return Err("leaderboard co-sign response contains zero key material".to_string());
    }
    let public_key = ed25519_dalek::VerifyingKey::from_bytes(&response.signer_public_key)
        .map_err(|error| format!("invalid leaderboard co-sign public key: {error}"))?;
    let signature = ed25519_dalek::Signature::from_bytes(&response.signature);
    let payload = request
        .signing_bytes()
        .map_err(|error| format!("invalid leaderboard co-sign signing payload: {error}"))?;
    public_key
        .verify_strict(&payload, &signature)
        .map_err(|_| "leaderboard co-sign signature does not match the exact request".to_string())
}
