//! Authenticated seat ownership, independent of transport and ranked policy.
//!
//! A writer may detach while its generation still owns the seat. Only a
//! matching owner/generation may release that seat into a reconnect reservation.
use super::{
    HostSessionContinuation, InactivePeerSession, NetMsg, PeerDispatchFailure, PeerOwner, PlayerId,
    RankedPeerIdentity, SeatClaim, SeatClaimKind,
};
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc::UnboundedSender;

/// Runtime authority cannot be restored from diagnostic serialization.
#[derive(serde::Serialize)]
pub(super) struct PeerSessions {
    next_seat: u8,
    seats: HashMap<u8, ServerSeat>,
    #[serde(serialize_with = "serialize_reservations")]
    disconnected_seats: HashMap<PeerOwner, u8>,
    next_session_generation: u64,
    expected_players: u32,
}

fn serialize_reservations<S: serde::Serializer>(
    reservations: &HashMap<PeerOwner, u8>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    // JSON cannot use the tagged authenticated-owner identity as an object key.
    serde::Serialize::serialize(&reservations.iter().collect::<Vec<_>>(), serializer)
}

robin_util::deny_deserialize!(
    PeerSessions,
    "peer session authority must be constructed by the live server"
);

/// One authenticated stream generation's metadata. Writer detachment is a
/// separate transition from release: snapshot commits must retain ownership
/// until continuation publication and stale reader teardown have completed.
#[derive(serde::Serialize, serde::Deserialize)]
struct ServerSeat {
    #[serde(skip)]
    sender: Option<UnboundedSender<NetMsg>>,
    nickname: String,
    owner: PeerOwner,
    ranked_identity: RankedPeerIdentity,
    claim_kind: SeatClaimKind,
    generation: u64,
    ready_frame: Option<u32>,
    sim_connected: bool,
}

impl PeerSessions {
    pub(super) fn authorize_session(
        &self,
        seat: PlayerId,
        generation: u64,
    ) -> Result<(), PeerDispatchFailure> {
        let inactive = |kind| PeerDispatchFailure::Inactive {
            seat,
            generation,
            kind,
        };
        let session = self
            .seats
            .get(&seat.0)
            .ok_or_else(|| inactive(InactivePeerSession::Released))?;
        if session.generation != generation {
            return Err(inactive(InactivePeerSession::Superseded {
                current_generation: session.generation,
            }));
        }
        if session.sender.is_none() {
            return Err(inactive(InactivePeerSession::Detached));
        }
        Ok(())
    }

    pub(super) fn sender(&self, seat: &u8) -> Option<&UnboundedSender<NetMsg>> {
        self.seats.get(seat)?.sender.as_ref()
    }

    pub(super) fn senders(&self) -> impl Iterator<Item = (&u8, &UnboundedSender<NetMsg>)> {
        self.seats
            .iter()
            .filter_map(|(seat, session)| session.sender.as_ref().map(|sender| (seat, sender)))
    }

    /// Host-directed reconnect invalidates new reader effects while returning
    /// the old FIFO writer for draining. Ownership and readiness remain until
    /// generation-matched release or replacement.
    pub(super) fn detach_writer(&mut self, seat: &u8) -> Option<UnboundedSender<NetMsg>> {
        self.seats.get_mut(seat)?.sender.take()
    }

    /// Snapshot commit detaches every writer without dropping authenticated
    /// ownership needed to publish the replacement mission's continuation.
    pub(super) fn detach_all_writers(&mut self) -> Vec<UnboundedSender<NetMsg>> {
        self.seats
            .values_mut()
            .filter_map(|session| session.sender.take())
            .collect()
    }

    pub(super) fn sim_connected_seats(&self) -> impl Iterator<Item = &u8> {
        self.seats
            .iter()
            .filter_map(|(seat, session)| session.sim_connected.then_some(seat))
    }

    pub(super) fn is_sim_connected(&self, seat: &u8) -> bool {
        self.seats
            .get(seat)
            .is_some_and(|session| session.sim_connected)
    }

    /// A completed admission belongs to one stream generation, never merely
    /// to a seat number that a replacement stream could have reclaimed.
    pub(super) fn admit_session(
        &mut self,
        seat: u8,
        generation: u64,
    ) -> Result<bool, PeerDispatchFailure> {
        self.authorize_session(PlayerId(seat), generation)?;
        let session = self
            .seats
            .get_mut(&seat)
            .expect("simulation connection requires an authenticated seat");
        Ok(!std::mem::replace(&mut session.sim_connected, true))
    }

    /// Browse-only policy admits all current provisional streams together.
    /// Detached writers are retained owners, not candidates for admission.
    pub(super) fn admit_provisional_sessions(&mut self) -> Vec<(u8, String)> {
        let mut seats = self
            .seats
            .iter_mut()
            .filter_map(|(&seat, session)| {
                if session.sender.is_none() || session.sim_connected {
                    return None;
                }
                session.sim_connected = true;
                Some((seat, session.nickname.clone()))
            })
            .collect::<Vec<_>>();
        seats.sort_unstable_by_key(|(seat, _)| *seat);
        seats
    }

    #[cfg(test)]
    pub(super) fn connect_sim_seat(&mut self, seat: u8) -> bool {
        let generation = *self.generation(&seat).expect("test seat must be claimed");
        self.admit_session(seat, generation)
            .expect("test seat must be active")
    }

    pub(super) fn generation(&self, seat: &u8) -> Option<&u64> {
        self.seats.get(seat).map(|session| &session.generation)
    }

    pub(super) fn ranked_identity(&self, seat: &u8) -> Option<&RankedPeerIdentity> {
        self.seats.get(seat).map(|session| &session.ranked_identity)
    }

    pub(super) fn nickname(&self, seat: &u8) -> Option<&String> {
        self.seats.get(seat).map(|session| &session.nickname)
    }

    pub(super) fn clear_ready(&mut self) {
        for session in self.seats.values_mut() {
            session.ready_frame = None;
        }
    }

    pub(super) fn record_ready(
        &mut self,
        seat: u8,
        generation: u64,
        frame: u32,
    ) -> Result<(), String> {
        self.authorize_session(PlayerId(seat), generation)
            .map_err(|error| error.to_string())?;
        let session = self
            .seats
            .get_mut(&seat)
            .expect("authorized session exists");
        session.ready_frame = Some(frame);
        Ok(())
    }

    pub(super) fn owner_seats(&self) -> HashMap<PeerOwner, u8> {
        let mut owners = self.disconnected_seats.clone();
        let mut occupied = owners.values().copied().collect::<HashSet<_>>();
        assert_eq!(
            occupied.len(),
            owners.len(),
            "duplicate retained seat ownership"
        );
        for (&seat, session) in &self.seats {
            assert!(
                occupied.insert(seat),
                "active seat is also reserved for a disconnected owner"
            );
            assert_eq!(
                owners.insert(session.owner, seat),
                None,
                "owner has multiple seat claims"
            );
        }
        owners
    }

    pub(super) fn new(expected_players: u32) -> Self {
        Self {
            next_seat: 1,
            seats: HashMap::new(),
            disconnected_seats: HashMap::new(),
            next_session_generation: 1,
            expected_players,
        }
    }

    pub(super) fn from_continuation(continuation: &HostSessionContinuation) -> Self {
        let next_seat = continuation
            .owner_seats
            .values()
            .copied()
            .max()
            .map_or(1, |seat| {
                seat.checked_add(1).expect("multiplayer seat overflow")
            });
        Self {
            next_seat,
            seats: HashMap::new(),
            disconnected_seats: continuation.owner_seats.clone(),
            next_session_generation: 1,
            expected_players: continuation.expected_players,
        }
    }

    pub(super) fn owner_seat(&self, owner: PeerOwner) -> Option<u8> {
        self.seats
            .iter()
            .find_map(|(&seat, session)| (session.owner == owner).then_some(seat))
            .or_else(|| self.disconnected_seats.get(&owner).copied())
    }

    pub(super) fn claim_seat(
        &mut self,
        owner: PeerOwner,
        nickname: &str,
        ranked_identity: RankedPeerIdentity,
        sender: UnboundedSender<NetMsg>,
    ) -> Result<SeatClaim, String> {
        // Prepare all fallible counters before consuming a retained reservation
        // or advancing allocation. Failed claims must leave ownership intact.
        let generation = self.next_session_generation;
        let next_generation = generation
            .checked_add(1)
            .ok_or_else(|| "multiplayer session generation overflow".to_string())?;
        let (seat, kind) = if let Some(active) = self
            .seats
            .iter()
            .find_map(|(&seat, session)| (session.owner == owner).then_some(seat))
        {
            assert!(
                !self.disconnected_seats.contains_key(&owner),
                "active owner also has a disconnected reservation"
            );
            (active, SeatClaimKind::ActiveReplacement)
        } else if let Some(disconnected) = self.disconnected_seats.remove(&owner) {
            assert!(
                !self.seats.contains_key(&disconnected),
                "retained seat already has an active owner"
            );
            (disconnected, SeatClaimKind::Reconnect)
        } else {
            if self.next_seat as u32 >= self.expected_players {
                return Err(format!(
                    "multiplayer session already has its configured {} players",
                    self.expected_players
                ));
            }
            let next = self.next_seat;
            self.next_seat = next
                .checked_add(1)
                .ok_or_else(|| "multiplayer seat overflow".to_string())?;
            (next, SeatClaimKind::Fresh)
        };
        self.next_session_generation = next_generation;
        let sim_connected = self
            .seats
            .get(&seat)
            .is_some_and(|session| session.sim_connected);
        self.seats.insert(
            seat,
            ServerSeat {
                sender: Some(sender),
                nickname: nickname.to_owned(),
                owner,
                ranked_identity,
                claim_kind: kind,
                generation,
                ready_frame: None,
                sim_connected,
            },
        );
        Ok(SeatClaim {
            seat,
            generation,
            kind,
        })
    }

    pub(super) fn release_seat_if_owner(
        &mut self,
        seat: u8,
        owner: PeerOwner,
        generation: u64,
    ) -> Option<bool> {
        let session = self.seats.get(&seat)?;
        if session.generation != generation || session.owner != owner {
            return None;
        }
        let session = self
            .seats
            .remove(&seat)
            .expect("matched active seat exists");
        assert_eq!(self.disconnected_seats.insert(owner, seat), None);
        Some(session.sim_connected)
    }

    pub(super) fn expected_players(&self) -> u32 {
        self.expected_players
    }

    pub(super) fn ranked_identities(&self) -> impl Iterator<Item = (&u8, &RankedPeerIdentity)> {
        self.seats
            .iter()
            .map(|(seat, session)| (seat, &session.ranked_identity))
    }

    /// Readiness projection contains no writable session authority.
    pub(super) fn readiness(&self) -> impl Iterator<Item = (bool, bool, Option<u32>)> + '_ {
        self.seats.values().map(|session| {
            (
                session.sim_connected,
                session.sender.is_some(),
                session.ready_frame,
            )
        })
    }

    #[cfg(test)]
    pub(super) fn ready_frame(&self, seat: u8) -> Option<u32> {
        self.seats
            .get(&seat)
            .expect("test requires a claimed seat")
            .ready_frame
    }

    #[cfg(test)]
    pub(super) fn set_test_ranked_identity(&mut self, seat: u8, identity: RankedPeerIdentity) {
        self.seats
            .get_mut(&seat)
            .expect("test requires a claimed seat")
            .ranked_identity = identity;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ServerPeers, maybe_begin_sim_locked};
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;
    fn ranked_identity(byte: u8) -> RankedPeerIdentity {
        RankedPeerIdentity {
            durable_public_key: Some([byte; 32]),
            transport_endpoint_id: [byte; 32],
            public_disclosure: robin_run_protocol::ParticipantPublicDisclosureV1::NamedProfile,
        }
    }
    fn claim_test_seat(peers: &mut ServerPeers, seat: u8, sender: UnboundedSender<NetMsg>) {
        let claim = peers
            .sessions
            .claim_seat(
                PeerOwner::Native([seat; 32]),
                "test peer",
                ranked_identity(seat),
                sender,
            )
            .unwrap();
        assert_eq!(claim.seat, seat);
    }

    #[test]
    fn stale_admission_and_readiness_cannot_mutate_a_replacement_session() {
        let mut sessions = PeerSessions::new(2);
        let owner = PeerOwner::Native([7; 32]);
        let (sender, _receiver) = unbounded_channel();
        let first = sessions
            .claim_seat(owner, "first", ranked_identity(7), sender)
            .unwrap();
        sessions
            .record_ready(first.seat, first.generation, 10)
            .unwrap();
        let (sender, _replacement_receiver) = unbounded_channel();
        let replacement = sessions
            .claim_seat(owner, "replacement", ranked_identity(7), sender)
            .unwrap();

        assert!(matches!(
            sessions.admit_session(first.seat, first.generation),
            Err(PeerDispatchFailure::Inactive {
                kind: InactivePeerSession::Superseded { .. },
                ..
            })
        ));
        assert!(
            sessions
                .record_ready(first.seat, first.generation, 20)
                .is_err()
        );
        assert_eq!(
            sessions.release_seat_if_owner(first.seat, owner, first.generation),
            None
        );
        assert!(!sessions.is_sim_connected(&replacement.seat));
        assert_eq!(sessions.ready_frame(replacement.seat), None);
        assert!(sessions.sender(&replacement.seat).is_some());

        assert!(
            sessions
                .admit_session(replacement.seat, replacement.generation)
                .unwrap()
        );
        sessions
            .record_ready(replacement.seat, replacement.generation, 30)
            .unwrap();
        assert_eq!(sessions.ready_frame(replacement.seat), Some(30));
        let _draining_writer = sessions.detach_writer(&replacement.seat).unwrap();
        assert!(matches!(
            sessions.admit_session(replacement.seat, replacement.generation),
            Err(PeerDispatchFailure::Inactive {
                kind: InactivePeerSession::Detached,
                ..
            })
        ));
        assert!(
            sessions
                .record_ready(replacement.seat, replacement.generation, 40)
                .is_err()
        );
        assert_eq!(sessions.ready_frame(replacement.seat), Some(30));
        assert_eq!(sessions.owner_seat(owner), Some(replacement.seat));
    }

    #[test]
    fn provisional_admission_excludes_detached_writers_and_is_idempotent() {
        let mut sessions = PeerSessions::new(4);
        let mut claims = Vec::new();
        let mut receivers = Vec::new();
        for byte in 1..=3 {
            let (sender, receiver) = unbounded_channel();
            receivers.push(receiver);
            claims.push(
                sessions
                    .claim_seat(
                        PeerOwner::Native([byte; 32]),
                        &format!("peer {byte}"),
                        ranked_identity(byte),
                        sender,
                    )
                    .unwrap(),
            );
        }
        let _draining_writer = sessions.detach_writer(&claims[1].seat).unwrap();
        assert_eq!(
            sessions.admit_provisional_sessions(),
            vec![(1, "peer 1".into()), (3, "peer 3".into())]
        );
        assert!(sessions.admit_provisional_sessions().is_empty());
        assert!(!sessions.is_sim_connected(&2));
        assert_eq!(sessions.owner_seats().len(), 3);
    }

    #[test]
    fn session_diagnostics_cannot_restore_live_or_reserved_authority() {
        let mut sessions = PeerSessions::new(2);
        let owner = PeerOwner::Browser([7; 32]);
        let (sender, _receiver) = unbounded_channel();
        let claim = sessions
            .claim_seat(owner, "peer", ranked_identity(7), sender)
            .unwrap();
        for release in [false, true] {
            if release {
                assert_eq!(
                    sessions.release_seat_if_owner(claim.seat, owner, claim.generation),
                    Some(false)
                );
            }
            let encoded = serde_json::to_string(&sessions).unwrap();
            let error = serde_json::from_str::<PeerSessions>(&encoded)
                .err()
                .expect("diagnostics cannot restore runtime authority");
            assert!(
                error
                    .to_string()
                    .contains("must be constructed by the live server")
            );
        }
    }
    #[test]
    fn registry_detachment_and_replacement_preserve_ownership_but_reset_readiness() {
        let mut peers = ServerPeers::new(2);
        let owner = PeerOwner::Native([7; 32]);
        let (sender, _receiver) = unbounded_channel();
        let first = peers
            .sessions
            .claim_seat(owner, "first", ranked_identity(7), sender)
            .unwrap();
        peers.sessions.connect_sim_seat(first.seat);
        peers
            .sessions
            .seats
            .get_mut(&first.seat)
            .unwrap()
            .ready_frame = Some(20);
        peers.readiness.host_frame = Some(10);
        let detached = peers.sessions.detach_writer(&first.seat).unwrap();
        assert_eq!(peers.sessions.owner_seats().get(&owner), Some(&first.seat));
        assert!(maybe_begin_sim_locked(&mut peers).unwrap().is_none());

        let (sender, _replacement_receiver) = unbounded_channel();
        let replacement = peers
            .sessions
            .claim_seat(owner, "replacement", ranked_identity(8), sender)
            .unwrap();
        let session = &peers.sessions.seats[&first.seat];
        assert_eq!(session.claim_kind, SeatClaimKind::ActiveReplacement);
        assert_eq!(session.nickname, "replacement");
        assert_eq!(session.ranked_identity, ranked_identity(8));
        assert!(session.sim_connected);
        assert_eq!(session.ready_frame, None);
        drop(detached);
        assert_eq!(
            peers
                .sessions
                .release_seat_if_owner(first.seat, owner, first.generation),
            None
        );
        assert_eq!(
            peers.sessions.release_seat_if_owner(
                first.seat,
                PeerOwner::Native([9; 32]),
                replacement.generation
            ),
            None
        );
        assert!(maybe_begin_sim_locked(&mut peers).unwrap().is_none());
        peers
            .sessions
            .seats
            .get_mut(&first.seat)
            .unwrap()
            .ready_frame = Some(30);
        let (frame, _, senders) = maybe_begin_sim_locked(&mut peers).unwrap().unwrap();
        assert_eq!(frame, 30);
        assert_eq!(senders.len(), 1);
        assert!(maybe_begin_sim_locked(&mut peers).unwrap().is_none());

        assert_eq!(
            peers
                .sessions
                .release_seat_if_owner(first.seat, owner, replacement.generation),
            Some(true)
        );
        assert!(peers.sessions.seats.is_empty());
        assert_eq!(peers.sessions.owner_seats().get(&owner), Some(&first.seat));
        let (sender, _reconnect_receiver) = unbounded_channel();
        let reconnect = peers
            .sessions
            .claim_seat(owner, "reconnect", ranked_identity(7), sender)
            .unwrap();
        assert_eq!(reconnect.kind, SeatClaimKind::Reconnect);
        assert_eq!(reconnect.seat, first.seat);
        assert!(!peers.sessions.is_sim_connected(&reconnect.seat));
        assert_eq!(peers.sessions.seats[&reconnect.seat].ready_frame, None);
    }

    #[test]
    fn registry_ready_barrier_requires_admission_and_resets_all_frames() {
        let mut peers = ServerPeers::new(3);
        for seat in [1, 2] {
            let (sender, _receiver) = unbounded_channel();
            claim_test_seat(&mut peers, seat, sender);
            peers
                .sessions
                .record_ready(seat, u64::from(seat), 10 + u32::from(seat))
                .unwrap();
        }
        peers.readiness.host_frame = Some(9);
        peers.sessions.connect_sim_seat(2);
        assert!(maybe_begin_sim_locked(&mut peers).unwrap().is_none());
        peers.sessions.connect_sim_seat(1);
        peers.sessions.clear_ready();
        assert!(
            peers
                .sessions
                .seats
                .values()
                .all(|session| session.ready_frame.is_none())
        );
        peers.sessions.record_ready(1, 1, 50).unwrap();
        assert!(maybe_begin_sim_locked(&mut peers).unwrap().is_none());
        peers.sessions.record_ready(2, 2, 40).unwrap();
        let (frame, _, senders) = maybe_begin_sim_locked(&mut peers).unwrap().unwrap();
        assert_eq!(frame, 50);
        assert_eq!(senders.len(), 2);
        assert_eq!(peers.sessions.detach_all_writers().len(), 2);
        assert_eq!(peers.sessions.owner_seats().len(), 2);
        assert_eq!(peers.sessions.sim_connected_seats().count(), 2);
    }

    #[test]
    fn registry_rejects_readiness_without_an_authenticated_session() {
        let mut peers = ServerPeers::new(2);
        assert!(
            peers
                .sessions
                .record_ready(1, 1, 10)
                .unwrap_err()
                .contains("has no active authenticated session")
        );
        let (sender, _receiver) = unbounded_channel();
        claim_test_seat(&mut peers, 1, sender);
        peers.sessions.record_ready(1, 1, 20).unwrap();
        let session = &peers.sessions.seats[&1];
        let (owner, generation) = (session.owner, session.generation);
        assert_eq!(
            peers.sessions.release_seat_if_owner(1, owner, generation),
            Some(false)
        );
        assert!(
            peers
                .sessions
                .record_ready(1, 1, 30)
                .unwrap_err()
                .contains("has no active authenticated session")
        );
        assert!(peers.sessions.seats.is_empty());
        assert_eq!(peers.sessions.owner_seat(owner), Some(1));
    }

    #[test]
    fn registry_generation_overflow_does_not_consume_any_claim() {
        for release in [false, true] {
            let mut peers = ServerPeers::new(3);
            let owner = PeerOwner::Native([7; 32]);
            let (sender, _receiver) = unbounded_channel();
            let first = peers
                .sessions
                .claim_seat(owner, "first", ranked_identity(7), sender)
                .unwrap();
            if release {
                assert_eq!(
                    peers
                        .sessions
                        .release_seat_if_owner(first.seat, owner, first.generation),
                    Some(false)
                );
            }
            let owners = peers.sessions.owner_seats();
            let next_seat = peers.sessions.next_seat;
            peers.sessions.next_session_generation = u64::MAX;
            for candidate in [owner, PeerOwner::Native([8; 32])] {
                let (sender, _receiver) = unbounded_channel();
                assert!(
                    peers
                        .sessions
                        .claim_seat(candidate, "failed", ranked_identity(8), sender)
                        .unwrap_err()
                        .contains("generation overflow")
                );
                assert_eq!(peers.sessions.owner_seats(), owners);
                assert_eq!(peers.sessions.next_seat, next_seat);
                assert_eq!(peers.sessions.next_session_generation, u64::MAX);
                assert_eq!(
                    peers.sessions.generation(&first.seat).copied(),
                    (!release).then_some(first.generation)
                );
            }
        }
    }

    #[test]
    fn registry_serialization_cannot_restore_a_writer() {
        let mut peers = ServerPeers::new(2);
        let (sender, _receiver) = unbounded_channel();
        claim_test_seat(&mut peers, 1, sender);
        let encoded = serde_json::to_string(&peers.sessions.seats[&1]).unwrap();
        let decoded: ServerSeat = serde_json::from_str(&encoded).unwrap();
        assert!(decoded.sender.is_none());
        assert_eq!(decoded.owner, peers.sessions.seats[&1].owner);
        assert_eq!(decoded.generation, peers.sessions.seats[&1].generation);
    }
}
