//! Non-blocking mission-end leaderboard browsing and recorded-replay upload.
//!
//! The game-loop owner calls [`MissionEndLeaderboardController::poll`] once
//! per rendered frame and feeds it explicit [`MissionEndLeaderboardAction`]s.
//! No method in this module awaits a network request or performs a synchronous
//! HTTP operation. Boards are available after wins, losses, and interrupted
//! attempts; a submission is offered only for a prepared eligible recording.
//!
//! One upload is one replay signed by the uploader's durable identity: the
//! backend signs the exact [`SubmissionV3`] with the current wall-clock time
//! and posts it together with the canonical compact replay in one request.
//! Every attempt (including a user retry) signs afresh, so a retry never
//! reuses a signature that may have aged out of the server's window.
//! The server re-simulates the replay; nothing here is trusted as a result.

use crate::leaderboard::task::PollTask;
use crate::leaderboard_preferences::{LeaderboardPreferences, LeaderboardTab};
use crate::leaderboard_service::{LeaderboardApi, decode_board, decode_submission_accepted};
use crate::leaderboard_signing::{GameIdentitySigner as _, PlatformSigner};
use robin_run_protocol::{
    ArtifactRefV1, BoardMetricV1, Digest32, LeaderboardPageV2, LeaderboardQueryV2, OpaqueId,
    ParticipantPublicDisclosureV1, PublicKey32, RANKED_REPLAY_MEDIA_TYPE_V1, ReplayArtifactV1,
    SCHEMA_VERSION_V3, SubmissionAcceptedV1, SubmissionV3, TickDurationV1, Validate as _,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Mission-end work reports failures as display strings.
pub use crate::leaderboard::task::TryTake as MissionEndTask;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionEndOutcome {
    Won,
    Lost,
    Interrupted,
}

impl MissionEndOutcome {
    pub(crate) fn from_replay(replay: &robin_engine::replay::ReplayData) -> Self {
        use robin_engine::{game_operation::GameCode, player_command::PlayerCommand};
        for ordinal in (0..replay.frame_count()).rev() {
            let frame = replay.frame(ordinal).expect("validated replay frame");
            for input in frame
                .input
                .commands
                .iter()
                .chain(&frame.input.post_commands)
                .rev()
            {
                if let PlayerCommand::ApplyQuitMissionUpdates { exit_code, .. } =
                    &input.player_input().command
                {
                    return if *exit_code == GameCode::LevelSucceeded {
                        Self::Won
                    } else {
                        Self::Lost
                    };
                }
            }
        }
        Self::Interrupted
    }

    pub const fn can_submit(self) -> bool {
        matches!(self, Self::Won)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionEndBoard {
    pub tab: LeaderboardTab,
    pub label: String,
    pub query: LeaderboardQueryV2,
}

impl MissionEndBoard {
    fn validate(&self) -> Result<(), MissionEndLeaderboardError> {
        if self.label.is_empty()
            || self.label.len() > 64
            || self.label.chars().any(char::is_control)
        {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "board labels must contain 1..=64 safe bytes".to_owned(),
            ));
        }
        self.query
            .validate()
            .map_err(|error| MissionEndLeaderboardError::InvalidRunBundle(error.to_string()))
    }
}

/// Everything locally selected for one upload except the signing time and
/// the signature: the board chosen from published metadata and the
/// exact identity of the canonical compact replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionEndSubmissionInput {
    pub board_id: OpaqueId,
    pub mission_id: String,
    pub requested_metrics: Vec<BoardMetricV1>,
    pub public_disclosure: ParticipantPublicDisclosureV1,
    pub replay: ReplayArtifactV1,
    /// Content identity of the recording, used for local submission links.
    pub replay_session_id: Digest32,
}

impl MissionEndSubmissionInput {
    pub(crate) fn validate(&self) -> Result<(), MissionEndLeaderboardError> {
        self.replay.validate().map_err(invalid_bundle)?;
        if self.mission_id.is_empty() || self.replay_session_id.is_zero() {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "submission needs a mission and a recording identity".to_owned(),
            ));
        }
        if self.requested_metrics.is_empty()
            || !self
                .requested_metrics
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "requested metrics must be non-empty and strictly sorted".to_owned(),
            ));
        }
        Ok(())
    }

    /// The exact document the uploader signs at `signed_at_unix_ms`.
    pub fn submission(
        &self,
        uploader_public_key: PublicKey32,
        signed_at_unix_ms: u64,
    ) -> SubmissionV3 {
        SubmissionV3 {
            schema_version: SCHEMA_VERSION_V3,
            uploader_public_key,
            signed_at_unix_ms,
            public_disclosure: self.public_disclosure,
            board_id: self.board_id.clone(),
            mission_id: self.mission_id.clone(),
            replay: self.replay.clone(),
            requested_metrics: self.requested_metrics.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionEndRunBundle {
    pub outcome: MissionEndOutcome,
    pub multiplayer: bool,
    /// Duration of one simulation tick published with the board metadata.
    pub tick_duration: TickDurationV1,
    pub boards: Vec<MissionEndBoard>,
    /// Present only when this recording may be offered for upload.
    pub eligible_submission: Option<MissionEndSubmissionInput>,
    pub submission_unavailable_reason: Option<String>,
}

impl MissionEndRunBundle {
    pub fn validate(&self) -> Result<(), MissionEndLeaderboardError> {
        self.tick_duration.validate().map_err(invalid_bundle)?;
        if self.boards.is_empty() {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "mission-end view requires at least one board".to_owned(),
            ));
        }
        let mut tabs = Vec::new();
        for board in &self.boards {
            board.validate()?;
            if tabs.contains(&board.tab) {
                return Err(MissionEndLeaderboardError::InvalidRunBundle(
                    "mission-end board tabs must be unique".to_owned(),
                ));
            }
            tabs.push(board.tab);
        }
        if let Some(input) = &self.eligible_submission {
            input.validate()?;
        }
        if self.eligible_submission.is_some() && self.submission_unavailable_reason.is_some() {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "submission cannot be both eligible and unavailable".to_owned(),
            ));
        }
        if self
            .submission_unavailable_reason
            .as_ref()
            .is_some_and(|reason| {
                reason.is_empty() || reason.len() > 500 || reason.chars().any(char::is_control)
            })
        {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "submission-unavailable reason must be a safe bounded message".to_owned(),
            ));
        }
        Ok(())
    }
}

/// A replay upload admitted into the server's verification queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuedSubmission {
    pub accepted: SubmissionAcceptedV1,
    /// Identity that signed the upload and owns its private status.
    pub uploader_public_key: PublicKey32,
}

pub trait MissionEndLeaderboardBackend {
    fn registration(&mut self) -> Result<crate::leaderboard_registration::Registration, String>;
    fn board(
        &mut self,
        query: LeaderboardQueryV2,
    ) -> Result<Box<dyn MissionEndTask<LeaderboardPageV2>>, String>;

    /// Sign and upload `replay` for `input`.
    fn submit(
        &mut self,
        input: MissionEndSubmissionInput,
        replay: Arc<[u8]>,
    ) -> Result<Box<dyn MissionEndTask<QueuedSubmission>>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardLoadState {
    Loading,
    Ready(LeaderboardPageV2),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionSubmissionState {
    WonMissionRequired,
    Unavailable(String),
    AwaitingConsent,
    Submitting,
    Queued(SubmissionAcceptedV1),
    Failed(String),
}

impl MissionSubmissionState {
    pub const fn is_busy(&self) -> bool {
        matches!(self, Self::Submitting)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionEndLeaderboardAction {
    SelectTab(LeaderboardTab),
    RetryBoard,
    SubmitThisRun,
    RetrySubmission,
    SetAlwaysSubmitRuns(bool),
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissionEndLeaderboardEvent {
    None,
    PreferencesChanged,
    Closed,
}

#[derive(Debug, thiserror::Error)]
pub enum MissionEndLeaderboardError {
    #[error("invalid mission-end leaderboard bundle: {0}")]
    InvalidRunBundle(String),
    #[error("ranked submission failed: {0}")]
    Submission(String),
    #[error("ranked replay export failed: {0}")]
    ReplayExport(String),
    #[error("leaderboard preference update failed: {0}")]
    Preferences(String),
}

enum SubmissionTask {
    Dormant(MissionSubmissionState),
    Submitting(Box<dyn MissionEndTask<QueuedSubmission>>),
    /// Upload succeeded; this owner survives until durable receipt handoff.
    Queued {
        queued: QueuedSubmission,
        presentation: MissionSubmissionState,
        persisted: bool,
    },
}

impl SubmissionTask {
    fn presentation(&self) -> &MissionSubmissionState {
        match self {
            Self::Dormant(state) => state,
            Self::Submitting(_) => &MissionSubmissionState::Submitting,
            Self::Queued { presentation, .. } => presentation,
        }
    }
}

enum BoardTask {
    Unrequested,
    Loading(Box<dyn MissionEndTask<LeaderboardPageV2>>),
    Complete(BoardLoadState),
}

impl BoardTask {
    fn presentation(&self) -> &BoardLoadState {
        match self {
            Self::Unrequested | Self::Loading(_) => &BoardLoadState::Loading,
            Self::Complete(state) => state,
        }
    }
}

pub struct MissionEndLeaderboardController {
    history_tracking: bool,
    run: MissionEndRunBundle,
    replay: Arc<[u8]>,
    preferences: LeaderboardPreferences,
    selected_tab: LeaderboardTab,
    board_task: BoardTask,
    submission_task: SubmissionTask,
    registration: crate::leaderboard_registration::RegistrationHandle,
    backend: Box<dyn MissionEndLeaderboardBackend>,
    closed: bool,
}

/// Session-lifetime owner for consented mission-end work after its local
/// presentation has closed. Keeping this outside the mission UI means a level
/// transition never waits for HTTP while the upload continues to be polled
/// cooperatively.
#[derive(Default)]
pub struct MissionEndLeaderboardBackground {
    active: Vec<MissionEndLeaderboardController>,
}

impl MissionEndLeaderboardBackground {
    pub fn adopt(&mut self, controller: MissionEndLeaderboardController) {
        assert!(
            controller.is_closed(),
            "only a closed mission-end controller can leave its presentation owner"
        );
        assert!(
            controller.requires_background_work(),
            "detached mission-end controller must own in-flight or unpersisted verification work"
        );
        self.active.push(controller);
    }

    /// Poll all detached tasks once without blocking the current game frame.
    pub fn poll(&mut self, application_context: &crate::host::ApplicationContext) {
        self.poll_with_receipt_sink(|handoff| {
            application_context.enqueue_leaderboard_receipt_watch(handoff)
        });
    }

    fn poll_with_receipt_sink(
        &mut self,
        mut enqueue: impl FnMut(
            crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch,
        ) -> Result<bool, String>,
    ) {
        let mut index = 0;
        while index < self.active.len() {
            self.active[index].poll();
            if let Err(error) = self.active[index].persist_queued_receipt_watch_with(&mut enqueue) {
                tracing::error!(
                    "queued leaderboard verification could not be handed to durable tracking: {error}"
                );
                index += 1;
                continue;
            }
            if self.active[index].can_retire_after_close() {
                let controller = self.active.swap_remove(index);
                match controller.submission_state() {
                    MissionSubmissionState::Queued(accepted) => tracing::info!(
                        submission_id = %accepted.submission_id,
                        "background leaderboard submission was accepted"
                    ),
                    MissionSubmissionState::Failed(error) => {
                        tracing::warn!("background leaderboard submission failed: {error}")
                    }
                    other => tracing::warn!(
                        ?other,
                        "background leaderboard controller retired in an unexpected state"
                    ),
                }
            } else {
                index += 1;
            }
        }
    }

    #[cfg(test)]
    pub fn active_count(&self) -> usize {
        self.active.len()
    }
}

impl MissionEndLeaderboardController {
    /// `replay` must be the exact canonical compact bytes named by the
    /// bundle's eligible submission.
    pub fn new(
        run: MissionEndRunBundle,
        replay: Arc<[u8]>,
        preferences: LeaderboardPreferences,
        backend: Box<dyn MissionEndLeaderboardBackend>,
    ) -> Result<Self, MissionEndLeaderboardError> {
        run.validate()?;
        preferences
            .validate()
            .map_err(|error| MissionEndLeaderboardError::Preferences(error.to_string()))?;
        if let Some(input) = &run.eligible_submission
            && (u64::try_from(replay.len()).ok() != Some(input.replay.artifact.byte_length)
                || Digest32::digest_bytes(&replay) != input.replay.artifact.sha256)
        {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "replay bytes differ from the artifact selected for upload".to_owned(),
            ));
        }
        let selected_tab = run
            .boards
            .iter()
            .find(|board| board.tab == preferences.preferred_tab)
            .or_else(|| run.boards.first())
            .expect("run validation requires one board")
            .tab;
        let submission_state = if run.eligible_submission.is_some() {
            MissionSubmissionState::AwaitingConsent
        } else if let Some(reason) = &run.submission_unavailable_reason {
            MissionSubmissionState::Unavailable(reason.clone())
        } else if !run.outcome.can_submit() {
            MissionSubmissionState::WonMissionRequired
        } else {
            MissionSubmissionState::Unavailable(
                "this run was not recorded as rank-eligible".to_owned(),
            )
        };
        let mut controller = Self {
            history_tracking: false,
            run,
            replay,
            preferences,
            selected_tab,
            board_task: BoardTask::Unrequested,
            submission_task: SubmissionTask::Dormant(submission_state),
            registration: Default::default(),
            backend,
            closed: false,
        };
        if controller.preferences.show_mission_end_boards {
            controller.start_selected_board();
        }
        if controller
            .preferences
            .automatically_submit(controller.run.outcome.can_submit())
            && controller.run.eligible_submission.is_some()
        {
            controller.start_submission();
        }
        Ok(controller)
    }

    pub fn is_visible(&self) -> bool {
        (self.preferences.show_mission_end_boards || self.needs_registration()) && !self.closed
    }

    pub(crate) fn registration_handle(
        &self,
    ) -> crate::leaderboard_registration::RegistrationHandle {
        self.registration.clone()
    }

    pub(crate) fn set_registration_handle(
        &mut self,
        handle: crate::leaderboard_registration::RegistrationHandle,
    ) {
        assert!(
            !self.needs_registration(),
            "cannot replace active registration"
        );
        self.registration = handle;
    }

    pub(crate) fn needs_registration(&self) -> bool {
        robin_util::sync::lock(&self.registration).is_some()
    }

    pub fn is_multiplayer(&self) -> bool {
        self.run.multiplayer
    }

    pub fn outcome(&self) -> MissionEndOutcome {
        self.run.outcome
    }

    pub fn tick_duration(&self) -> TickDurationV1 {
        self.run.tick_duration
    }

    pub fn boards(&self) -> &[MissionEndBoard] {
        &self.run.boards
    }

    pub fn selected_tab(&self) -> LeaderboardTab {
        self.selected_tab
    }

    pub fn board_state(&self) -> &BoardLoadState {
        self.board_task.presentation()
    }

    pub fn submission_state(&self) -> &MissionSubmissionState {
        self.submission_task.presentation()
    }

    pub fn preferences(&self) -> &LeaderboardPreferences {
        &self.preferences
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Whether a consented upload is still in flight. A presentation owner
    /// may dismiss the overlay and keep polling this controller until false.
    pub fn has_pending_submission(&self) -> bool {
        self.needs_registration() || matches!(self.submission_task, SubmissionTask::Submitting(_))
    }

    pub fn can_retire_after_close(&self) -> bool {
        self.closed && !self.has_pending_submission() && !self.has_unpersisted_receipt_watch()
    }

    pub fn requires_background_work(&self) -> bool {
        self.has_pending_submission() || self.has_unpersisted_receipt_watch()
    }

    /// Durably enqueue owner-status polling once `POST /submissions` has
    /// accepted the replay into verification. Idempotent; called both while
    /// the panel remains open and by the detached background owner.
    pub fn persist_queued_receipt_watch(
        &mut self,
        application_context: &crate::host::ApplicationContext,
    ) -> Result<bool, String> {
        self.persist_queued_receipt_watch_with(|handoff| {
            application_context.enqueue_leaderboard_receipt_watch(handoff)
        })
    }

    pub(crate) fn enable_history_tracking(&mut self) {
        self.history_tracking = true;
    }

    pub(crate) fn persist_queued_receipt_watch_with(
        &mut self,
        enqueue: impl FnOnce(
            crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch,
        ) -> Result<bool, String>,
    ) -> Result<bool, String> {
        let SubmissionTask::Queued {
            queued,
            persisted: false,
            ..
        } = &self.submission_task
        else {
            return Ok(false);
        };
        let handoff =
            crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch::from_accepted(
                &queued.accepted,
                queued.uploader_public_key,
            )
            .map_err(|error| error.to_string())?;
        if self.history_tracking {
            let input = self.run.eligible_submission.as_ref().ok_or_else(|| {
                "queued leaderboard submission lost its recorded input".to_owned()
            })?;
            super::history::persist_link(input, &self.preferences, &queued.accepted, &handoff)?;
        }
        enqueue(handoff)?;
        if let SubmissionTask::Queued { persisted, .. } = &mut self.submission_task {
            *persisted = true;
        }
        Ok(true)
    }

    fn has_unpersisted_receipt_watch(&self) -> bool {
        matches!(
            self.submission_task,
            SubmissionTask::Queued {
                persisted: false,
                ..
            }
        )
    }

    /// Advance each active task by one non-blocking poll.
    pub fn poll(&mut self) {
        self.poll_board();
        self.poll_registration();
        self.poll_submission();
    }

    pub fn apply_action(
        &mut self,
        action: MissionEndLeaderboardAction,
    ) -> Result<MissionEndLeaderboardEvent, MissionEndLeaderboardError> {
        match action {
            MissionEndLeaderboardAction::SelectTab(tab) => {
                if !self.run.boards.iter().any(|board| board.tab == tab) {
                    return Err(MissionEndLeaderboardError::InvalidRunBundle(
                        "selected board tab is unavailable for this run".to_owned(),
                    ));
                }
                let previous_tab = self.selected_tab;
                let previous_preferences = self.preferences.clone();
                self.selected_tab = tab;
                self.preferences.preferred_tab = tab;
                if let Err(error) = self.persist_preferences() {
                    self.selected_tab = previous_tab;
                    self.preferences = previous_preferences;
                    return Err(error);
                }
                self.start_selected_board();
                Ok(MissionEndLeaderboardEvent::PreferencesChanged)
            }
            MissionEndLeaderboardAction::RetryBoard => {
                self.start_selected_board();
                Ok(MissionEndLeaderboardEvent::None)
            }
            MissionEndLeaderboardAction::SubmitThisRun => {
                if !matches!(
                    self.submission_state(),
                    MissionSubmissionState::AwaitingConsent
                ) {
                    return Err(MissionEndLeaderboardError::Submission(
                        "this run is not awaiting upload consent".to_owned(),
                    ));
                }
                self.start_submission();
                Ok(MissionEndLeaderboardEvent::None)
            }
            MissionEndLeaderboardAction::RetrySubmission => {
                if !matches!(self.submission_state(), MissionSubmissionState::Failed(_)) {
                    return Err(MissionEndLeaderboardError::Submission(
                        "only a failed eligible submission can be retried".to_owned(),
                    ));
                }
                self.start_submission();
                Ok(MissionEndLeaderboardEvent::None)
            }
            MissionEndLeaderboardAction::SetAlwaysSubmitRuns(enabled) => {
                let previous_preferences = self.preferences.clone();
                self.preferences.always_submit_eligible_runs = enabled;
                if let Err(error) = self.persist_preferences() {
                    self.preferences = previous_preferences;
                    return Err(error);
                }
                if enabled
                    && matches!(
                        self.submission_state(),
                        MissionSubmissionState::AwaitingConsent
                    )
                {
                    self.start_submission();
                }
                Ok(MissionEndLeaderboardEvent::PreferencesChanged)
            }
            MissionEndLeaderboardAction::Close => {
                if robin_util::sync::lock(&self.registration).take().is_some() {
                    self.submission_task =
                        SubmissionTask::Dormant(MissionSubmissionState::AwaitingConsent);
                }
                self.closed = true;
                Ok(MissionEndLeaderboardEvent::Closed)
            }
        }
    }

    fn persist_preferences(&self) -> Result<(), MissionEndLeaderboardError> {
        crate::leaderboard_preferences::persist(&self.preferences)
            .map_err(|error| MissionEndLeaderboardError::Preferences(error.to_string()))
    }

    fn start_selected_board(&mut self) {
        let query = self
            .run
            .boards
            .iter()
            .find(|board| board.tab == self.selected_tab)
            .expect("selected tab is kept inside the validated board set")
            .query
            .clone();
        self.board_task = match self.backend.board(query) {
            Ok(task) => BoardTask::Loading(task),
            Err(error) => BoardTask::Complete(BoardLoadState::Failed(error)),
        };
    }

    fn poll_board(&mut self) {
        let BoardTask::Loading(task) = &mut self.board_task else {
            return;
        };
        let Some(result) = task.try_take() else {
            return;
        };
        self.board_task = BoardTask::Complete(match result {
            Ok(page) => BoardLoadState::Ready(page),
            Err(error) => BoardLoadState::Failed(error),
        });
    }

    fn start_submission(&mut self) {
        if self.run.eligible_submission.is_none() {
            self.upload_submission();
            return;
        }
        match self.backend.registration() {
            Ok(registration) => {
                *robin_util::sync::lock(&self.registration) = Some(registration);
                self.submission_task = SubmissionTask::Dormant(MissionSubmissionState::Submitting);
                self.poll_registration();
            }
            Err(error) => {
                self.submission_task =
                    SubmissionTask::Dormant(MissionSubmissionState::Failed(error))
            }
        }
    }

    fn poll_registration(&mut self) {
        let ready = {
            let mut slot = robin_util::sync::lock(&self.registration);
            let Some(registration) = slot.as_mut() else {
                return;
            };
            if registration.cancelled {
                *slot = None;
                self.submission_task = SubmissionTask::Dormant(MissionSubmissionState::Failed(
                    "Replay submission cancelled.".into(),
                ));
                return;
            }
            let ready = registration.poll();
            if ready {
                *slot = None;
            }
            ready
        };
        if ready {
            self.upload_submission();
        }
    }

    fn upload_submission(&mut self) {
        let Some(input) = self.run.eligible_submission.clone() else {
            self.submission_task = SubmissionTask::Dormant(MissionSubmissionState::Unavailable(
                self.run
                    .submission_unavailable_reason
                    .clone()
                    .unwrap_or_else(|| "this run is not rank-eligible".to_owned()),
            ));
            return;
        };
        self.submission_task = match self.backend.submit(input, Arc::clone(&self.replay)) {
            Ok(task) => SubmissionTask::Submitting(task),
            Err(error) => SubmissionTask::Dormant(MissionSubmissionState::Failed(error)),
        };
    }

    fn poll_submission(&mut self) {
        let SubmissionTask::Submitting(task) = &mut self.submission_task else {
            return;
        };
        let Some(result) = task.try_take() else {
            return;
        };
        self.submission_task = match result {
            Ok(queued) => SubmissionTask::Queued {
                presentation: MissionSubmissionState::Queued(queued.accepted.clone()),
                queued,
                persisted: false,
            },
            Err(error) => SubmissionTask::Dormant(MissionSubmissionState::Failed(error)),
        };
    }
}

fn invalid_bundle(error: impl std::fmt::Display) -> MissionEndLeaderboardError {
    MissionEndLeaderboardError::InvalidRunBundle(error.to_string())
}

/// Identity of canonical compact replay bytes, after checking that they
/// decode, re-encode byte-identically and record `expected_mission_id`.
/// Input taints are preserved: the server decides rankability.
pub(crate) fn canonical_replay_artifact(
    bytes: &[u8],
    expected_mission_id: &str,
) -> Result<ReplayArtifactV1, MissionEndLeaderboardError> {
    let limits = robin_replay_format::ReplayAdmissionLimits {
        max_input_bytes: bytes.len(),
        ..Default::default()
    };
    let (engine_hash, replay) = robin_replay_format::decode_compact_bounded(bytes, &limits)
        .map_err(|error| MissionEndLeaderboardError::ReplayExport(error.to_string()))?;
    let canonical = robin_replay_format::encode_compact(&replay, &engine_hash)
        .map_err(|error| MissionEndLeaderboardError::ReplayExport(error.to_string()))?;
    if canonical.as_slice() != bytes {
        return Err(MissionEndLeaderboardError::ReplayExport(
            "replay bytes are not their canonical compact re-encoding".to_owned(),
        ));
    }
    if replay.header().mission_id != expected_mission_id {
        return Err(MissionEndLeaderboardError::ReplayExport(
            "compact replay records a different mission".to_owned(),
        ));
    }
    Ok(ReplayArtifactV1 {
        artifact: ArtifactRefV1 {
            sha256: Digest32::digest_bytes(bytes),
            byte_length: u64::try_from(bytes.len()).map_err(|_| {
                MissionEndLeaderboardError::ReplayExport(
                    "compact replay length does not fit the protocol".to_owned(),
                )
            })?,
            media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
        },
        replay_schema_version: replay.header().version,
    })
}

/// Production backend adapter over the bounded native/browser HTTP client
/// and the platform identity signer.
pub struct HttpMissionEndLeaderboardBackend {
    api: LeaderboardApi,
}

impl HttpMissionEndLeaderboardBackend {
    pub fn new(api: LeaderboardApi) -> Self {
        Self { api }
    }
}

impl MissionEndLeaderboardBackend for HttpMissionEndLeaderboardBackend {
    fn registration(&mut self) -> Result<crate::leaderboard_registration::Registration, String> {
        crate::leaderboard_registration::Registration::new(self.api.clone())
    }
    fn board(
        &mut self,
        query: LeaderboardQueryV2,
    ) -> Result<Box<dyn MissionEndTask<LeaderboardPageV2>>, String> {
        let task = self.api.board(&query).map_err(|error| error.to_string())?;
        Ok(Box::new(task.map(move |result| {
            decode_board(result, &query).map_err(|error| error.to_string())
        })))
    }

    fn submit(
        &mut self,
        input: MissionEndSubmissionInput,
        replay: Arc<[u8]>,
    ) -> Result<Box<dyn MissionEndTask<QueuedSubmission>>, String> {
        let api = self.api.clone();
        let task = PollTask::spawn_background("leaderboard-submit", move || async move {
            upload_recorded_replay(&api, &input, replay).await
        })
        .map_err(|error| format!("failed to start leaderboard upload: {error}"))?;
        Ok(Box::new(task.into_try_take(|| {
            "leaderboard upload worker stopped unexpectedly".to_owned()
        })))
    }
}

/// Sign (at the current wall-clock time) and upload one recorded replay.
// TODO: a retry after a transport error whose first upload actually reached
// the server is rejected as a duplicate replay hash; there is no owner lookup
// by replay hash yet, so the retry surfaces as a failed submission.
async fn upload_recorded_replay(
    api: &LeaderboardApi,
    input: &MissionEndSubmissionInput,
    replay: Arc<[u8]>,
) -> Result<QueuedSubmission, String> {
    let uploader_public_key = PlatformSigner::public_key()
        .await
        .map_err(|error| error.to_string())?;
    let signed_at_unix_ms =
        crate::leaderboard_receipt_watcher::now_unix_ms().map_err(|error| error.to_string())?;
    let signed =
        PlatformSigner::sign_submission(input.submission(uploader_public_key, signed_at_unix_ms))
            .await
            .map_err(|error| error.to_string())?;
    let upload = api
        .submit(&signed, replay)
        .map_err(|error| error.to_string())?;
    let accepted =
        decode_submission_accepted(upload.take().await).map_err(|error| error.to_string())?;
    Ok(QueuedSubmission {
        accepted,
        uploader_public_key,
    })
}

/// Active bounded recorder export. Snapshotting shares complete spool chunks;
/// parsing and compact-bitcode encoding run off the graphical call stack.
// TODO: only the replay-service export back-pressure tests use this adapter
// now; move it next to those tests.
#[cfg(test)]
#[derive(Serialize, Deserialize)]
pub struct ActiveMissionReplayExporter {
    exports: crate::replay_service::ReplayExports,
}

#[cfg(test)]
pub trait MissionEndReplayExporter {
    fn begin(&mut self) -> Result<Box<dyn MissionEndTask<Arc<[u8]>>>, String>;
}

#[cfg(test)]
impl ActiveMissionReplayExporter {
    pub fn new(exports: crate::replay_service::ReplayExports) -> Self {
        Self { exports }
    }
}

#[cfg(test)]
struct ReplayExportTask(crate::replay_service::ExportResult);

#[cfg(test)]
impl MissionEndTask<Arc<[u8]>> for ReplayExportTask {
    fn try_take(&mut self) -> Option<Result<Arc<[u8]>, String>> {
        match self.0.try_recv() {
            Ok(result) => Some(
                result
                    .map(|compact| Arc::<[u8]>::from(compact))
                    .map_err(|error| error.to_string()),
            ),
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => Some(Err(
                "ranked replay export task stopped unexpectedly".to_owned(),
            )),
        }
    }
}

#[cfg(test)]
impl MissionEndReplayExporter for ActiveMissionReplayExporter {
    fn begin(&mut self) -> Result<Box<dyn MissionEndTask<Arc<[u8]>>>, String> {
        let snapshot = self.exports.snapshot()?;
        Ok(Box::new(ReplayExportTask(
            self.exports.export_snapshot(snapshot),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leaderboard::test_fixtures::{MISSION_ID, compact_replay_bytes};
    use ed25519_dalek::Signer as _;
    use robin_run_protocol::{SCHEMA_VERSION_V2, SignatureAlgorithmV1, SignedSubmissionV3};
    use robin_run_protocol::{Signature64, SubmissionLifecycleV1};
    use std::sync::Mutex;

    const SIGNED_AT_UNIX_MS: u64 = 1_800_000_000_000;

    #[test]
    fn replay_exporter_diagnostics_cannot_restore_export_authority() {
        let service = Arc::new(crate::replay_service::ReplayService::default());
        let exporter = ActiveMissionReplayExporter::new(service.exports());
        let diagnostic = serde_json::to_string(&exporter).unwrap();
        assert!(serde_json::from_str::<ActiveMissionReplayExporter>(&diagnostic).is_err());
    }

    struct ImmediateTask<T>(Option<Result<T, String>>);

    impl<T> MissionEndTask<T> for ImmediateTask<T> {
        fn try_take(&mut self) -> Option<Result<T, String>> {
            self.0.take()
        }
    }

    struct DelayedTask<T> {
        remaining_polls: usize,
        result: Option<Result<T, String>>,
    }

    impl<T> MissionEndTask<T> for DelayedTask<T> {
        fn try_take(&mut self) -> Option<Result<T, String>> {
            if self.remaining_polls > 0 {
                self.remaining_polls -= 1;
                None
            } else {
                self.result.take()
            }
        }
    }

    #[derive(Default)]
    struct BackendCalls {
        boards: usize,
        uploads: usize,
        registration_checks: usize,
        registration_required: bool,
        registration_error: Option<String>,
    }

    struct TestBackend {
        page: LeaderboardPageV2,
        calls: Arc<Mutex<BackendCalls>>,
        upload_delay_polls: usize,
    }

    fn uploader() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[31; 32])
    }

    fn uploader_public_key() -> PublicKey32 {
        PublicKey32::from_bytes(uploader().verifying_key().to_bytes())
    }

    impl MissionEndLeaderboardBackend for TestBackend {
        fn registration(
            &mut self,
        ) -> Result<crate::leaderboard_registration::Registration, String> {
            let mut calls = self.calls.lock().unwrap();
            calls.registration_checks += 1;
            Ok(crate::leaderboard_registration::Registration::test_result(
                calls
                    .registration_error
                    .clone()
                    .map_or(Ok(!calls.registration_required), Err),
            ))
        }
        fn board(
            &mut self,
            _query: LeaderboardQueryV2,
        ) -> Result<Box<dyn MissionEndTask<LeaderboardPageV2>>, String> {
            self.calls.lock().unwrap().boards += 1;
            Ok(Box::new(ImmediateTask(Some(Ok(self.page.clone())))))
        }

        fn submit(
            &mut self,
            input: MissionEndSubmissionInput,
            replay: Arc<[u8]>,
        ) -> Result<Box<dyn MissionEndTask<QueuedSubmission>>, String> {
            assert_eq!(
                Digest32::digest_bytes(&replay),
                input.replay.artifact.sha256
            );
            let submission = input.submission(uploader_public_key(), SIGNED_AT_UNIX_MS);
            let signature =
                uploader().sign(&SignedSubmissionV3::signing_bytes(&submission).unwrap());
            let signed = SignedSubmissionV3 {
                schema_version: SCHEMA_VERSION_V2,
                request: submission,
                algorithm: SignatureAlgorithmV1::Ed25519,
                signature: Signature64::from_bytes(signature.to_bytes()),
            };
            crate::leaderboard::signing::verify_signed_request_for_tests(&signed)
                .map_err(|error| error.to_string())?;
            self.calls.lock().unwrap().uploads += 1;
            Ok(Box::new(DelayedTask {
                remaining_polls: self.upload_delay_polls,
                result: Some(Ok(QueuedSubmission {
                    accepted: SubmissionAcceptedV1 {
                        schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
                        submission_id: OpaqueId::new("submission-1").unwrap(),
                        state: SubmissionLifecycleV1::Queued,
                        retry_after_ms: 250,
                    },
                    uploader_public_key: uploader_public_key(),
                })),
            }))
        }
    }

    fn query(metric: BoardMetricV1) -> LeaderboardQueryV2 {
        LeaderboardQueryV2 {
            schema_version: SCHEMA_VERSION_V2,
            board_id: OpaqueId::new("demo-standard-normal").unwrap(),
            mission_id: MISSION_ID.to_owned(),
            metric,
            max_concurrent_players: Some(1),
            player_public_key: None,
            limit: 50,
            cursor: None,
        }
    }

    fn board_page(query: &LeaderboardQueryV2) -> LeaderboardPageV2 {
        LeaderboardPageV2 {
            schema_version: SCHEMA_VERSION_V2,
            filter: query.filter(),
            accepted_sequence_watermark: 0,
            previous_cursor: None,
            entries: Vec::new(),
            next_cursor: None,
        }
    }

    struct Fixture {
        bundle: MissionEndRunBundle,
        compact: Arc<[u8]>,
    }

    fn fixture(outcome: MissionEndOutcome) -> Fixture {
        let compact: Arc<[u8]> = compact_replay_bytes().into();
        let eligible_submission = outcome.can_submit().then(|| MissionEndSubmissionInput {
            board_id: OpaqueId::new("demo-standard-normal").unwrap(),
            mission_id: MISSION_ID.to_owned(),
            requested_metrics: vec![BoardMetricV1::OriginalScore],
            public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
            replay: canonical_replay_artifact(&compact, MISSION_ID).unwrap(),
            replay_session_id: Digest32::from_bytes([9; 32]),
        });
        Fixture {
            bundle: MissionEndRunBundle {
                outcome,
                multiplayer: false,
                tick_duration: TickDurationV1 {
                    numerator_micros: 40_000,
                    denominator: 1,
                },
                boards: vec![MissionEndBoard {
                    tab: LeaderboardTab::Score,
                    label: "Score".to_owned(),
                    query: query(BoardMetricV1::OriginalScore),
                }],
                eligible_submission,
                submission_unavailable_reason: None,
            },
            compact,
        }
    }

    fn controller_with(
        fixture: Fixture,
        preferences: LeaderboardPreferences,
        calls: Arc<Mutex<BackendCalls>>,
        upload_delay_polls: usize,
    ) -> MissionEndLeaderboardController {
        let page = board_page(&fixture.bundle.boards[0].query);
        MissionEndLeaderboardController::new(
            fixture.bundle,
            fixture.compact,
            preferences,
            Box::new(TestBackend {
                page,
                calls,
                upload_delay_polls,
            }),
        )
        .unwrap()
    }

    fn controller(
        fixture: Fixture,
        preferences: LeaderboardPreferences,
        calls: Arc<Mutex<BackendCalls>>,
    ) -> MissionEndLeaderboardController {
        controller_with(fixture, preferences, calls, 0)
    }

    #[test]
    fn manual_and_automatic_uploads_share_registration_even_with_hidden_boards() {
        for automatic in [false, true] {
            let calls = Arc::new(Mutex::new(BackendCalls {
                registration_required: true,
                ..Default::default()
            }));
            let preferences = LeaderboardPreferences {
                show_mission_end_boards: !automatic,
                always_submit_eligible_runs: automatic,
                ..Default::default()
            };
            let mut controller =
                controller(fixture(MissionEndOutcome::Won), preferences, calls.clone());
            if !automatic {
                assert_eq!(calls.lock().unwrap().registration_checks, 0);
                controller
                    .apply_action(MissionEndLeaderboardAction::SubmitThisRun)
                    .unwrap();
            }
            assert!(
                controller.is_visible(),
                "hidden automatic uploads must keep the prompt visible"
            );
            assert!(controller.needs_registration());
            assert!(controller.has_pending_submission());
            assert_eq!(calls.lock().unwrap().uploads, 0);
            controller.poll();
            assert_eq!(calls.lock().unwrap().uploads, 0, "wait for a chosen name");
            *robin_util::sync::lock(&controller.registration_handle()) = Some(
                crate::leaderboard_registration::Registration::test_result(Ok(true)),
            );
            controller.poll();
            controller.poll();
            assert_eq!(calls.lock().unwrap().uploads, 1);
            assert!(!controller.needs_registration());
            assert_eq!(controller.is_visible(), !automatic);
            assert!(matches!(
                controller.submission_state(),
                MissionSubmissionState::Queued(_)
            ));
        }
    }

    #[test]
    fn shared_archive_prompt_cancels_and_retry_checks_registration_again() {
        let calls = Arc::new(Mutex::new(BackendCalls {
            registration_required: true,
            ..Default::default()
        }));
        let mut controller = controller(
            fixture(MissionEndOutcome::Won),
            LeaderboardPreferences::default(),
            calls.clone(),
        );
        let handle = Default::default();
        controller.set_registration_handle(Arc::clone(&handle));
        controller
            .apply_action(MissionEndLeaderboardAction::SubmitThisRun)
            .unwrap();
        robin_util::sync::lock(&handle).as_mut().unwrap().cancel();
        controller.poll();
        assert!(matches!(
            controller.submission_state(),
            MissionSubmissionState::Failed(_)
        ));
        assert_eq!(calls.lock().unwrap().uploads, 0);
        controller
            .apply_action(MissionEndLeaderboardAction::RetrySubmission)
            .unwrap();
        assert!(robin_util::sync::lock(&handle).as_ref().unwrap().needs_name);
        assert_eq!(calls.lock().unwrap().registration_checks, 2);
        controller
            .apply_action(MissionEndLeaderboardAction::Close)
            .unwrap();
        controller.poll();
        assert!(!controller.requires_background_work());
        assert_eq!(calls.lock().unwrap().uploads, 0);
    }

    #[test]
    fn automatic_registration_failure_stays_visible_and_never_uploads() {
        let calls = Arc::new(Mutex::new(BackendCalls {
            registration_error: Some("offline".into()),
            ..Default::default()
        }));
        let mut controller = controller(
            fixture(MissionEndOutcome::Won),
            LeaderboardPreferences {
                show_mission_end_boards: false,
                always_submit_eligible_runs: true,
                ..Default::default()
            },
            calls.clone(),
        );
        controller.poll();
        assert!(controller.is_visible());
        assert_eq!(
            robin_util::sync::lock(&controller.registration_handle())
                .as_ref()
                .unwrap()
                .message,
            "offline"
        );
        assert_eq!(calls.lock().unwrap().uploads, 0);
        controller
            .apply_action(MissionEndLeaderboardAction::Close)
            .unwrap();
        assert!(controller.can_retire_after_close());
    }

    #[test]
    fn boards_open_for_failed_missions_but_submission_never_does() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let mut controller = controller(
            fixture(MissionEndOutcome::Lost),
            LeaderboardPreferences::default(),
            Arc::clone(&calls),
        );
        assert!(controller.is_visible());
        assert_eq!(
            controller.submission_state(),
            &MissionSubmissionState::WonMissionRequired
        );
        assert!(matches!(controller.board_state(), BoardLoadState::Loading));
        controller.poll();
        assert!(matches!(controller.board_state(), BoardLoadState::Ready(_)));
        let calls = calls.lock().unwrap();
        assert_eq!(calls.boards, 1);
        assert_eq!(calls.uploads, 0);
    }

    #[test]
    fn presentation_toggle_hides_only_the_board_and_does_not_invent_work() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let preferences = LeaderboardPreferences {
            show_mission_end_boards: false,
            ..LeaderboardPreferences::default()
        };
        let mut controller = controller(
            fixture(MissionEndOutcome::Lost),
            preferences,
            Arc::clone(&calls),
        );
        assert!(!controller.is_visible());
        controller.poll();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.boards, 0);
        assert_eq!(calls.uploads, 0);
    }

    #[test]
    fn replay_bytes_must_match_the_selected_artifact() {
        let mut fixture = fixture(MissionEndOutcome::Won);
        let mut other = compact_replay_bytes();
        other.push(b'\n');
        fixture.compact = other.into();
        let page = board_page(&fixture.bundle.boards[0].query);
        assert!(matches!(
            MissionEndLeaderboardController::new(
                fixture.bundle,
                fixture.compact,
                LeaderboardPreferences::default(),
                Box::new(TestBackend {
                    page,
                    calls: Default::default(),
                    upload_delay_polls: 0,
                }),
            ),
            Err(MissionEndLeaderboardError::InvalidRunBundle(_))
        ));
    }

    #[test]
    fn default_consent_is_off_and_explicit_consent_uploads_once() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let mut controller = controller(
            fixture(MissionEndOutcome::Won),
            LeaderboardPreferences::default(),
            Arc::clone(&calls),
        );
        assert_eq!(
            controller.submission_state(),
            &MissionSubmissionState::AwaitingConsent
        );
        assert_eq!(calls.lock().unwrap().uploads, 0);
        controller
            .apply_action(MissionEndLeaderboardAction::SubmitThisRun)
            .unwrap();
        assert_eq!(
            controller.submission_state(),
            &MissionSubmissionState::Submitting
        );
        controller.poll();
        assert!(matches!(
            controller.submission_state(),
            MissionSubmissionState::Queued(_)
        ));
        assert_eq!(calls.lock().unwrap().uploads, 1);
        assert!(
            controller
                .apply_action(MissionEndLeaderboardAction::SubmitThisRun)
                .is_err()
        );
    }

    #[test]
    fn dismissing_does_not_cancel_a_consented_submission() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let mut controller = controller_with(
            fixture(MissionEndOutcome::Won),
            LeaderboardPreferences::default(),
            calls,
            2,
        );
        controller
            .apply_action(MissionEndLeaderboardAction::SubmitThisRun)
            .unwrap();
        assert!(controller.has_pending_submission());
        assert_eq!(
            controller
                .apply_action(MissionEndLeaderboardAction::Close)
                .unwrap(),
            MissionEndLeaderboardEvent::Closed
        );
        assert!(controller.is_closed());
        assert!(!controller.can_retire_after_close());

        for _ in 0..3 {
            controller.poll();
        }
        assert!(!controller.has_pending_submission());
        assert!(!controller.can_retire_after_close());
        let mut handed_off = None;
        assert!(
            controller
                .persist_queued_receipt_watch_with(|handoff| {
                    handed_off = Some(handoff);
                    Ok(true)
                })
                .unwrap()
        );
        assert_eq!(
            handed_off.unwrap().key.controller_public_key,
            uploader_public_key()
        );
        assert!(controller.can_retire_after_close());
        assert!(
            !controller
                .persist_queued_receipt_watch_with(|_| panic!("already persisted"))
                .unwrap()
        );
    }

    #[test]
    fn closed_delayed_upload_moves_to_session_owner_without_blocking_transition() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let mut controller = controller_with(
            fixture(MissionEndOutcome::Won),
            LeaderboardPreferences {
                show_mission_end_boards: false,
                always_submit_eligible_runs: true,
                ..LeaderboardPreferences::default()
            },
            Arc::clone(&calls),
            4,
        );
        controller
            .apply_action(MissionEndLeaderboardAction::Close)
            .unwrap();

        let mut background = MissionEndLeaderboardBackground::default();
        background.adopt(controller);
        let mut handed_off = 0;
        for _ in 0..4 {
            background.poll_with_receipt_sink(|_| {
                handed_off += 1;
                Ok(true)
            });
            assert_eq!(background.active_count(), 1);
        }
        background.poll_with_receipt_sink(|_| {
            handed_off += 1;
            Ok(true)
        });
        assert_eq!(background.active_count(), 0);
        assert_eq!(handed_off, 1);
        assert_eq!(calls.lock().unwrap().uploads, 1);
    }

    #[test]
    fn always_submit_is_explicit_and_starts_only_eligible_wins() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let preferences = LeaderboardPreferences {
            always_submit_eligible_runs: true,
            ..LeaderboardPreferences::default()
        };
        let won = controller(
            fixture(MissionEndOutcome::Won),
            preferences.clone(),
            Arc::clone(&calls),
        );
        assert_eq!(won.submission_state(), &MissionSubmissionState::Submitting);
        let lost = controller(
            fixture(MissionEndOutcome::Interrupted),
            preferences,
            Arc::clone(&calls),
        );
        assert_eq!(
            lost.submission_state(),
            &MissionSubmissionState::WonMissionRequired
        );
        assert_eq!(calls.lock().unwrap().uploads, 1);
    }

    #[test]
    fn submission_document_carries_the_exact_local_selection() {
        let fixture = fixture(MissionEndOutcome::Won);
        let input = fixture.bundle.eligible_submission.unwrap();
        let submission = input.submission(uploader_public_key(), SIGNED_AT_UNIX_MS);
        submission.validate().unwrap();
        assert_eq!(submission.signed_at_unix_ms, SIGNED_AT_UNIX_MS);
        assert_eq!(submission.board_id, input.board_id);
        assert_eq!(submission.replay, input.replay);
        assert_eq!(submission.requested_metrics, input.requested_metrics);
        assert_eq!(submission.uploader_public_key, uploader_public_key());
    }

    #[test]
    fn canonical_artifact_preserves_taints_for_server_verification() {
        let compact = compact_replay_bytes();
        let text = &compact;
        let (engine_hash, mut replay) = robin_replay_format::decode_compact(text).unwrap();
        replay
            .try_edit_header(|header| {
                header.rankability.taint(
                    robin_engine::replay_rankability::InputTaintKind::HttpPlayerCommand,
                    0,
                )
            })
            .unwrap();
        let tainted = robin_replay_format::encode_compact(&replay, &engine_hash).unwrap();
        let artifact = canonical_replay_artifact(&tainted, MISSION_ID).unwrap();
        assert_eq!(artifact.artifact.sha256, Digest32::digest_bytes(&tainted));
        assert!(replay.ranked_submission_verdict().is_err());
        assert!(canonical_replay_artifact(&tainted, "Demo_Lin").is_err());
        let mut padded = tainted;
        padded.push(b' ');
        assert!(canonical_replay_artifact(&padded, MISSION_ID).is_err());
    }
}
