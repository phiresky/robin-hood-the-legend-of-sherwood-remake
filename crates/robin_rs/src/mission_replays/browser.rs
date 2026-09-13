//! Browser builds keep no local attempt links and cannot open a replay viewer.

// TODO: Persist browser recordings and open playback in an isolated browser session.

use super::RecordingIndex;
use robin_engine::campaign_history::MissionAttemptKey;
use std::path::PathBuf;

/// Browser builds own no background scan or active recording path.
#[derive(Debug, Default)]
pub(super) struct Runtime;

impl RecordingIndex {
    /// There is no local recording directory to scan.
    pub(crate) fn refresh_index(&self) -> Result<(), String> {
        Ok(())
    }

    /// No scan is ever started, so none completes.
    pub(crate) fn take_completion(&self) -> Option<Result<(), String>> {
        None
    }

    /// Nothing to cancel or join.
    pub(crate) fn shutdown(&self) -> Result<(), String> {
        Ok(())
    }

    /// Browser recordings are not linked to attempts yet.
    pub(crate) fn recording_finished(&self, _key: MissionAttemptKey, _completed_at: Option<i64>) {}

    /// Browser recordings are not linked to attempts yet.
    pub(crate) fn find(
        &self,
        _key: MissionAttemptKey,
        _completed_at: Option<i64>,
    ) -> Option<PathBuf> {
        None
    }
}

pub(crate) fn watch(
    _path: &std::path::Path,
    _expected: (MissionAttemptKey, Option<i64>),
) -> Result<(), String> {
    Err("Local replay playback requires the desktop game.".into())
}
