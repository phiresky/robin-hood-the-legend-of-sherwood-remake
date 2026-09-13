//! Typed errors of ranked pre-frame admission and mission-end authorization.
//!
//! Leaf errors (leaderboard service, HTTP transport, durable signing, ranked
//! session documents, protocol validation, campaign-chain store, preferences,
//! trusted clock, multiplayer ranked port) are carried as transparent sources,
//! so every `Display` is exactly the text these paths reported as `String`.
//! Local trust and lifecycle rules are categorised text.
//!
//! Text is produced where the result leaves this module: browse-only admission
//! reasons, the multiplayer downgrade notice, logs, and the
//! `leaderboard::mission_end` task/authorizer/co-signer traits, whose contracts
//! are `String`.
//!
//! Not serde: variants carry source errors.

use std::borrow::Cow;

type Text = Cow<'static, str>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum RankedError {
    /// The authenticated multiplayer ranked port refused or failed.
    #[error(transparent)]
    Transport(#[from] crate::multiplayer::MultiplayerError),
    /// A leaderboard service request or response decode failed.
    #[error(transparent)]
    Service(#[from] crate::leaderboard_service::LeaderboardServiceError),
    /// The leaderboard HTTP transport failed.
    #[error(transparent)]
    Http(#[from] crate::leaderboard_http::HttpTransportError),
    /// The durable leaderboard identity could not sign.
    #[error(transparent)]
    Signing(#[from] crate::leaderboard_signing::LeaderboardSigningError),
    /// A ranked session document, evidence or lifecycle operation failed.
    #[error(transparent)]
    Session(#[from] crate::leaderboard_ranked_session::RankedSessionError),
    /// A mission-end submission document failed validation.
    #[error(transparent)]
    MissionEnd(#[from] crate::leaderboard_mission_end::MissionEndLeaderboardError),
    /// A run-protocol document failed validation.
    #[error(transparent)]
    Validation(#[from] robin_run_protocol::ValidationError),
    /// A run-protocol document could not be canonicalised.
    #[error(transparent)]
    Canonical(#[from] robin_run_protocol::CanonicalDocumentError),
    /// Ranked simulation-input projection rejected the rules configuration.
    #[error(transparent)]
    Projection(#[from] robin_engine::simulation_inputs::ProjectionError),
    /// The verified campaign-chain receipt store failed.
    #[error(transparent)]
    ChainStore(#[from] crate::leaderboard_chains::CampaignChainStoreError),
    /// Leaderboard preferences could not be loaded.
    #[error(transparent)]
    Preferences(#[from] crate::leaderboard_preferences::LeaderboardPreferencesError),
    /// The trusted wall clock is unavailable.
    #[error(transparent)]
    Clock(#[from] crate::leaderboard_receipt_watcher::ReceiptWatcherError),
    /// A host, peer, server or local document violates a ranked trust rule.
    #[error("{0}")]
    Rejected(Text),
    /// The requested ranked authority, facet or identity is not available.
    #[error("{0}")]
    Unavailable(Text),
    /// The ranked session lifecycle is poisoned, downgraded or unresolved.
    #[error("{0}")]
    Lifecycle(Text),
    /// Ranked multiplayer setup ran past its bounded window.
    #[error("{0}")]
    Timeout(Text),
    /// Local replay evidence (replay exports report text) is unavailable.
    #[error("{0}")]
    Evidence(Text),
    /// A mission-end signer or exporter task failed; its trait reports text.
    #[error("{0}")]
    Task(Text),
    /// `source` prefixed with the operation that failed, as
    /// `"{context}: {source}"`.
    #[error("{context}: {source}")]
    Context {
        context: Text,
        #[source]
        source: Box<RankedError>,
    },
}

impl RankedError {
    pub(crate) fn rejected(message: impl Into<Text>) -> Self {
        Self::Rejected(message.into())
    }

    pub(crate) fn unavailable(message: impl Into<Text>) -> Self {
        Self::Unavailable(message.into())
    }

    pub(crate) fn lifecycle(message: impl Into<Text>) -> Self {
        Self::Lifecycle(message.into())
    }

    pub(crate) fn timeout(message: impl Into<Text>) -> Self {
        Self::Timeout(message.into())
    }

    pub(crate) fn evidence(message: impl Into<Text>) -> Self {
        Self::Evidence(message.into())
    }

    pub(crate) fn task(message: impl Into<Text>) -> Self {
        Self::Task(message.into())
    }

    /// Prefix `self` with the operation that failed.
    pub(crate) fn context(self, context: impl Into<Text>) -> Self {
        Self::Context {
            context: context.into(),
            source: Box::new(self),
        }
    }
}
