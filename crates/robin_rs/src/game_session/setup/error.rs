//! Classified preparation failures. Optional absence is represented by `Ok(None)`.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub(in crate::game_session) enum ResourcePreparationError {
    #[error("preparation authority unavailable: {0}")]
    MissingAuthority(String),
    #[error("cannot read {path}: {detail}")]
    Unavailable { path: String, detail: String },
    #[error("malformed {path}: {detail}")]
    Malformed { path: String, detail: String },
}

impl ResourcePreparationError {
    pub(super) fn malformed(path: impl ToString, detail: impl ToString) -> Self {
        Self::Malformed {
            path: path.to_string(),
            detail: detail.to_string(),
        }
    }

    pub(super) fn unavailable(path: impl ToString, detail: impl ToString) -> Self {
        Self::Unavailable {
            path: path.to_string(),
            detail: detail.to_string(),
        }
    }
}
