//! Frame-polled metadata used by the live leaderboard admission UI.
//!
//! Run browsing, account management and replay downloads have no UI consumer.
//! Do not retain dormant task states or signing flows as an implicit feature.

use crate::leaderboard_http::HttpTask;
use crate::leaderboard_service::{LeaderboardApi, LeaderboardServiceError, decode_metadata};
use robin_run_protocol::LeaderboardMetadataV1;

#[derive(Debug)]
pub enum LeaderboardBrowseEvent {
    Metadata(LeaderboardMetadataV1),
}

pub struct LeaderboardBrowser {
    api: LeaderboardApi,
    task: Option<HttpTask>,
}

impl LeaderboardBrowser {
    pub fn new(api: LeaderboardApi) -> Self {
        Self { api, task: None }
    }

    pub fn begin_metadata(&mut self) -> Result<(), LeaderboardServiceError> {
        if self.task.is_some() {
            return Err(LeaderboardServiceError::InvalidProtocol(
                "another leaderboard metadata request is already in progress".to_owned(),
            ));
        }
        self.task = Some(self.api.metadata()?);
        Ok(())
    }

    pub fn poll(&mut self) -> Option<Result<LeaderboardBrowseEvent, LeaderboardServiceError>> {
        let result = self.task.as_ref()?.try_take()?;
        self.task = None;
        Some(decode_metadata(result).map(LeaderboardBrowseEvent::Metadata))
    }
}
