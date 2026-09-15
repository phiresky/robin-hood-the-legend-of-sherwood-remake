//! Pure, bounded server protocol state. Transport authority remains in the
//! parent dispatcher: deserializing these values cannot create an active stream.

use super::{MultiplayerError, PlayerId};
use robin_engine::multiplayer::{SnapshotTransitionId, SnapshotTransitionPayload};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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
    ) -> Result<(), MultiplayerError> {
        let remote = |message: String| MultiplayerError::RemoteProtocol(message.into());
        let transition = self.pending.as_mut().ok_or_else(|| {
            remote(format!(
                "peer {seat:?} acknowledged no active snapshot transition"
            ))
        })?;
        if transition.id != id {
            return Err(remote(format!(
                "peer {seat:?} acknowledged snapshot transition {id:?}, active is {:?}",
                transition.id
            )));
        }
        if !transition.awaiting.remove(&seat.0) {
            return Err(remote(format!(
                "peer {seat:?} duplicated or was not expected for snapshot transition {id:?}"
            )));
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
