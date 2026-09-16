//! Public leaderboard, run, and player query documents.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    BoardMetricV1, CanonicalDocument as _, CanonicalValue, Digest32, OfficialContentEditionV1,
    OpaqueId, PublicKey32, ReplayArtifactV1, Validate, ValidationError,
    VerifiedAchievementEvaluationV1, VerifiedAchievementV1, ViewerContentRequirementV2,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunMetricsV1 {
    pub original_score_delta: i64,
    pub active_simulation_ticks: u64,
    /// Signed net mission money, including spending.
    pub ransom_collected: i64,
}

/// Exact ranked value of one run for one board metric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "metric", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoardMetricValueV2 {
    OriginalScore { points: i64 },
    FastestSuccess { active_simulation_ticks: u64 },
}

impl BoardMetricValueV2 {
    pub const fn metric(&self) -> BoardMetricV1 {
        match self {
            Self::OriginalScore { .. } => BoardMetricV1::OriginalScore,
            Self::FastestSuccess { .. } => BoardMetricV1::FastestSuccess,
        }
    }

    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::OriginalScore { points } if !(0..=i64::from(u32::MAX)).contains(points) => {
                Err(ValidationError::InvalidOriginalScore)
            }
            _ => Ok(()),
        }
    }
}

/// Public identity of a named uploader. The short fingerprint is display-only
/// and may collide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicParticipantV1 {
    pub seat: u16,
    pub username: String,
    pub public_key: PublicKey32,
    pub public_key_fingerprint: String,
}

impl Validate for PublicParticipantV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.seat >= crate::MAX_REPLAY_SEATS_V1 {
            return Err(ValidationError::CountOutOfRange {
                field: "public_participant.seat",
            });
        }
        crate::validation::nonzero("public_participant.public_key", &self.public_key)?;
        crate::validation::text("public_participant.username", &self.username, 48)?;
        crate::validation::text(
            "public_participant.public_key_fingerprint",
            &self.public_key_fingerprint,
            80,
        )?;
        if self.public_key_fingerprint != self.public_key.short_fingerprint() {
            return Err(ValidationError::ClaimMismatch {
                field: "public_participant.public_key_fingerprint",
            });
        }
        Ok(())
    }
}

/// Public mutable profile for one durable key identity. Usernames are not
/// unique; UI must display the fingerprint alongside them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerProfileV1 {
    pub schema_version: u32,
    pub username: String,
    pub public_key: PublicKey32,
    pub public_key_fingerprint: String,
}

impl Validate for PlayerProfileV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PlayerProfileV1", self.schema_version)?;
        PublicParticipantV1 {
            seat: 0,
            username: self.username.clone(),
            public_key: self.public_key,
            public_key_fingerprint: self.public_key_fingerprint.clone(),
        }
        .validate()
    }
}

fn validate_participants(
    max_concurrent_players: u16,
    participant_instance_count: u16,
    uploader: Option<&PublicParticipantV1>,
) -> Result<(), ValidationError> {
    if max_concurrent_players == 0
        || max_concurrent_players > crate::MAX_REPLAY_SEATS_V1
        || participant_instance_count < max_concurrent_players
    {
        return Err(ValidationError::EmptyPlayerCount);
    }
    if let Some(uploader) = uploader {
        uploader.validate()?;
        if uploader.seat != 0 {
            return Err(ValidationError::MissingHostClaim);
        }
    }
    Ok(())
}

/// The exact board a query or page addresses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFilterV2 {
    pub schema_version: u32,
    pub board_id: OpaqueId,
    pub mission_id: String,
    pub metric: BoardMetricV1,
    pub max_concurrent_players: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_public_key: Option<PublicKey32>,
}

impl Validate for RunFilterV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "RunFilterV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        crate::validation::text("run_filter.mission_id", &self.mission_id, 256)?;
        if self.max_concurrent_players == Some(0) {
            return Err(ValidationError::EmptyPlayerCount);
        }
        if self.player_public_key.is_some_and(|key| key.is_zero()) {
            return Err(ValidationError::Zero {
                field: "run_filter.player_public_key",
            });
        }
        Ok(())
    }
}

/// Exact flat query document accepted by `GET /api/v1/leaderboards`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardQueryV2 {
    pub schema_version: u32,
    pub board_id: OpaqueId,
    pub mission_id: String,
    pub metric: BoardMetricV1,
    pub max_concurrent_players: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_public_key: Option<PublicKey32>,
    pub limit: u16,
    pub cursor: Option<String>,
}

impl LeaderboardQueryV2 {
    pub fn filter(&self) -> RunFilterV2 {
        RunFilterV2 {
            schema_version: self.schema_version,
            board_id: self.board_id.clone(),
            mission_id: self.mission_id.clone(),
            metric: self.metric,
            max_concurrent_players: self.max_concurrent_players,
            player_public_key: self.player_public_key,
        }
    }
}

impl Validate for LeaderboardQueryV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.filter().validate()?;
        if !(1..=100).contains(&self.limit) {
            return Err(ValidationError::CountOutOfRange {
                field: "leaderboard_query.limit",
            });
        }
        if let Some(cursor) = &self.cursor {
            crate::validation::text("leaderboard_query.cursor", cursor, 4096)?;
        }
        Ok(())
    }
}

/// Verification-derived fields needed to render a run card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSummaryV2 {
    pub schema_version: u32,
    pub run_id: OpaqueId,
    pub board_id: OpaqueId,
    pub mission_id: String,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub uploader: Option<PublicParticipantV1>,
    pub metrics: RunMetricsV1,
}

impl Validate for RunSummaryV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "RunSummaryV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        crate::validation::text("run_summary.mission_id", &self.mission_id, 256)?;
        validate_participants(
            self.max_concurrent_players,
            self.participant_instance_count,
            self.uploader.as_ref(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardEntryV2 {
    /// Stable one-based SQL/window position in this immutable snapshot.
    pub position: u64,
    pub rank: u64,
    pub run_id: OpaqueId,
    pub metric_value: BoardMetricValueV2,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    /// Named uploader, or `None` when the uploader chose anonymous disclosure.
    pub uploader: Option<PublicParticipantV1>,
    pub replay_sha256: Digest32,
    pub accepted_sequence: u64,
    pub verified_at_unix_ms: u64,
}

impl Validate for LeaderboardEntryV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.position == 0 || self.rank == 0 || self.rank > self.position {
            return Err(ValidationError::Zero {
                field: "leaderboard_entry.position/rank",
            });
        }
        if self.accepted_sequence == 0 || self.verified_at_unix_ms == 0 {
            return Err(ValidationError::Zero {
                field: "leaderboard_entry.verified_at_unix_ms",
            });
        }
        crate::validation::nonzero("leaderboard_entry.replay_sha256", &self.replay_sha256)?;
        self.metric_value.validate()?;
        validate_participants(
            self.max_concurrent_players,
            self.participant_instance_count,
            self.uploader.as_ref(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardOrderAnchorV2 {
    pub position: u64,
    pub rank: u64,
    pub metric_value: BoardMetricValueV2,
    pub accepted_sequence: u64,
    pub verified_at_unix_ms: u64,
    pub run_id: OpaqueId,
}

impl LeaderboardOrderAnchorV2 {
    pub fn from_entry(entry: &LeaderboardEntryV2) -> Self {
        Self {
            position: entry.position,
            rank: entry.rank,
            metric_value: entry.metric_value.clone(),
            accepted_sequence: entry.accepted_sequence,
            verified_at_unix_ms: entry.verified_at_unix_ms,
            run_id: entry.run_id.clone(),
        }
    }

    fn validate(&self) -> Result<(), ValidationError> {
        if self.position == 0
            || self.rank == 0
            || self.rank > self.position
            || self.accepted_sequence == 0
            || self.verified_at_unix_ms == 0
        {
            return Err(ValidationError::CountOutOfRange {
                field: "leaderboard_order_anchor",
            });
        }
        self.metric_value.validate()
    }
}

/// Decoded, authenticated cursor contract. `opaque_token` is issued and
/// authenticated by the service; the public fields let clients fail closed on
/// query/snapshot substitution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardCursorV2 {
    pub schema_version: u32,
    pub query_sha256: Digest32,
    pub accepted_sequence_watermark: u64,
    pub last: LeaderboardOrderAnchorV2,
    pub opaque_token: String,
}

impl Validate for LeaderboardCursorV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "LeaderboardCursorV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        if self.query_sha256.is_zero() || self.accepted_sequence_watermark == 0 {
            return Err(ValidationError::Zero {
                field: "leaderboard_cursor.snapshot",
            });
        }
        self.last.validate()?;
        if self.last.accepted_sequence > self.accepted_sequence_watermark {
            return Err(ValidationError::ClaimMismatch {
                field: "leaderboard_cursor.watermark",
            });
        }
        crate::validation::text("leaderboard_cursor.opaque_token", &self.opaque_token, 4096)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardPageV2 {
    pub schema_version: u32,
    pub filter: RunFilterV2,
    pub accepted_sequence_watermark: u64,
    /// Decoded cursor supplied for this page; absent only on the first page.
    pub previous_cursor: Option<LeaderboardCursorV2>,
    pub entries: Vec<LeaderboardEntryV2>,
    pub next_cursor: Option<LeaderboardCursorV2>,
}

impl Validate for LeaderboardPageV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "LeaderboardPageV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        self.filter.validate()?;
        if self.entries.len() > 100 {
            return Err(ValidationError::CountOutOfRange {
                field: "leaderboard.entries",
            });
        }
        if self.entries.is_empty() {
            if self.previous_cursor.is_some() || self.next_cursor.is_some() {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.empty_page_cursor",
                });
            }
            return Ok(());
        }
        if self.accepted_sequence_watermark == 0 {
            return Err(ValidationError::CountOutOfRange {
                field: "leaderboard.watermark",
            });
        }
        let query_sha256 =
            self.filter
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "leaderboard.filter_digest",
                })?;
        if let Some(cursor) = &self.previous_cursor {
            cursor.validate()?;
            if cursor.query_sha256 != query_sha256
                || cursor.accepted_sequence_watermark != self.accepted_sequence_watermark
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.previous_cursor",
                });
            }
        }
        let run_ids = self
            .entries
            .iter()
            .map(|entry| &entry.run_id)
            .collect::<BTreeSet<_>>();
        let sequences = self
            .entries
            .iter()
            .map(|entry| entry.accepted_sequence)
            .collect::<BTreeSet<_>>();
        if run_ids.len() != self.entries.len() || sequences.len() != self.entries.len() {
            return Err(ValidationError::Duplicate {
                field: "leaderboard.entries",
                value: "run_id/accepted_sequence".into(),
            });
        }
        for entry in &self.entries {
            entry.validate()?;
            if entry.metric_value.metric() != self.filter.metric {
                return Err(ValidationError::MetricValueMismatch {
                    field: "leaderboard.entries.metric_value",
                });
            }
            if self
                .filter
                .max_concurrent_players
                .is_some_and(|count| count != entry.max_concurrent_players)
            {
                return Err(ValidationError::CountOutOfRange {
                    field: "leaderboard.entries.max_concurrent_players",
                });
            }
            if entry.accepted_sequence > self.accepted_sequence_watermark {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.entries.accepted_sequence_watermark",
                });
            }
        }
        let mut previous = self
            .previous_cursor
            .as_ref()
            .map(|cursor| cursor.last.clone());
        for entry in &self.entries {
            if let Some(prior) = &previous {
                validate_leaderboard_order(self.filter.metric, prior, entry)?;
            } else if entry.position != 1 || entry.rank != 1 {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.first_entry",
                });
            }
            previous = Some(LeaderboardOrderAnchorV2::from_entry(entry));
        }
        if let Some(cursor) = &self.next_cursor {
            cursor.validate()?;
            if cursor.query_sha256 != query_sha256
                || cursor.accepted_sequence_watermark != self.accepted_sequence_watermark
                || self
                    .entries
                    .last()
                    .map(LeaderboardOrderAnchorV2::from_entry)
                    != Some(cursor.last.clone())
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.next_cursor",
                });
            }
        }
        Ok(())
    }
}

fn primary_metric_order(
    metric: BoardMetricV1,
    left: &BoardMetricValueV2,
    right: &BoardMetricValueV2,
) -> Result<Ordering, ValidationError> {
    match (metric, left, right) {
        (
            BoardMetricV1::OriginalScore,
            BoardMetricValueV2::OriginalScore { points: left },
            BoardMetricValueV2::OriginalScore { points: right },
        ) => Ok(right.cmp(left)),
        (
            BoardMetricV1::FastestSuccess,
            BoardMetricValueV2::FastestSuccess {
                active_simulation_ticks: left,
            },
            BoardMetricValueV2::FastestSuccess {
                active_simulation_ticks: right,
            },
        ) => Ok(left.cmp(right)),
        _ => Err(ValidationError::MetricValueMismatch {
            field: "leaderboard.order.metric",
        }),
    }
}

fn validate_leaderboard_order(
    metric: BoardMetricV1,
    prior: &LeaderboardOrderAnchorV2,
    current: &LeaderboardEntryV2,
) -> Result<(), ValidationError> {
    if current.position != prior.position.saturating_add(1) {
        return Err(ValidationError::ClaimMismatch {
            field: "leaderboard.order.position",
        });
    }
    match primary_metric_order(metric, &prior.metric_value, &current.metric_value)? {
        Ordering::Greater => {
            return Err(ValidationError::ClaimMismatch {
                field: "leaderboard.order.primary_metric",
            });
        }
        Ordering::Equal => {
            if current.rank != prior.rank
                || (
                    current.accepted_sequence,
                    current.verified_at_unix_ms,
                    &current.run_id,
                ) <= (
                    prior.accepted_sequence,
                    prior.verified_at_unix_ms,
                    &prior.run_id,
                )
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.order.tie",
                });
            }
        }
        Ordering::Less => {
            if current.rank != current.position {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.order.rank_skip",
                });
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerRunHistoryQueryV1 {
    pub schema_version: u32,
    pub limit: u16,
    pub cursor: Option<String>,
}

/// Canonical identity of one player's bounded public-history query.
///
/// Pagination cursors bind this document rather than the transport query so
/// that the opaque cursor itself is excluded while the path-owned player key
/// remains part of the authenticated query identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerRunHistoryFilterV1 {
    pub schema_version: u32,
    pub player_public_key: PublicKey32,
    pub limit: u16,
}

impl Validate for PlayerRunHistoryFilterV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PlayerRunHistoryFilterV1", self.schema_version)?;
        crate::validation::nonzero(
            "player_run_history_filter.player_public_key",
            &self.player_public_key,
        )?;
        if !(1..=100).contains(&self.limit) {
            return Err(ValidationError::CountOutOfRange {
                field: "player_run_history_filter.limit",
            });
        }
        Ok(())
    }
}

impl Validate for PlayerRunHistoryQueryV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PlayerRunHistoryQueryV1", self.schema_version)?;
        if !(1..=100).contains(&self.limit) {
            return Err(ValidationError::CountOutOfRange {
                field: "player_run_history_query.limit",
            });
        }
        if let Some(cursor) = &self.cursor {
            crate::validation::text("player_run_history_query.cursor", cursor, 4_096)?;
        }
        Ok(())
    }
}

impl PlayerRunHistoryQueryV1 {
    pub const fn filter_for_player(
        &self,
        player_public_key: PublicKey32,
    ) -> PlayerRunHistoryFilterV1 {
        PlayerRunHistoryFilterV1 {
            schema_version: self.schema_version,
            player_public_key,
            limit: self.limit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerRunHistoryEntryV2 {
    pub player_public_key: PublicKey32,
    pub run: RunSummaryV2,
    pub verified_at_unix_ms: u64,
}

impl Validate for PlayerRunHistoryEntryV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::nonzero(
            "player_run_history_entry.player_public_key",
            &self.player_public_key,
        )?;
        self.run.validate()?;
        crate::validation::nonzero(
            "player_run_history_entry.verified_at_unix_ms",
            &self.verified_at_unix_ms,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerPersonalBestV2 {
    pub filter: RunFilterV2,
    pub run_id: OpaqueId,
    pub metric_value: BoardMetricValueV2,
}

impl Validate for PlayerPersonalBestV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.filter.validate()?;
        if self.filter.player_public_key.is_none() {
            return Err(ValidationError::ClaimMismatch {
                field: "player_personal_best.filter.player_public_key",
            });
        }
        if self.metric_value.metric() != self.filter.metric {
            return Err(ValidationError::ClaimMismatch {
                field: "player_personal_best.metric_value",
            });
        }
        self.metric_value.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerRunHistoryPageV2 {
    pub schema_version: u32,
    pub player: PlayerProfileV1,
    pub accepted_sequence_watermark: u64,
    pub runs: Vec<PlayerRunHistoryEntryV2>,
    pub personal_bests: Vec<PlayerPersonalBestV2>,
    pub next_cursor: Option<String>,
}

impl Validate for PlayerRunHistoryPageV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "PlayerRunHistoryPageV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        self.player.validate()?;
        if self.runs.len() > 100 || self.personal_bests.len() > 512 {
            return Err(ValidationError::CountOutOfRange {
                field: "player_run_history_page.entries",
            });
        }
        let mut run_ids = BTreeSet::new();
        for run in &self.runs {
            run.validate()?;
            if run.player_public_key != self.player.public_key {
                return Err(ValidationError::ClaimMismatch {
                    field: "player_run_history_page.runs.player_public_key",
                });
            }
            if !run_ids.insert(run.run.run_id.clone()) {
                return Err(ValidationError::Duplicate {
                    field: "player_run_history_page.runs.run_id",
                    value: run.run.run_id.as_str().to_owned(),
                });
            }
        }
        let mut best_filters = BTreeSet::new();
        for best in &self.personal_bests {
            best.validate()?;
            if best.filter.player_public_key != Some(self.player.public_key) {
                return Err(ValidationError::ClaimMismatch {
                    field: "player_run_history_page.personal_bests.player_public_key",
                });
            }
            let digest =
                best.filter
                    .canonical_digest()
                    .map_err(|_| ValidationError::ClaimMismatch {
                        field: "player_run_history_page.personal_bests.filter",
                    })?;
            if !best_filters.insert(digest) {
                return Err(ValidationError::Duplicate {
                    field: "player_run_history_page.personal_bests.filter",
                    value: digest.to_string(),
                });
            }
        }
        if let Some(cursor) = &self.next_cursor {
            crate::validation::text("player_run_history_page.next_cursor", cursor, 4_096)?;
        }
        Ok(())
    }
}

/// Public achievement state deliberately excludes free-form verifier evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicAchievementDecisionV1 {
    pub achievement_id: OpaqueId,
    pub evaluation: VerifiedAchievementEvaluationV1,
}

impl PublicAchievementDecisionV1 {
    pub fn from_private(value: &VerifiedAchievementV1) -> Self {
        Self {
            achievement_id: value.achievement_id.clone(),
            evaluation: value.evaluation,
        }
    }

    pub const fn is_awarded(&self) -> bool {
        matches!(self.evaluation, VerifiedAchievementEvaluationV1::Earned)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AchievementSummaryV1 {
    pub display_name: String,
    pub verified: PublicAchievementDecisionV1,
}

impl Validate for AchievementSummaryV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::text("run.achievement.display_name", &self.display_name, 100)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewerAvailabilityV2 {
    Available,
    Unavailable { safe_reason: String },
}

/// Facts a replay viewer needs to launch playback of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewerLaunchV2 {
    pub availability: ViewerAvailabilityV2,
    pub content_requirement: ViewerContentRequirementV2,
    /// Runtime build that recorded the replay; equals the replay's recorded
    /// engine version.
    pub runtime_build: String,
}

impl Validate for ViewerLaunchV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::text("run.viewer.runtime_build", &self.runtime_build, 64)?;
        if let ViewerAvailabilityV2::Unavailable { safe_reason } = &self.availability {
            crate::validation::text("run.viewer.safe_reason", safe_reason, 500)?;
        }
        Ok(())
    }
}

/// Complete public detail of one verified run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunDetailV2 {
    pub schema_version: u32,
    pub run_id: OpaqueId,
    pub board_id: OpaqueId,
    pub mission_id: String,
    pub edition: OfficialContentEditionV1,
    pub metrics: RunMetricsV1,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub uploader: Option<PublicParticipantV1>,
    pub verified_at_unix_ms: u64,
    pub replay: ReplayArtifactV1,
    pub recorded_engine_version: String,
    pub sim_config: CanonicalValue,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    pub achievements: Vec<AchievementSummaryV1>,
    pub viewer: ViewerLaunchV2,
}

impl Validate for RunDetailV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "RunDetailV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        crate::validation::text("run.mission_id", &self.mission_id, 256)?;
        validate_participants(
            self.max_concurrent_players,
            self.participant_instance_count,
            self.uploader.as_ref(),
        )?;
        crate::validation::nonzero("run.verified_at_unix_ms", &self.verified_at_unix_ms)?;
        self.replay.validate()?;
        crate::validation::text(
            "run.recorded_engine_version",
            &self.recorded_engine_version,
            64,
        )?;
        if !matches!(self.sim_config, CanonicalValue::Object(_)) {
            return Err(ValidationError::NotObject {
                field: "run.sim_config",
            });
        }
        self.sim_config.validate_depth(64)?;
        if self.achievements.len() > 256 {
            return Err(ValidationError::CountOutOfRange {
                field: "run.achievements",
            });
        }
        for achievement in &self.achievements {
            achievement.validate()?;
        }
        self.viewer.validate()
    }
}

mod submission_status;
pub use submission_status::{
    PublicSubmissionStateV1, PublicSubmissionStatusV1, SUBMISSION_OWNER_STATUS_SIGNATURE_DOMAIN_V2,
    SignedSubmissionOwnerStatusRequestV2, SubmissionAcceptedV1, SubmissionFailureCodeV1,
    SubmissionLifecycleV1, SubmissionOwnerStatusRequestV2, SubmissionOwnerStatusResponseV2,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(position: u64, rank: u64, points: i64, sequence: u64) -> LeaderboardEntryV2 {
        LeaderboardEntryV2 {
            position,
            rank,
            run_id: OpaqueId::new(format!("run-{position}")).unwrap(),
            metric_value: BoardMetricValueV2::OriginalScore { points },
            max_concurrent_players: 1,
            participant_instance_count: 1,
            uploader: None,
            replay_sha256: Digest32::from_bytes([position as u8; 32]),
            accepted_sequence: sequence,
            verified_at_unix_ms: 1_000 + sequence,
        }
    }

    fn filter() -> RunFilterV2 {
        RunFilterV2 {
            schema_version: crate::SCHEMA_VERSION_V2,
            board_id: OpaqueId::new("demo-standard-normal").unwrap(),
            mission_id: "Dem_Lei_MP".into(),
            metric: BoardMetricV1::OriginalScore,
            max_concurrent_players: None,
            player_public_key: None,
        }
    }

    #[test]
    fn leaderboard_page_requires_score_order_and_competition_ranks() {
        let page = |entries| LeaderboardPageV2 {
            schema_version: crate::SCHEMA_VERSION_V2,
            filter: filter(),
            accepted_sequence_watermark: 10,
            previous_cursor: None,
            entries,
            next_cursor: None,
        };
        assert!(
            page(vec![
                entry(1, 1, 50, 1),
                entry(2, 1, 50, 2),
                entry(3, 3, 40, 3)
            ])
            .validate()
            .is_ok()
        );
        assert!(
            page(vec![entry(1, 1, 40, 1), entry(2, 2, 50, 2)])
                .validate()
                .is_err()
        );
        assert!(
            page(vec![entry(1, 1, 50, 1), entry(2, 2, 50, 2)])
                .validate()
                .is_err()
        );
        let mut wrong_metric = page(vec![entry(1, 1, 50, 1)]);
        wrong_metric.entries[0].metric_value = BoardMetricValueV2::FastestSuccess {
            active_simulation_ticks: 5,
        };
        assert!(wrong_metric.validate().is_err());
    }

    #[test]
    fn named_uploader_must_occupy_the_host_seat() {
        let key = PublicKey32::from_bytes([7; 32]);
        let mut named = entry(1, 1, 50, 1);
        named.uploader = Some(PublicParticipantV1 {
            seat: 0,
            username: "Robin".into(),
            public_key: key,
            public_key_fingerprint: key.short_fingerprint(),
        });
        assert!(named.validate().is_ok());
        named.uploader.as_mut().unwrap().seat = 1;
        assert!(named.validate().is_err());
    }
}
