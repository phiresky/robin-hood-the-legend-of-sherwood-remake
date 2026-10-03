//! Prepare mission-end leaderboard uploads from the recording itself.
use crate::ingame_menu::layout::{
    MenuTransform, dim_screen, draw_screen_background, enter_modal_gpu_phase, render_text_virt_font,
};
use crate::ingame_menu::widget_bridge::ModalCursor;
use crate::ingame_menu::{IngameMenuResources, MissionEndLeaderboardScreen};
use crate::leaderboard_mission_end::{
    HttpMissionEndLeaderboardBackend, MissionEndBoard, MissionEndLeaderboardAction,
    MissionEndLeaderboardController, MissionEndLeaderboardEvent, MissionEndOutcome,
    MissionEndRunBundle, MissionEndSubmissionInput, canonical_replay_artifact,
};
use crate::leaderboard_preferences::{LeaderboardPreferences, LeaderboardTab};
use crate::leaderboard_service::{DEFAULT_BOARD_PAGE_LIMIT, LeaderboardApi};
use crate::renderer::Renderer;
use robin_engine::campaign::Campaign;
use robin_run_protocol::{
    BoardMetricV1, BoardV2, LeaderboardQueryV2, OfficialContentEditionV1,
    ParticipantPublicDisclosureV1, SCHEMA_VERSION_V2,
};
use std::sync::Arc;
mod board;
mod error;
use error::RankedError;
mod presentation;
pub(super) use presentation::{MissionEndLeaderboardTaskProgress, MissionEndLeaderboardTaskState};

/// Official content edition of the installed datadir. Boards are published
/// per edition; the verifier re-simulates against the matching raw content.
pub(crate) fn installed_content_edition(
    application_context: &crate::host::ApplicationContext,
) -> OfficialContentEditionV1 {
    if crate::main_entry::detect_demo_mode_with_context(application_context).is_some() {
        OfficialContentEditionV1::Demo
    } else {
        OfficialContentEditionV1::Full
    }
}

/// Encode the recording, select its board from published metadata and build
/// the browse/upload bundle together with the exact replay bytes to upload.
/// Runs restored without complete input history retain leaderboard browsing
/// but have no upload artifact or submission action.
pub(crate) async fn prepare_recorded_submission(
    replay: &robin_engine::replay::ReplayData,
    preferences: &LeaderboardPreferences,
    edition: OfficialContentEditionV1,
) -> Result<(MissionEndRunBundle, Arc<[u8]>), String> {
    prepare(replay, preferences, edition)
        .await
        .map_err(|error| error.to_string())
}

async fn prepare(
    replay: &robin_engine::replay::ReplayData,
    preferences: &LeaderboardPreferences,
    edition: OfficialContentEditionV1,
) -> Result<(MissionEndRunBundle, Arc<[u8]>), RankedError> {
    let header = replay.header();
    let unavailable = replay_submission_unavailable_reason(replay)?;
    // Check reconstructibility before the bounded ranked codec: a local save
    // snapshot may be much larger than a ranked command collection.
    let (artifact, bytes): (_, Arc<[u8]>) = if unavailable.is_none() {
        let bytes: Arc<[u8]> =
            crate::replay_format::encode_compact(replay, robin_replay_format::ENGINE_VERSION_HASH)
                .map_err(|error| RankedError::evidence(error.to_string()))?
                .into();
        (
            Some(canonical_replay_artifact(&bytes, &header.mission_id)?),
            bytes,
        )
    } else {
        (None, Arc::from([]))
    };
    let api = LeaderboardApi::from_preferences(preferences)?;
    let metadata = crate::leaderboard_service::decode_metadata(api.metadata()?.take().await)?;
    let board = board::select_board(&metadata, edition, &header.mission_id, header.sim_config)?;
    let replay_session_id = replay.submission_id();
    // Participation counts come from the recorded seat events; other seats
    // stay anonymous and only the uploader signs.
    let transcript = replay
        .submission_transcript(replay_session_id, replay_session_id)
        .map_err(RankedError::evidence)?;
    let boards = metric_boards(board, &header.mission_id, transcript.max_concurrent_players);
    if boards.is_empty() {
        return Err(RankedError::unavailable(format!(
            "leaderboard board `{}` publishes no supported metric",
            board.board_id
        )));
    }
    let bundle = MissionEndRunBundle {
        outcome: MissionEndOutcome::from_replay(replay),
        multiplayer: transcript.max_concurrent_players > 1,
        tick_duration: metadata.tick_duration,
        boards,
        eligible_submission: artifact.map(|artifact| MissionEndSubmissionInput {
            board_id: board.board_id.clone(),
            mission_id: header.mission_id.clone(),
            requested_metrics: board.metrics.clone(),
            // TODO: expose an anonymous-upload preference; uploads are named.
            public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
            replay: artifact,
            replay_session_id,
        }),
        submission_unavailable_reason: unavailable,
    };
    bundle.validate()?;
    Ok((bundle, bytes))
}

fn replay_submission_unavailable_reason(
    replay: &robin_engine::replay::ReplayData,
) -> Result<Option<String>, RankedError> {
    let rankability = replay
        .rankability()
        .map_err(|error| RankedError::evidence(error.to_string()))?;
    Ok(rankability.taints().iter().any(|taint|
        taint.kind == robin_engine::replay_rankability::InputTaintKind::StateLoad
    ).then(|| "This save's complete replay history is unavailable. The local replay can be watched, but this run cannot be submitted.".to_owned()))
}

/// One tab per supported metric of `board`, filtered to this recording's
/// mission and concurrent player count.
fn metric_boards(board: &BoardV2, mission_id: &str, max_players: u16) -> Vec<MissionEndBoard> {
    [
        (BoardMetricV1::OriginalScore, LeaderboardTab::Score, "Score"),
        (BoardMetricV1::FastestSuccess, LeaderboardTab::Time, "Time"),
    ]
    .into_iter()
    .filter(|(metric, _, _)| board.metrics.contains(metric))
    .map(|(metric, tab, label)| MissionEndBoard {
        tab,
        label: label.to_owned(),
        query: LeaderboardQueryV2 {
            schema_version: SCHEMA_VERSION_V2,
            board_id: board.board_id.clone(),
            mission_id: mission_id.to_owned(),
            metric,
            max_concurrent_players: Some(max_players),
            player_public_key: None,
            include_metrics: None,
            limit: DEFAULT_BOARD_PAGE_LIMIT,
            cursor: None,
        },
    })
    .collect()
}

/// One presentation per completed attempt; no network work runs at mission launch.
pub(super) struct MissionLeaderboardRuntime {
    preparation: Option<MissionEndPreparation>,
}
impl MissionLeaderboardRuntime {
    pub(super) fn new() -> Self {
        let (preferences, error) = match crate::leaderboard_preferences::load() {
            Ok(preferences) => (preferences, None),
            Err(error) => (LeaderboardPreferences::default(), Some(error.to_string())),
        };
        Self {
            preparation: Some(MissionEndPreparation {
                preferences,
                upload: error.map(|error| {
                    crate::leaderboard::task::PollTask::start(async move { Err(error) })
                }),
            }),
        }
    }
    pub(super) fn after_state_restore(&mut self, _campaign: &Campaign) {
        if self.preparation.is_none() {
            *self = Self::new();
        }
    }
    pub(super) fn capture_terminal(
        &mut self,
        _outcome: MissionEndOutcome,
    ) -> Result<MissionEndPreparation, RankedError> {
        self.preparation.take().ok_or_else(|| {
            RankedError::lifecycle("mission-end leaderboard was captured more than once")
        })
    }
}
pub(super) struct MissionEndPreparation {
    preferences: LeaderboardPreferences,
    upload: Option<
        crate::leaderboard::task::PollTask<Result<(MissionEndRunBundle, Arc<[u8]>), String>>,
    >,
}
impl MissionEndPreparation {
    pub(super) fn preferences(&self) -> &LeaderboardPreferences {
        &self.preferences
    }

    /// Poll once. `None` means the bounded HTTP task is still in flight.
    pub(super) fn poll_bundle(
        &mut self,
        replay_exports: &crate::replay_service::ReplayExports,
        edition: OfficialContentEditionV1,
    ) -> Option<Result<(MissionEndRunBundle, Arc<[u8]>), RankedError>> {
        if self.upload.is_none() {
            let snapshot = match replay_exports.snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => return Some(Err(RankedError::evidence(error))),
            };
            let preferences = self.preferences.clone();
            let task = crate::leaderboard::task::PollTask::spawn_background(
                "prepare-replay-upload",
                move || async move {
                    let replay = snapshot.parse_sync()?;
                    let (mut bundle, bytes) =
                        prepare_recorded_submission(&replay, &preferences, edition).await?;
                    if !bundle.outcome.can_submit() {
                        bundle.eligible_submission = None;
                    }
                    Ok((bundle, bytes))
                },
            );
            self.upload = match task {
                Ok(task) => Some(task),
                Err(error) => return Some(Err(RankedError::unavailable(error.to_string()))),
            };
        }
        self.upload
            .as_ref()
            .expect("upload task initialized")
            .poll(|| "Replay upload preparation stopped unexpectedly".to_owned())
            .map(|result| result.map_err(RankedError::unavailable))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::replay::{ReplayData, ReplayFile, ReplayLoadBack, ReplaySaveSnapshot};

    #[test]
    fn oversized_local_restore_is_explained_before_ranked_encoding() {
        let replay = crate::leaderboard::test_fixtures::single_frame_replay(bitcode::encode(
            &Campaign::default(),
        ));
        assert!(
            replay_submission_unavailable_reason(&replay)
                .unwrap()
                .is_none()
        );
        let mut file = ReplayFile::from(&replay);
        file.load_backs.insert(
            0,
            ReplayLoadBack {
                snapshot: Some(ReplaySaveSnapshot {
                    payload: vec![0; 4_077_629],
                    timeline_frame: 0,
                }),
                to_frame: 0,
                is_continue: false,
            },
        );
        let replay = ReplayData::try_from(file).unwrap();
        assert!(
            replay_submission_unavailable_reason(&replay)
                .unwrap()
                .unwrap()
                .contains("complete replay history is unavailable")
        );
    }
}
