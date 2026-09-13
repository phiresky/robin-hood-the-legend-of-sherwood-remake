//! Typed mission setup and run errors.
//!
//! Every variant's `Display` reproduces the message the mission chain reported
//! when these errors were plain strings, so menu text, logs and tests asserting
//! on the text are unchanged. The variant records *which* part of mission
//! construction or execution failed; the text is carried only where the
//! failing leaf (application services, asset loaders, save and replay stores
//! outside `game_session`) still reports a `String`.
//!
//! Text is produced exactly once, at the [`super::SessionOutcome::result`]
//! boundary the menu displays and at the process-exit reporting in
//! `main_entry`.
//!
//! Not serde: several variants carry the underlying error as a source.

use std::borrow::Cow;

type Text = Cow<'static, str>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum MissionError {
    /// Launch arguments, restart checkpoints or replay/save launch admission
    /// rejected the requested mission before construction.
    #[error("{0}")]
    Launch(Text),
    /// An [`crate::host::ApplicationContext`] or `Host` service (active
    /// profile, preparation authority, shipping catalog, asset cache,
    /// browser audio) is unavailable.
    #[error("{0}")]
    Application(Text),
    /// Mission assets (shipping payload, exact archive, level, terrain,
    /// sprites, descriptors, localized names) could not be resolved or loaded.
    #[error("{0}")]
    Asset(Text),
    /// The exact mission assets recorded by a save or replay could not be
    /// restored.
    #[error(transparent)]
    AssetRestore(#[from] crate::mission_asset_restore::MissionAssetRestoreError),
    /// A mission script or Spellforge package could not be prepared.
    #[error("{0}")]
    Script(Text),
    /// Spellforge/Lua session startup failed.
    #[error(transparent)]
    Spellforge(#[from] crate::lua_session::SpellforgeSessionError),
    /// Replay recording, playback admission or a replay timeline boundary
    /// failed.
    #[error("{0}")]
    Replay(Text),
    /// A save payload, save store or save retirement failed.
    #[error("{0}")]
    Save(Text),
    /// Mission audio (sound banks, duration metadata) could not be prepared.
    #[error("{0}")]
    Audio(Text),
    /// Screenshot rendering, GPU capture or PNG output failed.
    #[error("{0}")]
    Render(Text),
    /// GPU readback of a rendered frame failed.
    #[error(transparent)]
    Capture(#[from] crate::renderer::CaptureError),
    /// A per-frame simulation/timeline invariant failed.
    #[error("{0}")]
    Frame(Text),
    /// A multiplayer transport operation failed.
    #[error(transparent)]
    Multiplayer(#[from] crate::multiplayer::MultiplayerError),
    /// The multiplayer session could not be established or broke a protocol
    /// rule while running.
    #[error(transparent)]
    Session(#[from] super::multiplayer::MultiplayerSessionError),
    /// Classified resource preparation failed.
    #[error(transparent)]
    Preparation(#[from] super::setup::error::ResourcePreparationError),
    /// A save-operation retirement failed after the mission or session body
    /// finished; `result` is the `Debug` rendering of the superseded result.
    #[error("{scope} save retirement failed: {error}; {scope} result: {result}")]
    SaveRetirement {
        scope: &'static str,
        #[source]
        error: Box<MissionError>,
        result: String,
    },
    /// `source` prefixed with the operation that failed, as
    /// `"{context}: {source}"`.
    #[error("{context}: {source}")]
    Context {
        context: Text,
        #[source]
        source: Box<MissionError>,
    },
}

impl MissionError {
    pub(crate) fn launch(message: impl Into<Text>) -> Self {
        Self::Launch(message.into())
    }

    pub(crate) fn application(message: impl Into<Text>) -> Self {
        Self::Application(message.into())
    }

    pub(crate) fn asset(message: impl Into<Text>) -> Self {
        Self::Asset(message.into())
    }

    pub(crate) fn script(message: impl Into<Text>) -> Self {
        Self::Script(message.into())
    }

    pub(crate) fn replay(message: impl Into<Text>) -> Self {
        Self::Replay(message.into())
    }

    pub(crate) fn save(message: impl Into<Text>) -> Self {
        Self::Save(message.into())
    }

    pub(crate) fn audio(message: impl Into<Text>) -> Self {
        Self::Audio(message.into())
    }

    pub(crate) fn render(message: impl Into<Text>) -> Self {
        Self::Render(message.into())
    }

    pub(crate) fn frame(message: impl Into<Text>) -> Self {
        Self::Frame(message.into())
    }

    /// Prefix `self` with the operation that failed.
    pub(crate) fn context(self, context: impl Into<Text>) -> Self {
        Self::Context {
            context: context.into(),
            source: Box::new(self),
        }
    }
}

/// `Debug` text of a result whose error used to be a `String`: `Ok(value)` or
/// `Err("message")`, so reports embedding a superseded result keep their text.
pub(crate) fn legacy_result_debug<T: std::fmt::Debug, E: std::fmt::Display>(
    result: &Result<T, E>,
) -> String {
    match result {
        Ok(value) => format!("{:?}", Ok::<&T, ()>(value)),
        Err(error) => format!("{:?}", Err::<(), String>(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_result_debug_matches_string_results() {
        let ok: Result<u32, String> = Ok(7);
        assert_eq!(
            legacy_result_debug(&ok.clone().map_err(MissionError::launch)),
            format!("{ok:?}")
        );
        let err: Result<u32, String> = Err("launch \"failed\"".to_owned());
        assert_eq!(
            legacy_result_debug(&err.clone().map_err(MissionError::launch)),
            format!("{err:?}")
        );
    }

    #[test]
    fn context_prefixes_the_source_text() {
        let error = MissionError::asset("missing map").context("load level");
        assert_eq!(error.to_string(), "load level: missing map");
    }
}
