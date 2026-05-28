//! Entity identity. A tiny module on its own because virtually every
//! sim module references `EntityId` and pulling in all of `element` to
//! get one `u32` newtype is not justified.

use serde::{Deserialize, Serialize};

/// Unique identifier for an entity in the game world.
///
/// Stored as a 0-based index into the engine's entity table.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct EntityId(pub u32);

impl std::fmt::Debug for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EntityId({})", self.0)
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EntityId({})", self.0)
    }
}
