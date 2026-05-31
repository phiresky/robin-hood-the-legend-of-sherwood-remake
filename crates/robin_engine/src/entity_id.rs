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

macro_rules! entity_leaf_id {
    ($name:ident) => {
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
        pub struct $name(pub u32);

        impl $name {
            pub const fn index(self) -> u32 {
                self.0
            }
        }
    };
}

entity_leaf_id!(PcId);
entity_leaf_id!(SoldierId);
entity_leaf_id!(CivilianId);
entity_leaf_id!(FxId);
entity_leaf_id!(TargetId);
entity_leaf_id!(BonusId);
entity_leaf_id!(ScrollId);
entity_leaf_id!(ProjectileId);
entity_leaf_id!(NetId);

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum ActorId {
    Pc(PcId),
    Soldier(SoldierId),
    Civilian(CivilianId),
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum HumanId {
    Pc(PcId),
    Soldier(SoldierId),
    Civilian(CivilianId),
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum NpcId {
    Soldier(SoldierId),
    Civilian(CivilianId),
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum ObjectId {
    Bonus(BonusId),
    Scroll(ScrollId),
    Projectile(ProjectileId),
    Net(NetId),
}

/// Unique identifier for an entity in the game world.
///
/// Each variant stores a 0-based index into the engine's entity table.  The
/// variant mirrors the concrete [`crate::element::Entity`] variant at that
/// slot, making type mismatches visible in debug output and serialized state
/// while still retaining the raw table index needed by script handles.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub enum EntityId {
    Pc(PcId),
    Soldier(SoldierId),
    Civilian(CivilianId),
    Fx(FxId),
    Target(TargetId),
    Bonus(BonusId),
    Scroll(ScrollId),
    Projectile(ProjectileId),
    Net(NetId),
}

impl PartialEq for EntityId {
    fn eq(&self, other: &Self) -> bool {
        self.kind() == other.kind() && self.index() == other.index()
    }
}

impl Eq for EntityId {}

macro_rules! entity_id_partial_eq {
    ($($id:ty),+ $(,)?) => {
        $(
            impl PartialEq<$id> for EntityId {
                fn eq(&self, other: &$id) -> bool {
                    *self == EntityId::from(*other)
                }
            }

            impl PartialEq<EntityId> for $id {
                fn eq(&self, other: &EntityId) -> bool {
                    EntityId::from(*self) == *other
                }
            }
        )+
    };
}

entity_id_partial_eq!(
    ActorId,
    HumanId,
    NpcId,
    ObjectId,
    PcId,
    SoldierId,
    CivilianId,
    FxId,
    TargetId,
    BonusId,
    ScrollId,
    ProjectileId,
    NetId,
);

impl PartialOrd for EntityId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EntityId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.index()
            .cmp(&other.index())
            .then_with(|| self.kind().cmp(&other.kind()))
    }
}

impl Hash for EntityId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind().hash(state);
        self.index().hash(state);
    }
}

impl EntityId {
    /// Build a typed ID for an entity table index and kind.
    pub const fn new(index: u32, kind: EntityIdKind) -> Self {
        match kind {
            EntityIdKind::Pc => Self::Pc(PcId(index)),
            EntityIdKind::Soldier => Self::Soldier(SoldierId(index)),
            EntityIdKind::Civilian => Self::Civilian(CivilianId(index)),
            EntityIdKind::Fx => Self::Fx(FxId(index)),
            EntityIdKind::Target => Self::Target(TargetId(index)),
            EntityIdKind::Bonus => Self::Bonus(BonusId(index)),
            EntityIdKind::Scroll => Self::Scroll(ScrollId(index)),
            EntityIdKind::Projectile => Self::Projectile(ProjectileId(index)),
            EntityIdKind::Net => Self::Net(NetId(index)),
        }
    }

    /// Return the raw 0-based entity table index.
    pub const fn index(self) -> u32 {
        match self {
            Self::Pc(id) => id.index(),
            Self::Soldier(id) => id.index(),
            Self::Civilian(id) => id.index(),
            Self::Fx(id) => id.index(),
            Self::Target(id) => id.index(),
            Self::Bonus(id) => id.index(),
            Self::Scroll(id) => id.index(),
            Self::Projectile(id) => id.index(),
            Self::Net(id) => id.index(),
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

impl ActorId {
    pub const fn index(self) -> u32 {
        match self {
            Self::Pc(id) => id.index(),
            Self::Soldier(id) => id.index(),
            Self::Civilian(id) => id.index(),
        }
    }
}

impl HumanId {
    pub const fn index(self) -> u32 {
        match self {
            Self::Pc(id) => id.index(),
            Self::Soldier(id) => id.index(),
            Self::Civilian(id) => id.index(),
        }
    }
}

impl NpcId {
    pub const fn index(self) -> u32 {
        match self {
            Self::Soldier(id) => id.index(),
            Self::Civilian(id) => id.index(),
        }
    }
}

impl ObjectId {
    pub const fn index(self) -> u32 {
        match self {
            Self::Bonus(id) => id.index(),
            Self::Scroll(id) => id.index(),
            Self::Projectile(id) => id.index(),
            Self::Net(id) => id.index(),
        }
    }
}

impl From<PcId> for ActorId {
    fn from(id: PcId) -> Self {
        Self::Pc(id)
    }
}

impl From<SoldierId> for ActorId {
    fn from(id: SoldierId) -> Self {
        Self::Soldier(id)
    }
}

impl From<CivilianId> for ActorId {
    fn from(id: CivilianId) -> Self {
        Self::Civilian(id)
    }
}

impl From<PcId> for HumanId {
    fn from(id: PcId) -> Self {
        Self::Pc(id)
    }
}

impl From<SoldierId> for HumanId {
    fn from(id: SoldierId) -> Self {
        Self::Soldier(id)
    }
}

impl From<CivilianId> for HumanId {
    fn from(id: CivilianId) -> Self {
        Self::Civilian(id)
    }
}

impl From<SoldierId> for NpcId {
    fn from(id: SoldierId) -> Self {
        Self::Soldier(id)
    }
}

impl From<CivilianId> for NpcId {
    fn from(id: CivilianId) -> Self {
        Self::Civilian(id)
    }
}

impl From<BonusId> for ObjectId {
    fn from(id: BonusId) -> Self {
        Self::Bonus(id)
    }
}

impl From<ScrollId> for ObjectId {
    fn from(id: ScrollId) -> Self {
        Self::Scroll(id)
    }
}

impl From<ProjectileId> for ObjectId {
    fn from(id: ProjectileId) -> Self {
        Self::Projectile(id)
    }
}

impl From<NetId> for ObjectId {
    fn from(id: NetId) -> Self {
        Self::Net(id)
    }
}

impl From<ActorId> for HumanId {
    fn from(id: ActorId) -> Self {
        match id {
            ActorId::Pc(id) => Self::Pc(id),
            ActorId::Soldier(id) => Self::Soldier(id),
            ActorId::Civilian(id) => Self::Civilian(id),
        }
    }
}

impl From<NpcId> for HumanId {
    fn from(id: NpcId) -> Self {
        match id {
            NpcId::Soldier(id) => Self::Soldier(id),
            NpcId::Civilian(id) => Self::Civilian(id),
        }
    }
}

impl From<ActorId> for EntityId {
    fn from(id: ActorId) -> Self {
        match id {
            ActorId::Pc(id) => Self::Pc(id),
            ActorId::Soldier(id) => Self::Soldier(id),
            ActorId::Civilian(id) => Self::Civilian(id),
        }
    }
}

impl From<HumanId> for EntityId {
    fn from(id: HumanId) -> Self {
        match id {
            HumanId::Pc(id) => Self::Pc(id),
            HumanId::Soldier(id) => Self::Soldier(id),
            HumanId::Civilian(id) => Self::Civilian(id),
        }
    }
}

impl From<NpcId> for EntityId {
    fn from(id: NpcId) -> Self {
        match id {
            NpcId::Soldier(id) => Self::Soldier(id),
            NpcId::Civilian(id) => Self::Civilian(id),
        }
    }
}

impl From<ObjectId> for EntityId {
    fn from(id: ObjectId) -> Self {
        match id {
            ObjectId::Bonus(id) => Self::Bonus(id),
            ObjectId::Scroll(id) => Self::Scroll(id),
            ObjectId::Projectile(id) => Self::Projectile(id),
            ObjectId::Net(id) => Self::Net(id),
        }
    }
}

impl From<PcId> for EntityId {
    fn from(id: PcId) -> Self {
        Self::Pc(id)
    }
}

impl From<SoldierId> for EntityId {
    fn from(id: SoldierId) -> Self {
        Self::Soldier(id)
    }
}

impl From<CivilianId> for EntityId {
    fn from(id: CivilianId) -> Self {
        Self::Civilian(id)
    }
}

impl From<FxId> for EntityId {
    fn from(id: FxId) -> Self {
        Self::Fx(id)
    }
}

impl From<TargetId> for EntityId {
    fn from(id: TargetId) -> Self {
        Self::Target(id)
    }
}

impl From<BonusId> for EntityId {
    fn from(id: BonusId) -> Self {
        Self::Bonus(id)
    }
}

impl From<ScrollId> for EntityId {
    fn from(id: ScrollId) -> Self {
        Self::Scroll(id)
    }
}

impl From<ProjectileId> for EntityId {
    fn from(id: ProjectileId) -> Self {
        Self::Projectile(id)
    }
}

impl From<NetId> for EntityId {
    fn from(id: NetId) -> Self {
        Self::Net(id)
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}({})", self.kind(), self.index())
    }
}
