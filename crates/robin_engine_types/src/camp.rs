//! Mission allegiance values.

use serde::{Deserialize, Serialize};

/// Faction / camp allegiance.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Camp {
    Royalists,
    Lacklandists,
    #[default]
    Error,
    /// A custom mission allegiance. IDs 0 and 1 are reserved for the
    /// Royalist and Lacklandist legacy camps respectively.
    Custom(u16),
}

impl Camp {
    pub const ROYALIST_ID: u16 = 0;
    pub const LACKLANDIST_ID: u16 = 1;
    pub const FIRST_CUSTOM_ID: u16 = 2;

    /// Resolve a mission-authored allegiance ID.
    pub fn from_allegiance_id(id: u16) -> Self {
        match id {
            Self::ROYALIST_ID => Self::Royalists,
            Self::LACKLANDIST_ID => Self::Lacklandists,
            id => Self::Custom(id),
        }
    }

    pub fn allegiance_id(self) -> Option<u16> {
        match self {
            Self::Royalists => Some(Self::ROYALIST_ID),
            Self::Lacklandists => Some(Self::LACKLANDIST_ID),
            Self::Custom(id) => Some(id),
            Self::Error => None,
        }
    }

    /// Legacy fallback for tests and data migration that have no mission
    /// state. Runtime systems must query `DiplomacyState`/`EngineInner` so
    /// authored and changed relationships are observed.
    pub fn is_hostile_to(self, other: Self) -> bool {
        match (self.allegiance_id(), other.allegiance_id()) {
            (Some(left), Some(right)) => left != right,
            _ => {
                tracing::warn!(
                    left = ?self,
                    right = ?other,
                    "cannot compare hostility for invalid camp; treating pair as non-hostile"
                );
                false
            }
        }
    }

    /// Legacy two-camp fallback. Runtime systems must use the mission's
    /// explicit player coalition through `DiplomacyState`/`EngineInner`.
    pub fn is_player_aligned(self) -> bool {
        self == Self::Royalists
    }

    pub fn index(self) -> Option<usize> {
        self.allegiance_id().map(usize::from)
    }
}
