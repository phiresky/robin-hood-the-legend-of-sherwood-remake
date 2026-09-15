//! Host-only links between immutable attempts and local recordings.
//! These paths never enter deterministic campaign/save state.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
use native as platform;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::{default_directory, replay_attempt_identity};
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
use browser as platform;
pub(crate) use platform::watch;

/// Application-owned recording links and background scan. Runtime ownership cannot
/// be restored from a diagnostic serialization.
#[derive(Debug, Serialize)]
pub(crate) struct RecordingIndex {
    directory: Option<PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(skip)]
    pub(crate) submissions: std::sync::Mutex<crate::leaderboard::history::ReplaySubmissions>,
    #[serde(skip)]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "browser builds own no scan runtime")
    )]
    runtime: platform::Runtime,
}

impl<'de> Deserialize<'de> for RecordingIndex {
    fn deserialize<D: serde::Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "recording index runtime ownership cannot be deserialized",
        ))
    }
}

impl RecordingIndex {
    pub(crate) fn submission_info(
        &self,
        path: &std::path::Path,
    ) -> crate::leaderboard::history::ReplaySubmissionInfo {
        #[cfg(not(target_arch = "wasm32"))]
        {
            robin_util::sync::lock(&self.submissions).info(path)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            crate::leaderboard::history::ReplaySubmissionInfo {
                message: "Local replay submission is unavailable in this browser.".into(),
                can_submit: false,
                url: None,
            }
        }
    }
    pub(crate) fn submit_recording(
        &self,
        path: &std::path::Path,
        expected: (
            robin_engine::campaign_history::MissionAttemptKey,
            Option<i64>,
        ),
        edition: robin_run_protocol::OfficialContentEditionV1,
    ) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            robin_util::sync::lock(&self.submissions).submit(path, expected, edition)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (path, expected, edition);
            Err("Local replay submission is unavailable in this browser".into())
        }
    }
    pub(crate) fn disabled() -> Self {
        Self {
            directory: None,
            #[cfg(not(target_arch = "wasm32"))]
            submissions: Default::default(),
            runtime: Default::default(),
        }
    }
}

impl Drop for RecordingIndex {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            tracing::warn!("Cannot shut down recording index: {error}");
        }
    }
}
