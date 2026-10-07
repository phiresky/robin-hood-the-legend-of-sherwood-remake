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
    let boards = metric_boards(
        board,
        &header.mission_id,
        Some(transcript.max_concurrent_players),
    );
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
fn metric_boards(
    board: &BoardV2,
    mission_id: &str,
    max_players: Option<u16>,
) -> Vec<MissionEndBoard> {
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
            max_concurrent_players: max_players,
            player_public_key: None,
            include_metrics: None,
            limit: DEFAULT_BOARD_PAGE_LIMIT,
            cursor: None,
        },
    })
    .collect()
}

/// Browse completed missions that cannot provide a ranked upload artifact.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct BrowseLeaderboardContext {
    mission_id: String,
    sim_config: robin_engine::engine::SimConfig,
    outcome: MissionEndOutcome,
}

impl BrowseLeaderboardContext {
    async fn prepare(
        &self,
        preferences: &LeaderboardPreferences,
        edition: OfficialContentEditionV1,
    ) -> Result<(MissionEndRunBundle, Arc<[u8]>), RankedError> {
        let api = LeaderboardApi::from_preferences(preferences)?;
        let metadata = crate::leaderboard_service::decode_metadata(api.metadata()?.take().await)?;
        Ok((self.bundle(&metadata, edition)?, Arc::from([])))
    }

    fn bundle(
        &self,
        metadata: &robin_run_protocol::LeaderboardMetadataV2,
        edition: OfficialContentEditionV1,
    ) -> Result<MissionEndRunBundle, RankedError> {
        let board = if self.sim_config.coop.players > 1 {
            board::select_coop_browsing_board(metadata, edition, &self.mission_id)?
        } else {
            board::select_board(metadata, edition, &self.mission_id, self.sim_config)?
        };
        let bundle = MissionEndRunBundle {
            outcome: self.outcome,
            multiplayer: true,
            tick_duration: metadata.tick_duration,
            // Browsing does not inspect a participant transcript.
            // Show all team sizes without inventing a participant count.
            boards: metric_boards(board, &self.mission_id, None),
            eligible_submission: None,
            submission_unavailable_reason: Some(if self.sim_config.coop.players > 1 {
                "Co-op runs cannot be submitted to the current leaderboards. You can browse this mission's scores.".into()
            } else {
                "Only the host can submit this multiplayer run.".into()
            }),
        };
        bundle.validate()?;
        Ok(bundle)
    }
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
                browse: None,
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
        outcome: MissionEndOutcome,
        local_seat: robin_engine::player_command::PlayerId,
        mission: &robin_engine::profiles::MissionProfile,
        sim_config: robin_engine::engine::SimConfig,
    ) -> Result<MissionEndPreparation, RankedError> {
        let mut preparation = self.preparation.take().ok_or_else(|| {
            RankedError::lifecycle("mission-end leaderboard was captured more than once")
        })?;
        if local_seat != robin_engine::player_command::PlayerId::HOST || sim_config.coop.players > 1
        {
            preparation.browse = Some(BrowseLeaderboardContext {
                mission_id: mission.mission_filename.clone(),
                sim_config,
                outcome,
            });
        }
        Ok(preparation)
    }
}
pub(super) struct MissionEndPreparation {
    preferences: LeaderboardPreferences,
    browse: Option<BrowseLeaderboardContext>,
    upload: Option<
        crate::leaderboard::task::PollTask<Result<(MissionEndRunBundle, Arc<[u8]>), String>>,
    >,
}
impl MissionEndPreparation {
    pub(super) fn preferences(&self) -> &LeaderboardPreferences {
        &self.preferences
    }

    fn capture_replay(
        &self,
        exports: &crate::replay_service::ReplayExports,
    ) -> Result<Option<crate::replay_service::ReplaySnapshot>, RankedError> {
        if self.browse.is_some() {
            return Ok(None);
        }
        exports.snapshot().map(Some).map_err(RankedError::evidence)
    }

    /// Poll once. `None` means the bounded HTTP task is still in flight.
    pub(super) fn poll_bundle(
        &mut self,
        replay_exports: &crate::replay_service::ReplayExports,
        edition: OfficialContentEditionV1,
    ) -> Option<Result<(MissionEndRunBundle, Arc<[u8]>), RankedError>> {
        if self.upload.is_none() {
            let snapshot = match self.capture_replay(replay_exports) {
                Ok(snapshot) => snapshot,
                Err(error) => return Some(Err(error)),
            };
            let browse = self.browse.clone();
            let preferences = self.preferences.clone();
            let task = crate::leaderboard::task::PollTask::spawn_background(
                "prepare-replay-upload",
                move || async move {
                    if let Some(browse) = browse {
                        return browse
                            .prepare(&preferences, edition)
                            .await
                            .map_err(|error| error.to_string());
                    }
                    let replay = snapshot
                        .expect("host preparation captures its recording")
                        .parse_sync()?;
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
    fn client_browsing_does_not_require_a_valid_local_replay_or_offer_upload() {
        use crate::leaderboard::test_fixtures::{MISSION_ID, board};
        use robin_engine::player_command::PlayerId;
        use robin_run_protocol::{BoardSimulationPolicyV1, LeaderboardMetadataV2, TickDurationV1};
        use std::io::Write;

        let service = Arc::<crate::replay_service::ReplayService>::default();
        let replay = crate::leaderboard::test_fixtures::single_frame_replay(bitcode::encode(
            &Campaign::default(),
        ));
        let mut writer = service.recording().begin_recording();
        serde_json::to_writer(&mut writer, replay.header()).unwrap();
        writeln!(writer).unwrap();
        let mut recorded = replay.frame(0).unwrap().clone();
        recorded.timeline_after = 1;
        writeln!(writer, "{}", serde_json::json!({"f":0,"i":recorded})).unwrap();
        recorded.timeline_before = 54;
        recorded.timeline_after = 55;
        writeln!(writer, "{}", serde_json::json!({"f":1,"i":recorded})).unwrap();
        writer.flush().unwrap();
        let error = service
            .exports()
            .snapshot()
            .unwrap()
            .parse_sync()
            .unwrap_err();
        assert!(
            error.contains("starts at timeline 54, previous frame ended at 1"),
            "the replay continuity error must remain rejected: {error}"
        );
        let mission = robin_engine::profiles::MissionProfile {
            mission_filename: MISSION_ID.into(),
            ..Default::default()
        };
        let mut config = robin_engine::engine::SimConfig::default();
        config.coop.players = 1;
        let metadata = LeaderboardMetadataV2 {
            schema_version: SCHEMA_VERSION_V2,
            tick_duration: TickDurationV1 {
                numerator_micros: 40_000,
                denominator: 1,
            },
            boards: vec![board(
                "demo-any",
                OfficialContentEditionV1::Demo,
                BoardSimulationPolicyV1::AnyConfig,
            )],
        };
        let mut coop_config = config;
        coop_config.coop.players = 2;
        for seat in [PlayerId::HOST, PlayerId(1)] {
            let mut runtime = MissionLeaderboardRuntime {
                preparation: Some(MissionEndPreparation {
                    preferences: LeaderboardPreferences::default(),
                    browse: None,
                    upload: None,
                }),
            };
            let preparation = runtime
                .capture_terminal(MissionEndOutcome::Won, seat, &mission, coop_config)
                .unwrap();
            assert!(
                preparation
                    .capture_replay(&service.exports())
                    .unwrap()
                    .is_none()
            );
            let bundle = preparation
                .browse
                .unwrap()
                .bundle(&metadata, OfficialContentEditionV1::Demo)
                .unwrap();
            assert!(bundle.eligible_submission.is_none());
            assert!(
                bundle
                    .submission_unavailable_reason
                    .unwrap()
                    .contains("Co-op runs cannot be submitted")
            );
            assert_eq!(bundle.boards.len(), 2);
        }
        for seat in [PlayerId::HOST, PlayerId(1)] {
            let mut runtime = MissionLeaderboardRuntime {
                preparation: Some(MissionEndPreparation {
                    preferences: LeaderboardPreferences::default(),
                    browse: None,
                    upload: None,
                }),
            };
            let preparation = runtime
                .capture_terminal(MissionEndOutcome::Won, seat, &mission, config)
                .unwrap();
            let captured = preparation.capture_replay(&service.exports()).unwrap();
            if seat == PlayerId::HOST {
                assert!(
                    captured.unwrap().parse_sync().is_err(),
                    "the host must still validate its upload evidence"
                );
                assert!(preparation.browse.is_none());
            } else {
                assert!(captured.is_none());
                let bundle = preparation
                    .browse
                    .unwrap()
                    .bundle(&metadata, OfficialContentEditionV1::Demo)
                    .unwrap();
                assert_eq!(bundle.outcome, MissionEndOutcome::Won);
                assert!(bundle.multiplayer);
                assert!(bundle.eligible_submission.is_none());
                assert!(
                    bundle
                        .submission_unavailable_reason
                        .unwrap()
                        .contains("Only the host")
                );
                assert_eq!(bundle.boards.len(), 2);
                assert!(
                    bundle
                        .boards
                        .iter()
                        .all(|board| board.query.mission_id == MISSION_ID
                            && board.query.max_concurrent_players.is_none())
                );
            }
        }
    }

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
