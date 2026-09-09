//! Snapshot-transition application without mission or application-service authority.

use crate::host::{
    CommittedSnapshotTransition, HostTransport, PendingSnapshotTransition,
    PendingSnapshotTransitionPayload,
};
use crate::main_entry::{PendingLevelLoad, PreparedLoad, SaveLoadRequest};
use robin_engine::engine::Engine;
use robin_engine::game_operation::{GameCode, GameOperationState};
use robin_engine::multiplayer::SnapshotTransitionId;
use robin_engine::player_command::PlayerId;

pub(super) fn apply_committed_transition(
    transition: CommittedSnapshotTransition,
    engine: &mut Engine,
    operation: &mut GameOperationState,
    pending_load: &mut Option<PendingLevelLoad>,
) -> Result<GameCode, String> {
    let transition_id = transition.id();
    if transition.is_save() {
        let load = PreparedLoad::from_committed_snapshot(transition)
            .map_err(|error| format!("committed load admission failed: {error:#}"))?;
        let target_mission_id = load.mission_id();
        *pending_load = Some(PendingLevelLoad::new(load));
        operation.set(GameCode::LevelLoad);
        tracing::info!(
            ?transition_id,
            target_mission_id,
            "multiplayer: authoritative load committed; rebuilding mission transport"
        );
        Ok(GameCode::LevelLoad)
    } else {
        match transition.into_payload() {
            PendingSnapshotTransitionPayload::CampaignExit {
                exit_code,
                engine: replacement,
            } => {
                if let Some(replacement) = replacement {
                    *engine = *replacement;
                }
                operation.set(exit_code);
                tracing::info!(
                    ?transition_id,
                    ?exit_code,
                    "multiplayer: host campaign transition committed"
                );
                Ok(exit_code)
            }
            PendingSnapshotTransitionPayload::Save { .. } => {
                unreachable!("save transition handled above")
            }
        }
    }
}

/// Returns the request to queue before the caller logs the transition wait.
/// Queue ownership stays outside transport; this function cannot replace an
/// application operation or acquire broader callback authority.
pub(super) fn begin_deferred_campaign_exit(
    transport: &mut HostTransport,
    engine: &Engine,
    frame: u32,
    pending_request_absent: bool,
) -> Option<(SaveLoadRequest, SnapshotTransitionId)> {
    let pending = transport.take_campaign_exit_at(frame)?;
    assert_eq!(
        transport.local_seat(),
        PlayerId::HOST,
        "only the host may publish a campaign-exit snapshot"
    );
    assert!(
        !transport.has_snapshot_transition() && !transport.reconnecting(),
        "campaign exit reached its snapshot boundary during another transition"
    );
    assert!(
        pending_request_absent,
        "campaign exit cannot overwrite another pending save/load request"
    );
    let engine_bytes = engine.encode_native_snapshot();
    let id = transport
        .net()
        .expect("deferred multiplayer campaign exit lost its transport")
        .begin_campaign_exit_transition(GameCode::LevelInterrupted, engine_bytes)
        .unwrap_or_else(|error| panic!("failed to begin multiplayer campaign transition: {error}"));
    transport.prepare_snapshot_transition(PendingSnapshotTransition::new(
        id,
        PendingSnapshotTransitionPayload::CampaignExit {
            exit_code: GameCode::LevelInterrupted,
            engine: None,
        },
    ));
    Some((
        SaveLoadRequest::Sherwood {
            mission_id: pending.mission_id,
        },
        id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::engine::LevelAssets;
    use robin_engine::multiplayer::MultiplayerSessionId;

    fn engine() -> Engine {
        Engine::new_for_test(640.0, 480.0, Default::default(), &mut LevelAssets::new()).unwrap()
    }

    fn committed_campaign_exit(replacement: Option<Engine>) -> CommittedSnapshotTransition {
        let id = SnapshotTransitionId {
            session_id: MultiplayerSessionId([7; 32]),
            sequence: 3,
        };
        let mut pending = PendingSnapshotTransition::new(
            id,
            PendingSnapshotTransitionPayload::CampaignExit {
                exit_code: GameCode::LevelInterrupted,
                engine: replacement.map(Box::new),
            },
        );
        pending.commit_authenticated(id).unwrap();
        let mut transport = HostTransport::default();
        transport.prepare_snapshot_transition(pending);
        transport.take_committed_snapshot_transition().unwrap()
    }

    #[test]
    fn committed_host_exit_keeps_the_current_engine() {
        let mut engine = engine();
        let before = engine.encode_native_snapshot();
        let mut operation = GameOperationState::new();
        let mut pending_load = None;
        let exit = apply_committed_transition(
            committed_campaign_exit(None),
            &mut engine,
            &mut operation,
            &mut pending_load,
        )
        .unwrap();
        assert_eq!(exit, GameCode::LevelInterrupted);
        assert_eq!(operation.get_current(), exit);
        assert_eq!(engine.encode_native_snapshot(), before);
        assert!(pending_load.is_none());
    }

    #[test]
    fn committed_client_exit_installs_the_authoritative_engine() {
        let mut current = engine();
        let mut replacement = engine();
        replacement.test_set_frame_counter(41);
        let expected = replacement.encode_native_snapshot();
        assert_ne!(current.encode_native_snapshot(), expected);
        let mut operation = GameOperationState::new();
        let mut pending_load = None;
        assert_eq!(
            apply_committed_transition(
                committed_campaign_exit(Some(replacement)),
                &mut current,
                &mut operation,
                &mut pending_load,
            )
            .unwrap(),
            GameCode::LevelInterrupted
        );
        assert_eq!(operation.get_current(), GameCode::LevelInterrupted);
        assert_eq!(current.encode_native_snapshot(), expected);
        assert!(pending_load.is_none());
    }

    #[test]
    fn deferred_exit_before_boundary_retains_its_request_without_transport() {
        let mut transport = HostTransport::default();
        transport.defer_campaign_exit(crate::main_entry::PendingMultiplayerCampaignExit {
            not_before_frame: 8,
            mission_id: 12,
        });
        assert!(begin_deferred_campaign_exit(&mut transport, &engine(), 7, false).is_none());
        assert_eq!(transport.pending_campaign_exit().unwrap().mission_id, 12);
        assert!(!transport.has_snapshot_transition());
        assert!(!transport.reconnecting());
    }
}
