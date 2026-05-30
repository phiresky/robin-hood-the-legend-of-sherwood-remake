//! Entity table wrapper and typed iteration helpers.

use serde::{Deserialize, Serialize};

use crate::element::{
    ActorCivilian, ActorPc, ActorSoldier, Camp, ElementBonus, ElementFx, ElementNet,
    ElementProjectile, ElementScroll, ElementTarget, Entity, Human,
};
use crate::entity_id::{
    ActorId, BonusId, CivilianId, EntityId, FxId, HumanId, NetId, NpcId, ObjectId, PcId,
    ProjectileId, ScrollId, SoldierId, TargetId,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
#[serde(transparent)]
pub struct Entities(Vec<Option<Entity>>);

impl Entities {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_slots(slots: Vec<Option<Entity>>) -> Self {
        Self(slots)
    }

    pub fn into_slots(self) -> Vec<Option<Entity>> {
        self.0
    }

    pub fn occupied(&self) -> impl Iterator<Item = (EntityId, &Entity)> + '_ {
        self.0.iter().enumerate().filter_map(|(idx, slot)| {
            slot.as_ref()
                .map(|entity| (EntityId::new(idx as u32, entity.entity_id_kind()), entity))
        })
    }

    pub fn occupied_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut Entity)> + '_ {
        self.0.iter_mut().enumerate().filter_map(|(idx, slot)| {
            slot.as_mut()
                .map(|entity| (EntityId::new(idx as u32, entity.entity_id_kind()), entity))
        })
    }

    pub fn actors(&self) -> impl Iterator<Item = (ActorId, &Entity)> + '_ {
        self.occupied().filter_map(|(id, entity)| match id {
            EntityId::Pc(id) => Some((ActorId::Pc(id), entity)),
            EntityId::Soldier(id) => Some((ActorId::Soldier(id), entity)),
            EntityId::Civilian(id) => Some((ActorId::Civilian(id), entity)),
            _ => None,
        })
    }

    pub fn actors_mut(&mut self) -> impl Iterator<Item = (ActorId, &mut Entity)> + '_ {
        self.occupied_mut().filter_map(|(id, entity)| match id {
            EntityId::Pc(id) => Some((ActorId::Pc(id), entity)),
            EntityId::Soldier(id) => Some((ActorId::Soldier(id), entity)),
            EntityId::Civilian(id) => Some((ActorId::Civilian(id), entity)),
            _ => None,
        })
    }

    pub fn humans(&self) -> impl Iterator<Item = (HumanId, &Entity)> + '_ {
        self.occupied().filter_map(|(id, entity)| match id {
            EntityId::Pc(id) => Some((HumanId::Pc(id), entity)),
            EntityId::Soldier(id) => Some((HumanId::Soldier(id), entity)),
            EntityId::Civilian(id) => Some((HumanId::Civilian(id), entity)),
            _ => None,
        })
    }

    pub fn humans_mut(&mut self) -> impl Iterator<Item = (HumanId, &mut Entity)> + '_ {
        self.occupied_mut().filter_map(|(id, entity)| match id {
            EntityId::Pc(id) => Some((HumanId::Pc(id), entity)),
            EntityId::Soldier(id) => Some((HumanId::Soldier(id), entity)),
            EntityId::Civilian(id) => Some((HumanId::Civilian(id), entity)),
            _ => None,
        })
    }

    pub fn npcs(&self) -> impl Iterator<Item = (NpcId, &Entity)> + '_ {
        self.occupied().filter_map(|(id, entity)| match id {
            EntityId::Soldier(id) => Some((NpcId::Soldier(id), entity)),
            EntityId::Civilian(id) => Some((NpcId::Civilian(id), entity)),
            _ => None,
        })
    }

    pub fn npc_ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.npcs().map(|(id, _)| EntityId::from(id))
    }

    pub fn npcs_mut(&mut self) -> impl Iterator<Item = (NpcId, &mut Entity)> + '_ {
        self.occupied_mut().filter_map(|(id, entity)| match id {
            EntityId::Soldier(id) => Some((NpcId::Soldier(id), entity)),
            EntityId::Civilian(id) => Some((NpcId::Civilian(id), entity)),
            _ => None,
        })
    }

    pub fn objects(&self) -> impl Iterator<Item = (ObjectId, &Entity)> + '_ {
        self.occupied().filter_map(|(id, entity)| match id {
            EntityId::Bonus(id) => Some((ObjectId::Bonus(id), entity)),
            EntityId::Scroll(id) => Some((ObjectId::Scroll(id), entity)),
            EntityId::Projectile(id) => Some((ObjectId::Projectile(id), entity)),
            EntityId::Net(id) => Some((ObjectId::Net(id), entity)),
            _ => None,
        })
    }

    pub fn objects_mut(&mut self) -> impl Iterator<Item = (ObjectId, &mut Entity)> + '_ {
        self.occupied_mut().filter_map(|(id, entity)| match id {
            EntityId::Bonus(id) => Some((ObjectId::Bonus(id), entity)),
            EntityId::Scroll(id) => Some((ObjectId::Scroll(id), entity)),
            EntityId::Projectile(id) => Some((ObjectId::Projectile(id), entity)),
            EntityId::Net(id) => Some((ObjectId::Net(id), entity)),
            _ => None,
        })
    }

    pub fn pcs(&self) -> impl Iterator<Item = (PcId, &ActorPc)> + '_ {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Pc(entity)) => Some((PcId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn pcs_mut(&mut self) -> impl Iterator<Item = (PcId, &mut ActorPc)> + '_ {
        self.0
            .iter_mut()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Pc(entity)) => Some((PcId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn soldiers(&self) -> impl Iterator<Item = (SoldierId, &ActorSoldier)> + '_ {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Soldier(entity)) => Some((SoldierId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn soldier_ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.soldiers().map(|(id, _)| EntityId::from(id))
    }

    pub fn fighter_ids_for_camp(&self, camp: Camp) -> impl Iterator<Item = EntityId> + '_ {
        self.occupied()
            .filter_map(move |(id, entity)| match entity {
                Entity::Pc(_) | Entity::Soldier(_) if entity.camp() == camp => Some(id),
                _ => None,
            })
    }

    pub fn soldier_ids_for_camp(&self, camp: Camp) -> impl Iterator<Item = EntityId> + '_ {
        self.soldiers()
            .filter_map(move |(id, soldier)| (soldier.camp() == camp).then_some(EntityId::from(id)))
    }

    pub fn soldiers_mut(&mut self) -> impl Iterator<Item = (SoldierId, &mut ActorSoldier)> + '_ {
        self.0
            .iter_mut()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Soldier(entity)) => Some((SoldierId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn civilians(&self) -> impl Iterator<Item = (CivilianId, &ActorCivilian)> + '_ {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Civilian(entity)) => Some((CivilianId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn civilians_mut(&mut self) -> impl Iterator<Item = (CivilianId, &mut ActorCivilian)> + '_ {
        self.0
            .iter_mut()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Civilian(entity)) => Some((CivilianId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn fxs(&self) -> impl Iterator<Item = (FxId, &ElementFx)> + '_ {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Fx(entity)) => Some((FxId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn fxs_mut(&mut self) -> impl Iterator<Item = (FxId, &mut ElementFx)> + '_ {
        self.0
            .iter_mut()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Fx(entity)) => Some((FxId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn targets(&self) -> impl Iterator<Item = (TargetId, &ElementTarget)> + '_ {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Target(entity)) => Some((TargetId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn bonuses(&self) -> impl Iterator<Item = (BonusId, &ElementBonus)> + '_ {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Bonus(entity)) => Some((BonusId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn scrolls(&self) -> impl Iterator<Item = (ScrollId, &ElementScroll)> + '_ {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Scroll(entity)) => Some((ScrollId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn projectiles(&self) -> impl Iterator<Item = (ProjectileId, &ElementProjectile)> + '_ {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Projectile(entity)) => Some((ProjectileId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn projectiles_mut(
        &mut self,
    ) -> impl Iterator<Item = (ProjectileId, &mut ElementProjectile)> + '_ {
        self.0
            .iter_mut()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Projectile(entity)) => Some((ProjectileId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn nets(&self) -> impl Iterator<Item = (NetId, &ElementNet)> + '_ {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Net(entity)) => Some((NetId(idx as u32), entity)),
                _ => None,
            })
    }

    pub fn nets_mut(&mut self) -> impl Iterator<Item = (NetId, &mut ElementNet)> + '_ {
        self.0
            .iter_mut()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(Entity::Net(entity)) => Some((NetId(idx as u32), entity)),
                _ => None,
            })
    }
}

impl std::ops::Deref for Entities {
    type Target = Vec<Option<Entity>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Entities {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<Vec<Option<Entity>>> for Entities {
    fn from(slots: Vec<Option<Entity>>) -> Self {
        Self::from_slots(slots)
    }
}

impl From<Entities> for Vec<Option<Entity>> {
    fn from(entities: Entities) -> Self {
        entities.into_slots()
    }
}
