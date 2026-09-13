//! Host-only links between immutable attempts and local recordings.
//! These paths never enter deterministic campaign/save state.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
use native as platform;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::default_directory;
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
    pub(crate) fn disabled() -> Self {
        Self {
            directory: None,
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
