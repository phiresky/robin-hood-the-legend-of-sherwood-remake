//! Typed failures that stop one parity run before it reaches a result.
//!
//! Classification rule: a failure caused by something the run consumes — the
//! native trace store and its format, recorded trace content (including
//! recorded identities that have no Rust correspondence), game data, save
//! bodies, the process environment, host I/O — or by the engine rejecting a
//! recorded boundary is a [`TraceRunError`] and propagates to the runner's
//! single print-and-exit point. A violated invariant of this crate's own
//! reconstruction or of engine state it has just enumerated (an identity the
//! replay inserted itself disappearing, a Rust arena number outside its own
//! domain) indicates a bug and remains a panic with context.

#[derive(Debug, thiserror::Error)]
pub(super) enum TraceRunError {
    /// Native trace storage, conversion and trace-format failures.
    #[error("{0}")]
    Storage(String),
    /// Recorded trace content this replay cannot admit or map onto the Rust
    /// engine.
    #[error("{0}")]
    TraceContent(String),
    /// Game data, save bodies, process environment and host I/O outside the
    /// trace store.
    #[error("{0}")]
    Input(String),
    /// The Rust engine rejected a recorded frame boundary.
    #[error("{0}")]
    Admission(String),
    /// Rust consumed a different number of draws than the Original's
    /// authoritative RNG stream for a frame.
    #[error("{0}")]
    RngDivergence(String),
}

pub(super) type TraceRunResult<T> = Result<T, TraceRunError>;

/// The storage layer reports `String` errors; `?` lifts them into the storage
/// class.
impl From<String> for TraceRunError {
    fn from(error: String) -> Self {
        Self::Storage(error)
    }
}
