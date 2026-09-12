//! Wire frame classes and direction-specific allocation limits.
use super::*;

/// A class byte precedes each frame length so an impossible client snapshot
/// or oversized control message is rejected before allocating its body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum NetFrameClass {
    Control = 0,
    Input = 1,
    Snapshot = 2,
    Content = 3,
}

pub(crate) const MAX_SERVER_CONTROL_FRAME_BYTES: usize = 64 * 1024;
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) const MAX_CLIENT_CONTROL_FRAME_BYTES: usize = 32 * 1024;
pub(crate) const MAX_INPUT_FRAME_BYTES: usize = 256 * 1024;
pub(crate) const MAX_SNAPSHOT_FRAME_BYTES: usize =
    robin_engine::multiplayer::MAX_SNAPSHOT_FRAME_BYTES;
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) const MAX_HELLO_FRAME_BYTES: usize = 24 * 1024;
pub(crate) const MAX_CONTENT_FRAME_BYTES: usize =
    robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT + 16 * 1024;

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
