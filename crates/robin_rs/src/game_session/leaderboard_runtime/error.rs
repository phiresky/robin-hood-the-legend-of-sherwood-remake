//! Typed errors of mission-end leaderboard preparation.
//!
//! Leaf errors (leaderboard service, HTTP transport, mission-end documents)
//! are carried as transparent sources; local rules are categorised text.
//! Text is produced where the result leaves this module: presentation
//! notices, logs, and the frame-polled preparation task.
//!
//! Not serde: variants carry source errors.

use std::borrow::Cow;

type Text = Cow<'static, str>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum RankedError {
    /// A leaderboard service request or response decode failed.
    #[error(transparent)]
    Service(#[from] crate::leaderboard_service::LeaderboardServiceError),
    /// The leaderboard HTTP transport failed.
    #[error(transparent)]
    Http(#[from] crate::leaderboard_http::HttpTransportError),
    /// A mission-end submission document failed validation.
    #[error(transparent)]
    MissionEnd(#[from] crate::leaderboard_mission_end::MissionEndLeaderboardError),
    /// No published board, endpoint or identity is available for this run.
    #[error("{0}")]
    Unavailable(Text),
    /// The mission-end leaderboard lifecycle was used out of order.
    #[error("{0}")]
    Lifecycle(Text),
    /// Local replay evidence is unavailable or malformed.
    #[error("{0}")]
    Evidence(Text),
}

impl RankedError {
    pub(crate) fn unavailable(message: impl Into<Text>) -> Self {
        Self::Unavailable(message.into())
    }

    pub(crate) fn lifecycle(message: impl Into<Text>) -> Self {
        Self::Lifecycle(message.into())
    }

    pub(crate) fn evidence(message: impl Into<Text>) -> Self {
        Self::Evidence(message.into())
    }
}
