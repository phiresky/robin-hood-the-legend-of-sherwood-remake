//! Ordered multiplayer admission, independent of timeline and history owners.
use crate::game_session::multiplayer::{MultiplayerAdmissionEvent, MultiplayerSessionError};
use serde::{Deserialize, Serialize};

/// The transport owns handshakes and wire delivery; this state machine owns
/// the point at which a loaded mission may begin advancing simulation. Keeping
/// it in `TimelineRuntime` also keeps snapshot adoption ahead of replay and
/// rollback frame capture for both graphical and true-headless drivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::game_session) enum MultiplayerAdmission {
    NotRequired,
    HostWaitingForBegin,
    HostWaitingForResyncBegin { snapshot_frame: u32 },
    PeerWaitingForSnapshot,
    PeerWaitingForBegin { snapshot_frame: u32 },
    WaitingForStart { frame: u32, start_epoch_ms: u64 },
    Running,
}

impl MultiplayerAdmission {
    pub(super) fn apply(
        &mut self,
        event: MultiplayerAdmissionEvent,
    ) -> Result<(), MultiplayerSessionError> {
        *self = match (*self, event) {
            (
                MultiplayerAdmission::Running | MultiplayerAdmission::WaitingForStart { .. },
                MultiplayerAdmissionEvent::HostResynchronizing { frame },
            ) => MultiplayerAdmission::HostWaitingForResyncBegin {
                snapshot_frame: frame,
            },
            (
                MultiplayerAdmission::HostWaitingForResyncBegin { snapshot_frame },
                MultiplayerAdmissionEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                },
            ) if snapshot_frame == frame => MultiplayerAdmission::WaitingForStart {
                frame,
                start_epoch_ms,
            },
            (_, MultiplayerAdmissionEvent::Disconnected) => {
                MultiplayerAdmission::PeerWaitingForSnapshot
            }
            (
                MultiplayerAdmission::PeerWaitingForSnapshot,
                MultiplayerAdmissionEvent::InitialSnapshotAdopted { frame },
            ) => MultiplayerAdmission::PeerWaitingForBegin {
                snapshot_frame: frame,
            },
            (
                MultiplayerAdmission::PeerWaitingForBegin { snapshot_frame },
                MultiplayerAdmissionEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                },
            ) if frame == snapshot_frame => MultiplayerAdmission::WaitingForStart {
                frame,
                start_epoch_ms,
            },
            (
                MultiplayerAdmission::HostWaitingForBegin,
                MultiplayerAdmissionEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                },
            ) => MultiplayerAdmission::WaitingForStart {
                frame,
                start_epoch_ms,
            },
            (state, event) => {
                return Err(MultiplayerSessionError::Protocol(format!(
                    "invalid multiplayer admission ordering: state {state:?}, event {event:?}"
                )));
            }
        };
        Ok(())
    }
}
