//! Shared post-admission gameplay delivery for native and browser clients.
//!
//! Welcome/content admission and ranked transitions belong to their lifecycle
//! owners. In particular, BeginSim must pass the platform's ranked-admission
//! policy before it can become an event; it is deliberately not decoded here.

use super::{NetEvent, NetMsg};
use std::sync::mpsc::Sender;

/// Decode only messages whose meaning is independent of transport and rank.
/// Other messages retain their ownership for the adapter's lifecycle handler.
pub(super) fn decode(message: NetMsg) -> Result<NetEvent, NetMsg> {
    Ok(match message {
        NetMsg::BroadcastInput {
            server_frame,
            origin_frame,
            target_frame,
            input,
        } => NetEvent::Input {
            server_frame,
            origin_frame,
            target_frame,
            input,
        },
        NetMsg::Note(note) => NetEvent::Note(note),
        NetMsg::StateHash {
            frame,
            hash,
            clock_frame,
            ms_until_next_frame,
        } => NetEvent::PeerStateHash {
            frame,
            hash,
            clock_frame,
            ms_until_next_frame,
        },
        NetMsg::InitialSnapshot {
            frame,
            engine_bytes,
        } => NetEvent::InitialSnapshot {
            frame,
            engine_bytes,
        },
        NetMsg::ModalDecision {
            instance,
            kind,
            result,
            decision_frame,
        } => NetEvent::ModalDecision {
            instance,
            kind,
            result,
            decision_frame,
        },
        NetMsg::PrepareSnapshotTransition { id, payload } => {
            NetEvent::PrepareSnapshotTransition { id, payload }
        }
        NetMsg::CommitSnapshotTransition { id } => NetEvent::CommitSnapshotTransition { id },
        other => return Err(other),
    })
}

/// Required local delivery: a closed receiver means the game loop has gone
/// away, never an idle connection. Even a diagnostic Note detects that terminal
/// condition here; this does not make network transmission of notes reliable.
pub(super) fn forward(
    message: NetMsg,
    incoming: &Sender<NetEvent>,
) -> Result<Option<NetMsg>, String> {
    match decode(message) {
        Ok(event) => {
            incoming
                .send(event)
                .map_err(|_| "client network event channel is closed".to_string())?;
            Ok(None)
        }
        Err(message) => Ok(Some(message)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn authoritative_snapshot_and_clock_fields_are_preserved() {
        assert!(
            matches!(decode(NetMsg::InitialSnapshot { frame: 41, engine_bytes: vec![3, 7] }),
            Ok(NetEvent::InitialSnapshot { frame: 41, engine_bytes }) if engine_bytes == [3, 7])
        );
        assert!(matches!(
            decode(NetMsg::StateHash {
                frame: 9,
                hash: Some(23),
                clock_frame: Some(8),
                ms_until_next_frame: None
            }),
            Ok(NetEvent::PeerStateHash {
                frame: 9,
                hash: Some(23),
                clock_frame: Some(8),
                ms_until_next_frame: None
            })
        ));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn begin_sim_cannot_bypass_adapter_admission_policy() {
        let (tx, rx) = mpsc::channel();
        assert!(matches!(
            forward(
                NetMsg::BeginSim {
                    frame: 42,
                    start_epoch_ms: 77
                },
                &tx
            )
            .unwrap(),
            Some(NetMsg::BeginSim {
                frame: 42,
                start_epoch_ms: 77
            })
        ));
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn local_delivery_preserves_order_and_reports_closed_game_loop() {
        let (tx, rx) = mpsc::channel();
        for frame in [7, 8] {
            assert!(
                forward(
                    NetMsg::InitialSnapshot {
                        frame,
                        engine_bytes: vec![]
                    },
                    &tx
                )
                .unwrap()
                .is_none()
            );
        }
        for expected in [7, 8] {
            assert!(
                matches!(rx.try_recv().unwrap(), NetEvent::InitialSnapshot { frame, .. } if frame == expected)
            );
        }
        drop(rx);
        assert!(
            forward(
                NetMsg::InitialSnapshot {
                    frame: 9,
                    engine_bytes: vec![]
                },
                &tx
            )
            .is_err()
        );
        assert!(forward(NetMsg::Note("diagnostic".into()), &tx).is_err());
    }
}
