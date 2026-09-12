//! Shared client publication policy, independent of stream implementation.

use super::{NetEvent, NetMsg, NetOutbound, SharedClientLeaderboardCoSignState};
use crate::leaderboard_ranked_session::{
    CampaignContinuationReceiptSelectionResponseV1, decode_canonical_ranked_wire_document,
    decode_ranked_wire_document,
};
use robin_run_protocol::{ParticipantSignatureV1, PublicKey32};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::Sender;

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
pub(super) enum ClientProtocolError {
    #[error("client attempted a host-only multiplayer publication")]
    HostOnly,
    #[error("client queued content-admission traffic after gameplay began")]
    LateContent,
    #[error("ranked admission control must use the isolated setup/signer seam")]
    AdmissionBypass,
    #[error("leaderboard co-sign requires an admitted ranked client")]
    NotRanked,
    #[error("full-snapshot resynchronization requested: {0}")]
    Reconnect(String),
    #[error("{0}")]
    Protocol(String),
}

impl From<String> for ClientProtocolError {
    fn from(message: String) -> Self {
        Self::Protocol(message)
    }
}

/// Adapters supply their validated lifecycle evidence. Browser admission and
/// native browse-only fallback remain distinct; neither grants publication
/// authority merely because an outgoing variant exists.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(super) struct ClientPublicationAuthority {
    pub(super) co_sign_allowed: bool,
    pub(super) durable_public_key: Option<PublicKey32>,
}

pub(super) fn prepare(
    outgoing: NetOutbound,
    incoming: &Sender<NetEvent>,
    cosign: &SharedClientLeaderboardCoSignState,
    authority: ClientPublicationAuthority,
) -> Result<Option<NetMsg>, ClientProtocolError> {
    Ok(Some(match outgoing {
        NetOutbound::Input {
            origin_frame,
            command,
        } => NetMsg::Input {
            origin_frame,
            command,
        },
        NetOutbound::ReadyToSim { frame } => NetMsg::ReadyToSim { frame },
        NetOutbound::ModalProposal {
            instance,
            kind,
            result,
            requested_frame,
        } => NetMsg::ModalProposal {
            instance,
            kind,
            result,
            requested_frame,
        },
        NetOutbound::SnapshotTransitionReady { id } => NetMsg::SnapshotTransitionReady { id },
        NetOutbound::ReconnectForSnapshot { reason, .. }
        | NetOutbound::ReconnectAllForSnapshot { reason } => {
            return Err(ClientProtocolError::Reconnect(reason));
        }
        NetOutbound::RankedJoinResponse(_) | NetOutbound::ArmRankedJoin { .. } => {
            return Err(ClientProtocolError::AdmissionBypass);
        }
        NetOutbound::StateHash { .. }
        | NetOutbound::InitialSnapshot { .. }
        | NetOutbound::ModalDecision { .. }
        | NetOutbound::BeginSnapshotTransition { .. }
        | NetOutbound::RankedJoinChallenge { .. }
        | NetOutbound::RankedJoinAccepted { .. }
        | NetOutbound::RankedParticipantRoster { .. }
        | NetOutbound::RankedOfficialSessionSetup(_)
        | NetOutbound::RankedBrowseOnly { .. }
        | NetOutbound::RankedContinuationReceiptSelectionRequest(_)
        | NetOutbound::RankedContinuationPreflightClaim { .. }
        | NetOutbound::RankedCoSignContext { .. }
        | NetOutbound::RankedSubmissionAccepted { .. }
        | NetOutbound::LeaderboardCoSignRequest { .. } => {
            return Err(ClientProtocolError::HostOnly);
        }
        NetOutbound::ContentRequest { .. }
        | NetOutbound::ContentReject { .. }
        | NetOutbound::ContentReady { .. }
        | NetOutbound::ContentPrepared { .. } => return Err(ClientProtocolError::LateContent),
        NetOutbound::ArmLeaderboardCoSignRequest { request } => {
            if !authority.co_sign_allowed {
                return Err(ClientProtocolError::NotRanked);
            }
            if let Some(request) = cosign.arm_request(request)? {
                super::client_gameplay::deliver(
                    incoming,
                    NetEvent::LeaderboardCoSignRequest(request),
                )?;
            }
            return Ok(None);
        }
        NetOutbound::LeaderboardCoSignResponse(response) => {
            if !authority.co_sign_allowed {
                return Err(ClientProtocolError::NotRanked);
            }
            cosign.authorize_response(&response)?;
            NetMsg::LeaderboardCoSignResponse(response)
        }
        NetOutbound::RankedContinuationReceiptSelection(selection) => {
            let decoded = decode_ranked_wire_document::<
                CampaignContinuationReceiptSelectionResponseV1,
            >(selection.as_bytes())
            .map_err(|error| format!("invalid continuation receipt selection: {error}"))?;
            let local = authority.durable_public_key.ok_or_else(|| {
                "continuation receipt selection has no durable identity".to_owned()
            })?;
            if decoded.responder_public_key() != local {
                return Err(ClientProtocolError::Protocol(
                    "continuation receipt selection is controlled by another identity".into(),
                ));
            }
            NetMsg::RankedContinuationReceiptSelection(selection)
        }
        NetOutbound::RankedContinuationPreflightSignature(signature) => {
            let decoded = decode_canonical_ranked_wire_document::<ParticipantSignatureV1>(
                signature.as_bytes(),
            )
            .map_err(|error| format!("invalid continuation preflight signature: {error}"))?;
            let local = authority
                .durable_public_key
                .ok_or_else(|| "continuation preflight has no durable identity".to_owned())?;
            if decoded.public_key != local
                || decoded.public_key.is_zero()
                || decoded.signature.is_zero()
            {
                return Err(ClientProtocolError::Protocol(
                    "continuation preflight signature uses invalid identity material".into(),
                ));
            }
            NetMsg::RankedContinuationPreflightSignature(signature)
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn direct_inputs_preserve_payload_and_server_only_publication_fails() {
        let (incoming, _receiver) = std::sync::mpsc::channel();
        let cosign = Default::default();
        let authority = ClientPublicationAuthority {
            co_sign_allowed: false,
            durable_public_key: None,
        };
        let message = prepare(
            NetOutbound::Input {
                origin_frame: 19,
                command: robin_engine::player_command::PlayerCommand::RegisterPeasantName {
                    name: "Robin".into(),
                },
            },
            &incoming,
            &cosign,
            authority,
        )
        .unwrap()
        .unwrap();
        assert!(
            matches!(message, NetMsg::Input { origin_frame: 19, command: robin_engine::player_command::PlayerCommand::RegisterPeasantName { name } } if name == "Robin")
        );
        assert!(matches!(
            prepare(
                NetOutbound::InitialSnapshot {
                    frame: 0,
                    engine_bytes: vec![]
                },
                &incoming,
                &cosign,
                authority
            ),
            Err(ClientProtocolError::HostOnly)
        ));
    }
}
