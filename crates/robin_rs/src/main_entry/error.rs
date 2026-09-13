//! Typed errors of the process entry points (`run_rust_game*`, browser replay
//! preparation, launch-route resolution).
//!
//! Every variant's `Display` reproduces the message these paths reported when
//! they returned `String`, so the process-exit log line is unchanged. Text is
//! produced once, where the binary, Android activity or browser shell reports
//! the failure. The browser join-ticket export and clap value parsers keep
//! their own `String` contracts.

use crate::game_session::MissionError;
use std::borrow::Cow;

type Text = Cow<'static, str>;

/// Why a graphical, headless or browser game run failed.
///
/// Opaque outside the crate: callers render it with `Display`.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct LaunchError(pub(crate) LaunchErrorKind);

#[derive(Debug, thiserror::Error)]
pub(crate) enum LaunchErrorKind {
    /// Mission construction or execution failed.
    #[error(transparent)]
    Mission(MissionError),
    /// A menu-launched session failed; `SessionOutcome::result` is already
    /// the menu-facing text.
    #[error("{0}")]
    Session(String),
    /// An `ApplicationContext` service (RPC transport, shipping catalog,
    /// asset cache, preparation authority) is unavailable.
    #[error("{0}")]
    Application(Text),
    /// The launch arguments select an impossible or unsupported route.
    #[error("{0}")]
    Arguments(Text),
    /// A requested or commanded replay could not be loaded or prepared.
    #[error("{0}")]
    Replay(Text),
    /// The save store could not be opened for a headless run.
    #[error(transparent)]
    SaveStore(crate::save_recovery::SaveStoreOpenError),
    /// A save slot selected by the menu is unavailable.
    #[error("{0}")]
    Save(Text),
    /// The requested mission could not be selected in the campaign.
    #[error("{0}")]
    Campaign(Text),
    /// Custom or distributed mission content could not be admitted.
    #[cfg_attr(
        all(target_arch = "wasm32", not(feature = "multiplayer")),
        allow(
            dead_code,
            reason = "browser builds admit content only through multiplayer"
        )
    )]
    #[error("{0}")]
    Content(Text),
    /// The main menu failed.
    #[error("{0}")]
    Menu(String),
    /// A multiplayer join invitation or transport operation failed.
    #[error(transparent)]
    Multiplayer(crate::multiplayer::MultiplayerError),
    /// The system clock cannot validate a join invitation.
    #[cfg_attr(
        not(feature = "multiplayer"),
        allow(dead_code, reason = "join invitations exist only with multiplayer")
    )]
    #[error("{0}")]
    Clock(Text),
    /// The browser environment (window, URL query) is unavailable.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(dead_code, reason = "only the browser shell reads its window query")
    )]
    #[error("{0}")]
    Browser(Text),
    /// The run succeeded but draining application services failed.
    #[error("application shutdown: {0}")]
    Shutdown(String),
    /// The run failed and draining application services failed too.
    #[error("{run}; application shutdown: {shutdown}")]
    RunAndShutdown {
        run: Box<LaunchError>,
        shutdown: String,
    },
}

impl LaunchError {
    pub(crate) fn application(message: impl Into<Text>) -> Self {
        Self(LaunchErrorKind::Application(message.into()))
    }

    pub(crate) fn arguments(message: impl Into<Text>) -> Self {
        Self(LaunchErrorKind::Arguments(message.into()))
    }

    pub(crate) fn replay(message: impl Into<Text>) -> Self {
        Self(LaunchErrorKind::Replay(message.into()))
    }

    pub(crate) fn save(message: impl Into<Text>) -> Self {
        Self(LaunchErrorKind::Save(message.into()))
    }

    pub(crate) fn campaign(message: impl Into<Text>) -> Self {
        Self(LaunchErrorKind::Campaign(message.into()))
    }

    #[cfg_attr(
        all(target_arch = "wasm32", not(feature = "multiplayer")),
        allow(
            dead_code,
            reason = "browser builds admit content only through multiplayer"
        )
    )]
    pub(crate) fn content(message: impl Into<Text>) -> Self {
        Self(LaunchErrorKind::Content(message.into()))
    }

    pub(crate) fn session(message: String) -> Self {
        Self(LaunchErrorKind::Session(message))
    }

    pub(crate) fn menu(message: String) -> Self {
        Self(LaunchErrorKind::Menu(message))
    }

    #[cfg_attr(
        not(feature = "multiplayer"),
        allow(dead_code, reason = "join invitations exist only with multiplayer")
    )]
    pub(crate) fn clock(message: impl Into<Text>) -> Self {
        Self(LaunchErrorKind::Clock(message.into()))
    }

    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(dead_code, reason = "only the browser shell reads its window query")
    )]
    pub(crate) fn browser(message: impl Into<Text>) -> Self {
        Self(LaunchErrorKind::Browser(message.into()))
    }

    /// Combine a run result with the application-service shutdown result.
    pub(crate) fn with_shutdown(
        result: Result<i32, LaunchError>,
        shutdown: Result<(), String>,
    ) -> Result<i32, LaunchError> {
        match (result, shutdown) {
            (result, Ok(())) => result,
            (Ok(_), Err(error)) => Err(Self(LaunchErrorKind::Shutdown(error))),
            (Err(run), Err(shutdown)) => Err(Self(LaunchErrorKind::RunAndShutdown {
                run: Box::new(run),
                shutdown,
            })),
        }
    }
}

impl From<MissionError> for LaunchError {
    fn from(error: MissionError) -> Self {
        Self(LaunchErrorKind::Mission(error))
    }
}

impl From<crate::multiplayer::MultiplayerError> for LaunchError {
    fn from(error: crate::multiplayer::MultiplayerError) -> Self {
        Self(LaunchErrorKind::Multiplayer(error))
    }
}

impl From<crate::save_recovery::SaveStoreOpenError> for LaunchError {
    fn from(error: crate::save_recovery::SaveStoreOpenError) -> Self {
        Self(LaunchErrorKind::SaveStore(error))
    }
}

// Launch errors cross into `anyhow` in the browser and Android shells.
const _: () = {
    const fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_send_sync::<LaunchError>();
    assert_send_sync::<MissionError>();
};
