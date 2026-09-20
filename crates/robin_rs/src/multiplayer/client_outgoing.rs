//! Shared client publication policy, independent of stream implementation.

use super::{NetMsg, NetOutbound};

/// Why a local client publication was refused before reaching the wire.
///
/// Not serde: a local classification of a refused publication, never persisted.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ClientProtocolError {
    #[error("client attempted a host-only multiplayer publication")]
    HostOnly,
    #[error("client queued content-admission traffic after gameplay began")]
    LateContent,
    #[error("full-snapshot resynchronization requested: {0}")]
    Reconnect(String),
}

pub(super) fn prepare(outgoing: NetOutbound) -> Result<NetMsg, ClientProtocolError> {
    Ok(match outgoing {
        NetOutbound::Latency { to, nonce, reply } => NetMsg::Latency {
            from: robin_engine::player_command::PlayerId::HOST,
            to,
            nonce,
            reply,
        },
        NetOutbound::Chat { text } => NetMsg::ChatSend { text },
        NetOutbound::Input {
            origin_frame,
            command,
        } => NetMsg::Input {
            origin_frame,
            command,
        },
        NetOutbound::ReadyToSim { frame } => NetMsg::ReadyToSim { frame },
        NetOutbound::ModalProposal(proposal) => NetMsg::ModalProposal(proposal),
        NetOutbound::SnapshotTransitionReady { id } => NetMsg::SnapshotTransitionReady { id },
        NetOutbound::ReconnectForSnapshot { reason, .. }
        | NetOutbound::ReconnectAllForSnapshot { reason } => {
            return Err(ClientProtocolError::Reconnect(reason));
        }
        NetOutbound::StateHash { .. }
        | NetOutbound::InitialSnapshot { .. }
        | NetOutbound::ModalDecision { .. }
        | NetOutbound::BeginSnapshotTransition { .. } => {
            return Err(ClientProtocolError::HostOnly);
        }
        NetOutbound::ContentRequest { .. }
        | NetOutbound::ContentReject { .. }
        | NetOutbound::ContentReady { .. }
        | NetOutbound::ContentPrepared { .. } => return Err(ClientProtocolError::LateContent),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn direct_inputs_preserve_payload_and_server_only_publication_fails() {
        let message = prepare(NetOutbound::Input {
            origin_frame: 19,
            command: robin_engine::player_command::PlayerCommand::RegisterPeasantName {
                name: "Robin".into(),
            },
        })
        .unwrap();
        assert!(
            matches!(message, NetMsg::Input { origin_frame: 19, command: robin_engine::player_command::PlayerCommand::RegisterPeasantName { name } } if name == "Robin")
        );
        assert!(matches!(
            prepare(NetOutbound::InitialSnapshot {
                frame: 0,
                engine_bytes: vec![]
            }),
            Err(ClientProtocolError::HostOnly)
        ));
    }
}
