//! Entity identity. A tiny module on its own because virtually every
//! sim module references `EntityId` and pulling in all of `element` to
//! get one identifier type is not justified.

use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// The concrete entity table class an [`EntityId`] points at.
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
    robin_state_hash_derive::StateHash,
)]
pub enum EntityIdKind {
    Pc,
    Soldier,
    Civilian,
    Fx,
    Target,
    Bonus,
    Scroll,
    Projectile,
    Net,
}

/// Unique identifier for an entity in the game world.
///
/// Each variant stores a 0-based index into the engine's entity table.  The
/// variant mirrors the concrete [`crate::element::Entity`] variant at that
/// slot, making type mismatches visible in debug output and serialized state
/// while preserving the raw index needed by legacy script handles.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub enum EntityId {
    Pc(u32),
    Soldier(u32),
    Civilian(u32),
    Fx(u32),
    Target(u32),
    Bonus(u32),
    Scroll(u32),
    Projectile(u32),
    Net(u32),
}

impl PartialEq for EntityId {
    fn eq(&self, other: &Self) -> bool {
        self.index() == other.index()
    }
}

impl Eq for EntityId {}

impl PartialOrd for EntityId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EntityId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.index().cmp(&other.index())
    }
}

impl Hash for EntityId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index().hash(state);
    }
}

impl EntityId {
    /// Build a typed ID for an entity table index and kind.
    pub const fn new(index: u32, kind: EntityIdKind) -> Self {
        match kind {
            EntityIdKind::Pc => Self::Pc(index),
            EntityIdKind::Soldier => Self::Soldier(index),
            EntityIdKind::Civilian => Self::Civilian(index),
            EntityIdKind::Fx => Self::Fx(index),
            EntityIdKind::Target => Self::Target(index),
            EntityIdKind::Bonus => Self::Bonus(index),
            EntityIdKind::Scroll => Self::Scroll(index),
            EntityIdKind::Projectile => Self::Projectile(index),
            EntityIdKind::Net => Self::Net(index),
        }
    }

    /// Return the raw 0-based entity table index.
    pub const fn index(self) -> u32 {
        match self {
            Self::Pc(index)
            | Self::Soldier(index)
            | Self::Civilian(index)
            | Self::Fx(index)
            | Self::Target(index)
            | Self::Bonus(index)
            | Self::Scroll(index)
            | Self::Projectile(index)
            | Self::Net(index) => index,
        }
    }

    /// Return the entity table class carried by this ID.
    pub const fn kind(self) -> EntityIdKind {
        match self {
            Self::Pc(_) => EntityIdKind::Pc,
            Self::Soldier(_) => EntityIdKind::Soldier,
            Self::Civilian(_) => EntityIdKind::Civilian,
            Self::Fx(_) => EntityIdKind::Fx,
            Self::Target(_) => EntityIdKind::Target,
            Self::Bonus(_) => EntityIdKind::Bonus,
            Self::Scroll(_) => EntityIdKind::Scroll,
            Self::Projectile(_) => EntityIdKind::Projectile,
            Self::Net(_) => EntityIdKind::Net,
        }
    }

    /// Return the entity table class carried by this ID.
    pub const fn expect_kind(self) -> EntityIdKind {
        self.kind()
    }

    /// Return true when the ID variant matches `kind`.
    pub const fn is_kind(self, kind: EntityIdKind) -> bool {
        self.kind() as u8 == kind as u8
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}({})", self.kind(), self.index())
    }
}
