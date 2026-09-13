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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leaderboard_http::{HttpResponse, HttpTransportError};
    use crate::leaderboard_preferences::LeaderboardApiBaseUrl;
    use robin_run_protocol::SCHEMA_VERSION_V1;

    fn browser() -> LeaderboardBrowser {
        // A closed loopback port: the spawned request fails on its own, and
        // nothing in these tests waits for it.
        let base_url = LeaderboardApiBaseUrl::parse_development("http://127.0.0.1:9/api/v1")
            .expect("loopback development endpoint");
        LeaderboardBrowser::new(LeaderboardApi::new(base_url))
    }

    fn json(status: u16, body: Vec<u8>) -> HttpResponse {
        HttpResponse {
            status,
            content_type: Some("application/json".to_owned()),
            body,
        }
    }

    #[test]
    fn begin_metadata_holds_one_request_until_it_is_polled_off() {
        let mut browser = browser();
        assert!(browser.task.is_none());
        assert!(browser.poll().is_none(), "no request in flight yet");

        browser.begin_metadata().unwrap();
        assert!(browser.task.is_some());
        assert!(matches!(
            browser.begin_metadata(),
            Err(LeaderboardServiceError::InvalidProtocol(reason))
                if reason.contains("already in progress")
        ));

        // Completing the request clears the slot and re-arms the browser.
        browser.task = Some(HttpTask::ready(Err(HttpTransportError::Timeout)));
        assert!(matches!(
            browser.poll(),
            Some(Err(LeaderboardServiceError::Transport(
                HttpTransportError::Timeout
            )))
        ));
        assert!(browser.task.is_none());
        browser.begin_metadata().unwrap();
        assert!(browser.task.is_some());
    }

    #[test]
    fn poll_decodes_metadata_exactly_once_and_reports_bad_responses() {
        let metadata = LeaderboardMetadataV1 {
            schema_version: SCHEMA_VERSION_V1,
            missions: Vec::new(),
            rulesets: Vec::new(),
            competitions: Vec::new(),
            full_campaign: None,
        };
        let mut browser = browser();
        browser.task = Some(HttpTask::ready(Ok(json(
            200,
            serde_json::to_vec(&metadata).unwrap(),
        ))));
        match browser.poll() {
            Some(Ok(LeaderboardBrowseEvent::Metadata(loaded))) => assert_eq!(loaded, metadata),
            other => panic!("expected decoded metadata, got {other:?}"),
        }
        assert!(browser.task.is_none());
        assert!(browser.poll().is_none(), "a completed task is taken once");

        browser.task = Some(HttpTask::ready(Ok(json(503, b"down".to_vec()))));
        assert!(matches!(
            browser.poll(),
            Some(Err(LeaderboardServiceError::HttpStatus { status: 503 }))
        ));

        browser.task = Some(HttpTask::ready(Ok(json(
            200,
            b"{\"schema_version\":1".to_vec(),
        ))));
        assert!(matches!(
            browser.poll(),
            Some(Err(LeaderboardServiceError::InvalidJson(_)))
        ));
        assert!(browser.task.is_none());
    }
}
