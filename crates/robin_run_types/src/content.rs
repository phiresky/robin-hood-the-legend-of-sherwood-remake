//! Official content editions.

use serde::{Deserialize, Serialize};

/// Operator-installed raw game edition a ranked board is verified against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialContentEditionV1 {
    Demo,
    Full,
}
