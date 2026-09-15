//! Replay uploads and durable verification links, keyed by recording content.
use super::{mission_end::*, preferences::LeaderboardPreferences};
use robin_run_protocol::{Digest32, SubmissionAcceptedV1};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmissionLink {
    schema_version: u16,
    session: Digest32,
    api_base: String,
    accepted: SubmissionAcceptedV1,
    receipt: crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch,
}

fn store_key(session: Digest32, api_base: &str) -> String {
    let digest = Sha256::digest(format!("{api_base}\n{session}"));
    format!("replay-submission-{}.json", hex::encode(digest))
}

pub(crate) fn persist_link(
    input: &MissionEndSubmissionInput,
    preferences: &LeaderboardPreferences,
    accepted: &SubmissionAcceptedV1,
    receipt: &crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch,
) -> Result<(), String> {
    let session = input.replay_session_id;
    let api_base = preferences
        .effective_api_base_url()
        .map_err(|e| e.to_string())?
        .as_str()
        .to_owned();
    let key = store_key(session, &api_base);
    let bytes = serde_json::to_vec(&SubmissionLink {
        schema_version: 1,
        session,
        api_base,
        accepted: accepted.clone(),
        receipt: receipt.clone(),
    })
    .map_err(|e| e.to_string())?;
    super::store::write(&key, &key, ".replay-submission-", &bytes).map_err(|e| e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn load_link(
    session: Digest32,
    preferences: &LeaderboardPreferences,
) -> Result<Option<SubmissionLink>, String> {
    let api_base = preferences
        .effective_api_base_url()
        .map_err(|e| e.to_string())?
        .as_str()
        .to_owned();
    let key = store_key(session, &api_base);
    let Some(text) = super::store::read(&key, &key).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    decode_link(&text, session, &api_base).map(Some)
}

#[cfg(any(test, not(target_arch = "wasm32")))]
fn decode_link(text: &str, session: Digest32, api_base: &str) -> Result<SubmissionLink, String> {
    let link: SubmissionLink = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if link.schema_version != 1 {
        return Err("Unsupported replay submission link schema".into());
    }
    robin_run_protocol::Validate::validate(&link.accepted).map_err(|e| e.to_string())?;
    if link.receipt.retry_after_ms == 0 {
        return Err("Stored submission has an invalid retry interval".into());
    }
    if link.session != session
        || link.api_base != api_base
        || link.accepted.submission_id != link.receipt.key.submission_id
    {
        return Err("Stored replay submission does not match its session or server".into());
    }
    Ok(link)
}

pub(crate) fn page_url(
    preferences: &LeaderboardPreferences,
    query: &[(&str, &str)],
) -> Result<String, String> {
    let base = preferences
        .effective_api_base_url()
        .map_err(|e| e.to_string())?;
    let mut parameters = url::form_urlencoded::Serializer::new(String::new());
    parameters.extend_pairs(query.iter().copied());
    if base.as_str() == "/api/v1" {
        return Ok(format!("/leaderboards/?{}", parameters.finish()));
    }
    let mut url = url::Url::parse(base.as_str()).map_err(|e| e.to_string())?;
    url.set_path("/leaderboards/");
    url.set_query(Some(&parameters.finish()));
    Ok(url.into())
}

pub(crate) fn open_page(query: &[(&str, &str)]) -> Result<(), String> {
    open_url(&page_url(
        &super::preferences::load().map_err(|e| e.to_string())?,
        query,
    )?)
}

pub(crate) fn open_url(url: &str) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        webbrowser::open(url).map_err(|e| format!("Cannot open leaderboard in browser: {e}"))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window().ok_or("Browser window unavailable")?;
        window
            .open_with_url_and_target(url, "_blank")
            .map_err(|e| format!("Cannot open leaderboard: {e:?}"))?
            .ok_or("Browser blocked the leaderboard window")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReplaySubmissionInfo {
    pub message: String,
    pub can_submit: bool,
    pub url: Option<String>,
}
#[cfg(not(target_arch = "wasm32"))]
impl ReplaySubmissionInfo {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            can_submit: false,
            url: None,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use crate::leaderboard::task::PollTask;
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::Arc,
    };
    type Identity = (
        robin_engine::campaign_history::MissionAttemptKey,
        Option<i64>,
    );
    type Completion = Result<
        (
            ReplaySubmissionInfo,
            Option<crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch>,
        ),
        String,
    >;

    #[derive(Debug, Default, Serialize)]
    pub(crate) struct ReplaySubmissions {
        #[serde(skip)]
        entries: HashMap<PathBuf, Entry>,
    }
    robin_util::deny_deserialize!(
        ReplaySubmissions,
        "replay submission tasks belong to the application"
    );
    #[derive(Debug, Serialize)]
    struct Entry {
        #[serde(skip)]
        info: Arc<std::sync::Mutex<ReplaySubmissionInfo>>,
        retry_submission: bool,
        #[serde(skip)]
        inspected_at: std::time::Instant,
        #[serde(skip)]
        task: Option<PollTask<Completion>>,
        receipt: Option<crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch>,
        #[serde(skip)]
        receipt_retry_at: Option<std::time::Instant>,
    }

    robin_util::deny_deserialize!(Entry, "submission entries contain live worker ownership");

    impl ReplaySubmissions {
        #[cfg(test)]
        pub(crate) fn seed_capture_info(&mut self, path: PathBuf, info: ReplaySubmissionInfo) {
            self.entries.insert(
                path,
                Entry {
                    info: Arc::new(std::sync::Mutex::new(info)),
                    retry_submission: true,
                    inspected_at: std::time::Instant::now(),
                    task: None,
                    receipt: None,
                    receipt_retry_at: None,
                },
            );
        }

        pub(crate) fn info(&mut self, path: &Path) -> ReplaySubmissionInfo {
            let refresh = self.entries.get(path).is_some_and(|entry| {
                entry.task.is_none()
                    && entry.receipt.is_none()
                    && !entry.retry_submission
                    && robin_util::sync::lock(&entry.info).can_submit
                    && entry.inspected_at.elapsed() >= std::time::Duration::from_secs(5)
            });
            if refresh {
                self.entries.remove(path);
            }
            if !self.entries.contains_key(path) {
                if self
                    .entries
                    .values()
                    .filter(|entry| entry.task.is_some())
                    .count()
                    >= 4
                {
                    return ReplaySubmissionInfo::unavailable("Checking submission...");
                }
                let owned = path.to_owned();
                let task =
                    PollTask::spawn_background("replay-submission-info", move || async move {
                        inspect(&owned)
                    });
                let entry = match task {
                    Ok(task) => Entry {
                        info: Arc::new(std::sync::Mutex::new(ReplaySubmissionInfo::unavailable(
                            "Checking submission...",
                        ))),
                        retry_submission: false,
                        inspected_at: std::time::Instant::now(),
                        task: Some(task),
                        receipt: None,
                        receipt_retry_at: None,
                    },
                    Err(error) => Entry {
                        info: Arc::new(std::sync::Mutex::new(ReplaySubmissionInfo::unavailable(
                            format!("Cannot inspect submission: {error}"),
                        ))),
                        retry_submission: false,
                        inspected_at: std::time::Instant::now(),
                        task: None,
                        receipt: None,
                        receipt_retry_at: None,
                    },
                };
                self.entries.insert(path.to_owned(), entry);
            }
            robin_util::sync::lock(&self.entries[path].info).clone()
        }
        pub(crate) fn submit(
            &mut self,
            path: &Path,
            expected: Identity,
            edition: robin_run_protocol::OfficialContentEditionV1,
        ) -> Result<(), String> {
            let entry = self
                .entries
                .get_mut(path)
                .ok_or("Replay submission information is still loading")?;
            if entry.task.is_some() || !robin_util::sync::lock(&entry.info).can_submit {
                return Err("This replay is not ready to submit".into());
            }
            let owned = path.to_owned();
            let progress = entry.info.clone();
            entry.retry_submission = true;
            entry.task = Some(
                PollTask::spawn_background("archive-submit", move || async move {
                    submit(&owned, expected, edition, &progress)
                })
                .map_err(|e| e.to_string())?,
            );
            *robin_util::sync::lock(&entry.info) =
                ReplaySubmissionInfo::unavailable("Submitting replay...");
            Ok(())
        }
        pub(crate) fn poll(&mut self, application: &crate::host::ApplicationContext) {
            for (path, entry) in &mut self.entries {
                if let Some(result) = entry.task.as_ref().and_then(|task| {
                    task.poll(|| "Replay submission worker stopped unexpectedly".into())
                }) {
                    entry.task = None;
                    match result {
                        Ok((info, receipt)) => {
                            *robin_util::sync::lock(&entry.info) = info;
                            entry.receipt = receipt;
                        }
                        Err(error) => {
                            tracing::error!(recording = %path.display(), "Replay submission: {error}");
                            let mut info = robin_util::sync::lock(&entry.info);
                            if info.url.is_some() {
                                info.message = format!("Submitted; local tracking failed: {error}");
                            } else {
                                *info = ReplaySubmissionInfo {
                                    message: error,
                                    can_submit: entry.retry_submission,
                                    url: None,
                                };
                            }
                        }
                    }
                }
                if let Some(receipt) = entry.receipt.as_ref()
                    && entry
                        .receipt_retry_at
                        .is_none_or(|due| std::time::Instant::now() >= due)
                {
                    match application.enqueue_leaderboard_receipt_watch(receipt.clone()) {
                        Ok(_) => entry.receipt = None,
                        Err(error) => {
                            tracing::error!("Cannot track replay verification: {error}");
                            entry.receipt_retry_at =
                                Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
                        }
                    }
                }
            }
        }
    }
    fn recording(path: &Path) -> Result<robin_engine::replay::ReplayData, String> {
        crate::replay_format::load_replay_spec(path.to_str().ok_or("Recording path is not UTF-8")?)
            .map_err(|error| format!("Cannot read replay: {error}"))
    }
    fn submitted(link: SubmissionLink, preferences: &LeaderboardPreferences) -> Completion {
        Ok((
            ReplaySubmissionInfo {
                message: "Submitted / open verification status online".into(),
                can_submit: false,
                url: Some(page_url(
                    preferences,
                    &[("submission", link.accepted.submission_id.as_str())],
                )?),
            },
            Some(link.receipt),
        ))
    }
    fn inspect(path: &Path) -> Completion {
        let replay = recording(path)?;
        let preferences = super::super::preferences::load().map_err(|e| e.to_string())?;
        if let Some(link) = load_link(replay.submission_id(), &preferences)? {
            return submitted(link, &preferences);
        }
        Ok((
            ReplaySubmissionInfo {
                message: "Not submitted".into(),
                can_submit: true,
                url: None,
            },
            None,
        ))
    }
    #[test]
    fn recording_without_ranked_sidecar_is_submittable_even_without_a_win() {
        let replay = crate::leaderboard::test_fixtures::single_frame_replay(bitcode::encode(
            &robin_engine::campaign::Campaign::default(),
        ));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recording.rhrec");
        let bytes =
            crate::replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
                .unwrap();
        std::fs::write(&path, bytes).unwrap();
        assert!(!directory.path().join("ranked.json").exists());
        assert_eq!(
            MissionEndOutcome::from_replay(&replay),
            MissionEndOutcome::Interrupted
        );
        let (info, receipt) = inspect(&path).unwrap();
        assert!(info.can_submit);
        assert!(receipt.is_none());
    }

    #[test]
    fn upload_identity_and_multiplayer_counts_come_from_the_recording() {
        use robin_engine::player_command::{PlayerCommand, PlayerId};
        let replay = crate::leaderboard::test_fixtures::single_frame_replay(bitcode::encode(
            &robin_engine::campaign::Campaign::default(),
        ));
        let mut file = robin_engine::replay::ReplayFile::from(&replay);
        file.frames.get_mut(&0).unwrap().input.commands = vec![
            PlayerCommand::ConnectSeat {
                player_id: PlayerId(1),
                nickname: "Guest".into(),
            }
            .into(),
            PlayerCommand::DisconnectSeat {
                player_id: PlayerId(1),
            }
            .into(),
            PlayerCommand::ConnectSeat {
                player_id: PlayerId(1),
                nickname: "Another guest".into(),
            }
            .into(),
        ];
        let replay = robin_engine::replay::ReplayData::try_from(file).unwrap();
        let transcript = replay
            .submission_transcript(replay.submission_id(), Digest32::from_bytes([1; 32]))
            .unwrap();
        assert_eq!(transcript.max_concurrent_players, 2);
        assert_eq!(transcript.participant_instance_count, 3);
        replay
            .validate_ranked_command_admission(&transcript)
            .unwrap();
        for build in ["123456789abc", "abcdef012345"] {
            let text = crate::replay_format::encode_compact(&replay, build).unwrap();
            let (_, decoded) = robin_replay_format::decode_compact(&text).unwrap();
            assert_eq!(decoded.submission_id(), replay.submission_id());
        }
    }
    fn submit(
        path: &Path,
        expected: Identity,
        edition: robin_run_protocol::OfficialContentEditionV1,
        progress: &std::sync::Mutex<ReplaySubmissionInfo>,
    ) -> Completion {
        let replay = recording(path)?;
        if crate::mission_replays::replay_attempt_identity(&replay)? != Some(expected) {
            return Err("Recording does not match the selected campaign attempt".into());
        }
        let mut preferences = super::super::preferences::load().map_err(|e| e.to_string())?;
        if let Some(link) = load_link(replay.submission_id(), &preferences)? {
            return submitted(link, &preferences);
        }
        let (bundle, bytes) = pollster::block_on(
            crate::game_session::leaderboard_runtime::prepare_recorded_submission(
                &replay,
                &preferences,
                edition,
            ),
        )?;
        let session = bundle
            .eligible_submission
            .as_ref()
            .ok_or("Prepared replay has no submission")?
            .replay_session_id;
        let api = super::super::service::LeaderboardApi::from_preferences(&preferences)
            .map_err(|e| e.to_string())?;
        preferences.show_mission_end_boards = false;
        preferences.always_submit_eligible_runs = false;
        let mut controller = MissionEndLeaderboardController::new(
            bundle,
            bytes,
            preferences.clone(),
            Box::new(HttpMissionEndLeaderboardBackend::new(api)),
        )
        .map_err(|e| e.to_string())?;
        controller.enable_history_tracking();
        controller
            .apply_action(MissionEndLeaderboardAction::SubmitThisRun)
            .map_err(|e| e.to_string())?;
        loop {
            controller.poll();
            match controller.submission_state() {
                MissionSubmissionState::Queued(accepted) => {
                    let url = page_url(
                        &preferences,
                        &[("submission", accepted.submission_id.as_str())],
                    )?;
                    *robin_util::sync::lock(progress) = ReplaySubmissionInfo {
                        message: "Submitted / open verification status online".into(),
                        can_submit: false,
                        url: Some(url),
                    };
                    tracing::info!(submission_id = %accepted.submission_id, "Archived replay accepted into verification");
                    // Keep the accepted ID and controller alive if local storage fails;
                    // retry persistence, never create a second upload.
                    loop {
                        match controller.persist_queued_receipt_watch_with(|_| Ok(true)) {
                            Ok(_) => break,
                            Err(error) => {
                                tracing::error!(
                                    "Cannot persist accepted replay submission; retrying: {error}"
                                );
                                std::thread::sleep(std::time::Duration::from_secs(5));
                            }
                        }
                    }
                    let link = load_link(session, &preferences)?
                        .ok_or("Uploaded replay lost its durable submission link")?;
                    return submitted(link, &preferences);
                }
                MissionSubmissionState::Failed(error)
                | MissionSubmissionState::Unavailable(error) => return Err(error.clone()),
                MissionSubmissionState::WonMissionRequired => {
                    return Err("Only successful mission recordings can be submitted".into());
                }
                _ => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::ReplaySubmissions;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_links_are_partitioned_by_exact_recording_session_and_server() {
        let session = Digest32::from_bytes([3; 32]);
        let origin = "https://robinhood.phiresky.xyz/api/v1";
        assert_ne!(
            store_key(session, origin),
            store_key(Digest32::from_bytes([4; 32]), origin)
        );
        assert_ne!(
            store_key(session, origin),
            store_key(session, "http://localhost:3000/api/v1")
        );
        let key = store_key(session, origin);
        assert!(!key.contains('/') && !key.contains('\\'));
        assert_eq!(key, store_key(session, origin));
    }

    #[test]
    fn durable_link_recovery_checks_the_exact_server_session_and_receipt() {
        use robin_run_protocol::{OpaqueId, PublicKey32, SubmissionLifecycleV1};
        let session = Digest32::from_bytes([5; 32]);
        let accepted = SubmissionAcceptedV1 {
            schema_version: 1,
            submission_id: OpaqueId::new("submission-1").unwrap(),
            state: SubmissionLifecycleV1::Queued,
            retry_after_ms: 1000,
        };
        let link = SubmissionLink {
            schema_version: 1,
            session,
            api_base: "https://example.test/api/v1".into(),
            receipt:
                crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch::from_accepted(
                    &accepted,
                    PublicKey32::from_bytes([7; 32]),
                )
                .unwrap(),
            accepted,
        };
        let text = serde_json::to_string(&link).unwrap();
        assert_eq!(
            decode_link(&text, session, &link.api_base)
                .unwrap()
                .accepted
                .submission_id,
            link.accepted.submission_id
        );
        assert!(decode_link(&text, Digest32::from_bytes([6; 32]), &link.api_base).is_err());
        assert!(decode_link(&text, session, "https://other.test/api/v1").is_err());
        let mut document = serde_json::to_value(&link).unwrap();
        document["receipt"]["key"]["submission_id"] = "another-submission".into();
        assert!(decode_link(&document.to_string(), session, &link.api_base).is_err());
        let mut document = serde_json::to_value(&link).unwrap();
        document["schema_version"] = 2.into();
        assert!(decode_link(&document.to_string(), session, &link.api_base).is_err());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn browser_links_encode_mission_and_submission_as_query_values() {
        let prefs = LeaderboardPreferences::default();
        let page = page_url(
            &prefs,
            &[("subject", "campaign"), ("mission", "mission & other/#")],
        )
        .unwrap();
        let url = url::Url::parse(&page).unwrap();
        assert_eq!(url.path(), "/leaderboards/");
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            [
                ("subject".into(), "campaign".into()),
                ("mission".into(), "mission & other/#".into())
            ]
        );
        assert!(url.fragment().is_none());
        let page = page_url(&prefs, &[("submission", "queued?run=other")]).unwrap();
        let url = url::Url::parse(&page).unwrap();
        assert_eq!(url.query_pairs().count(), 1);
        assert_eq!(url.query_pairs().next().unwrap().1, "queued?run=other");
    }
}
