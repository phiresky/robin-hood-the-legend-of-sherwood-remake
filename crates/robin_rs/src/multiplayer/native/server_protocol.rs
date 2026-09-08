//! Pure, bounded server protocol state. Transport authority remains in the
//! parent dispatcher: deserializing these values cannot create an active stream.

use super::{
    LeaderboardCoSignInstanceV1, LeaderboardCoSignRequestV1, LeaderboardCoSignResponse,
    MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION, PendingRankedAdmission, PlayerId,
    verify_leaderboard_cosign_response,
};
use robin_engine::multiplayer::{SnapshotTransitionId, SnapshotTransitionPayload};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Instant;

#[derive(Default, Serialize, Deserialize)]
pub(super) struct ReadyBarrier {
    pub(super) host_frame: Option<u32>,
    pub(super) begun: Option<(u32, u64)>,
}

impl ReadyBarrier {
    pub(super) fn reset(&mut self) {
        self.host_frame = None;
        self.begun = None;
    }

    /// Facts include all authenticated seats; only connected seats count toward
    /// quorum, but every ready frame contributes to the release frame, matching
    /// the existing reconnect/provisional-seat policy.
    pub(super) fn candidate(
        &self,
        expected_players: u32,
        facts: impl IntoIterator<Item = (bool, bool, Option<u32>)>,
    ) -> Option<u32> {
        if self.begun.is_some() {
            return None;
        }
        let mut frame = self.host_frame?;
        let mut connected = 0;
        for (in_sim, attached, ready) in facts {
            if in_sim {
                connected += 1;
                if !attached || ready.is_none() {
                    return None;
                }
            }
            if let Some(ready) = ready {
                frame = frame.max(ready);
            }
        }
        (connected >= expected_players.saturating_sub(1)).then_some(frame)
    }

    pub(super) fn commit(&mut self, frame: u32, epoch_ms: u64) {
        assert!(self.begun.is_none(), "ready barrier released twice");
        self.begun = Some((frame, epoch_ms));
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct PendingSnapshotTransition {
    pub(super) id: SnapshotTransitionId,
    pub(super) payload: SnapshotTransitionPayload,
    pub(super) awaiting: HashSet<u8>,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct SnapshotTransitions {
    pending: Option<PendingSnapshotTransition>,
}

impl SnapshotTransitions {
    pub(super) fn pending(&self) -> Option<&PendingSnapshotTransition> {
        self.pending.as_ref()
    }

    pub(super) fn begin(&mut self, transition: PendingSnapshotTransition) {
        assert!(
            self.pending.is_none(),
            "another snapshot transition is pending"
        );
        self.pending = Some(transition);
    }

    pub(super) fn acknowledge(
        &mut self,
        seat: PlayerId,
        id: SnapshotTransitionId,
    ) -> Result<(), String> {
        let transition = self
            .pending
            .as_mut()
            .ok_or_else(|| format!("peer {seat:?} acknowledged no active snapshot transition"))?;
        if transition.id != id {
            return Err(format!(
                "peer {seat:?} acknowledged snapshot transition {id:?}, active is {:?}",
                transition.id
            ));
        }
        if !transition.awaiting.remove(&seat.0) {
            return Err(format!(
                "peer {seat:?} duplicated or was not expected for snapshot transition {id:?}"
            ));
        }
        Ok(())
    }

    pub(super) fn retain_for_reconnect(&mut self, seat: u8) {
        if let Some(transition) = self.pending.as_mut() {
            // Connectivity never shrinks the barrier, including after an ack.
            transition.awaiting.insert(seat);
        }
    }

    pub(super) fn take_completed(&mut self) -> Option<SnapshotTransitionId> {
        if !self.pending.as_ref()?.awaiting.is_empty() {
            return None;
        }
        Some(self.pending.take().expect("checked pending transition").id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct PendingLeaderboardCoSign {
    target_seat: u8,
    request: LeaderboardCoSignRequestV1,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct CoSignTracker {
    pending: Vec<PendingLeaderboardCoSign>,
    seen: Vec<(LeaderboardCoSignInstanceV1, u8)>,
}

impl CoSignTracker {
    pub(super) fn begin(
        &mut self,
        target: PlayerId,
        request: LeaderboardCoSignRequestV1,
    ) -> Result<(), String> {
        if target == PlayerId::HOST {
            return Err("leaderboard co-sign requests to the host must be signed locally".into());
        }
        request
            .signing_bytes()
            .map_err(|error| format!("invalid leaderboard co-sign request: {error}"))?;
        if self.seen.len() >= MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION {
            return Err(format!(
                "leaderboard co-sign request history exceeds the per-session limit of {}",
                MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION
            ));
        }
        let key = (request.instance, target.0);
        if self.seen.contains(&key) {
            return Err(format!(
                "duplicate leaderboard co-sign request instance for target {target:?}"
            ));
        }
        self.seen.push(key);
        self.pending.push(PendingLeaderboardCoSign {
            target_seat: target.0,
            request,
        });
        Ok(())
    }

    pub(super) fn complete(
        &mut self,
        from: PlayerId,
        expected_signer: Option<[u8; 32]>,
        response: &LeaderboardCoSignResponse,
    ) -> Result<(), String> {
        let position = self.pending.iter().position(|pending| {
            pending.target_seat == from.0 && pending.request.instance == response.instance
        }).ok_or_else(|| format!("peer {from:?} submitted a duplicate, wrong-target, or wrong-session leaderboard co-sign response"))?;
        let expected_signer = expected_signer.ok_or_else(|| {
            format!(
                "peer {from:?} has no admitted durable ranked identity for leaderboard co-signing"
            )
        })?;
        if response.signer_public_key != expected_signer {
            return Err(format!(
                "peer {from:?} signed a leaderboard request with a key other than its admitted durable identity"
            ));
        }
        verify_leaderboard_cosign_response(&self.pending[position].request, response)?;
        self.pending.remove(position);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn pending_count(&self) -> usize {
        self.pending.len()
    }
    #[cfg(test)]
    pub(super) fn seen_count(&self) -> usize {
        self.seen.len()
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct RankedAdmissionTracker {
    pending: Option<PendingRankedAdmission>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum AdmissionDeadline {
    Finished,
    Waiting,
    Expired,
}

impl RankedAdmissionTracker {
    pub(super) fn pending(&self) -> Option<&PendingRankedAdmission> {
        self.pending.as_ref()
    }
    pub(super) fn clear(&mut self) {
        self.pending = None;
    }
    pub(super) fn begin(&mut self, pending: PendingRankedAdmission) {
        assert!(
            self.pending.is_none(),
            "ranked admission challenge already pending"
        );
        self.pending = Some(pending);
    }
    pub(super) fn cancel_for(&mut self, seat: u8, generation: u64) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.seat == seat && pending.generation == generation)
        {
            self.pending = None;
            true
        } else {
            false
        }
    }
    pub(super) fn deadline(
        &self,
        seat: u8,
        generation: u64,
        current_attached_generation: Option<u64>,
        connected: bool,
        now: Instant,
    ) -> AdmissionDeadline {
        if current_attached_generation != Some(generation) || connected {
            return AdmissionDeadline::Finished;
        }
        match self
            .pending
            .as_ref()
            .filter(|pending| pending.seat == seat && pending.generation == generation)
        {
            Some(pending) if now >= pending.deadline => AdmissionDeadline::Expired,
            _ => AdmissionDeadline::Waiting,
        }
    }
}
