//! Typed multiplayer transport errors.
//!
//! Every variant's `Display` reproduces the message the transport reported
//! when these errors were plain strings, so menu text, logs and the
//! `NetEvent::Fatal` payload shown to players are unchanged. Callers that need
//! to decide something (reconnect vs fail, remote fault vs local invariant)
//! match on the variant instead of sniffing message prefixes.
//!
//! Not serde: several variants carry the underlying error as a source.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

/// An underlying error kept as a shared trait object so [`MultiplayerError`]
/// stays `Clone` (startup errors are published to several observers) and can
/// be downcast to the original type.
pub type SharedError = Arc<dyn std::error::Error + Send + Sync + 'static>;

type Text = Cow<'static, str>;

#[derive(Clone, Debug, thiserror::Error)]
pub enum MultiplayerError {
    /// An iroh endpoint, connection, stream or worker-thread operation failed.
    #[error("{context}: {source}")]
    Transport {
        context: Text,
        #[source]
        source: SharedError,
    },
    /// A frame violated the class/size/decoding rules of the wire framing.
    #[cfg(feature = "multiplayer")]
    #[error(transparent)]
    Framing(#[from] super::framing::FramingError),
    /// A local I/O operation (thread spawn, campaign lease) failed.
    #[error(transparent)]
    Io(Arc<std::io::Error>),
    /// An authenticated peer session lost dispatch authority.
    #[error("{0}")]
    PeerSession(#[source] SharedError),
    /// The Unix system clock cannot schedule multiplayer.
    #[error(transparent)]
    Clock(#[from] super::clock::ClockError),
    #[error("{phase} timed out after {after:?}")]
    Timeout { phase: Text, after: Duration },
    /// A whole multi-step phase ran past its overall budget.
    #[error("{phase} exceeded {limit:?}")]
    DeadlineExceeded { phase: Text, limit: Duration },
    /// Shutdown began before the operation finished.
    #[error("{0}")]
    Cancelled(Text),
    /// A local channel whose owner must still exist was dropped.
    #[error("{0}")]
    ChannelClosed(Text),
    /// The Hello/Welcome handshake could not be completed.
    #[error("{0}")]
    Handshake(Text),
    /// Distributed-mod content offer, transfer or reconnect content differs
    /// from what was admitted.
    #[error("{0}")]
    ContentMismatch(Text),
    /// The local player or local validation declined the host content.
    #[error("{0}")]
    ContentDeclined(Text),
    /// The host explicitly rejected the connection or session.
    #[error("host rejected {stage}: {reason}")]
    HostRejected { stage: &'static str, reason: String },
    /// The host requires a full-snapshot reconnect; the session is retried.
    #[error("host requires a full-snapshot reconnect: {reason}")]
    ReconnectRequired { reason: String },
    /// The remote side sent a message its role or session phase forbids.
    #[error("{0}")]
    RemoteProtocol(Text),
    /// A local or remote identity/key is unavailable or does not bind.
    #[error("{0}")]
    Identity(Text),
    /// A connect string, endpoint id or relay URL does not parse.
    #[error("{context}: {source}")]
    InvalidAddress {
        context: Text,
        #[source]
        source: SharedError,
    },
    /// A browser join invitation is malformed, expired or mismatched.
    #[error("{0}")]
    Invitation(Text),
    /// A browser join invitation's encoding (base64, JSON) is invalid.
    #[error("{context}: {source}")]
    InvitationDecode {
        context: Text,
        #[source]
        source: SharedError,
    },
    /// A local transport invariant failed (poisoned lock, one-shot reused,
    /// runtime missing, limit exceeded).
    #[error("{0}")]
    LocalState(Text),
    /// The build or platform does not provide this multiplayer capability.
    #[error("{0}")]
    Unavailable(Text),
    /// The stable browser shell (JavaScript) failed; its values are not Rust
    /// errors, so the shell's own message is carried.
    #[error("{0}")]
    Browser(String),
    /// Local content identity files are invalid.
    #[error("{0}")]
    ContentIdentity(Text),
    /// Local content identity files could not be read or parsed.
    #[error("{context}: {source}")]
    ContentIo {
        context: Text,
        #[source]
        source: SharedError,
    },
    /// Rendezvous/matchmaking discovery failed.
    #[error("{0}")]
    Matchmaking(Text),
    /// A local client publication was refused before reaching the wire.
    #[cfg(feature = "multiplayer")]
    #[error(transparent)]
    Protocol(#[from] super::client_outgoing::ClientProtocolError),
    /// `source` with the operation that failed, as `"{context}: {source}"`.
    #[error("{context}: {source}")]
    Context {
        context: Text,
        #[source]
        source: Arc<MultiplayerError>,
    },
}

impl From<std::io::Error> for MultiplayerError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(Arc::new(error))
    }
}

impl MultiplayerError {
    pub fn transport(
        context: impl Into<Text>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Transport {
            context: context.into(),
            source: Arc::new(source),
        }
    }

    pub fn invalid_address(
        context: impl Into<Text>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::InvalidAddress {
            context: context.into(),
            source: Arc::new(source),
        }
    }

    pub fn invitation_decode(
        context: impl Into<Text>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::InvitationDecode {
            context: context.into(),
            source: Arc::new(source),
        }
    }

    pub fn timeout(phase: impl Into<Text>, after: Duration) -> Self {
        Self::Timeout {
            phase: phase.into(),
            after,
        }
    }

    /// Prefix `self` with the operation that failed.
    pub fn context(self, context: impl Into<Text>) -> Self {
        Self::Context {
            context: context.into(),
            source: Arc::new(self),
        }
    }
}

/// An error that is only a message from a non-Rust or string-reporting
/// boundary (engine validation helpers, `serde` messages already formatted),
/// usable as the `source` of [`MultiplayerError::Transport`] and friends.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct MessageError(pub String);
