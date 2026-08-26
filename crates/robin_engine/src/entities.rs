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

macro_rules! typed_entity_accessors {
    ($get:ident, $get_mut:ident, $id:ty, $as_ref:ident, $as_mut:ident, $entity:ty) => {
        pub fn $get(&self, id: $id) -> Option<&$entity> {
            self.get(id)?.$as_ref()
        }

        pub fn $get_mut(&mut self, id: $id) -> Option<&mut $entity> {
            self.get_mut(id)?.$as_mut()
        }
    };
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(transparent)]
pub struct Entities(
    Vec<Option<Entity>>,
    /// Transient per-slot invalidation counters for derived runtime caches.
    ///
    /// A mutable borrow is conservatively treated as a mutation. The counters
    /// are deliberately absent from saves and deterministic state hashes: they
    /// describe cache freshness, not gameplay state.
    #[serde(skip)]
    #[state_hash(skip)]
    #[bitcode(skip)]
    Vec<u64>,
);

impl Entities {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconstruct the original engine's sparse `marrayElements` layout.
    ///
    /// This is intentionally narrower than `From<Vec<_>>`: only legacy
    /// loaders, compatibility DTOs, and parity fixtures should manufacture
    /// raw slots. Runtime code should use typed IDs and entity accessors.
    pub fn from_legacy_slots(slots: Vec<Option<Entity>>) -> Self {
        let generations = vec![0; slots.len()];
        Self(slots, generations)
    }

    /// Exact sparse slots used by the current native engine snapshot codec.
    pub(crate) fn snapshot_slots(&self) -> &[Option<Entity>] {
        &self.0
    }

    /// Restore exact sparse slots from the current native snapshot codec.
    pub(crate) fn from_snapshot_slots(slots: Vec<Option<Entity>>) -> Self {
        let generations = vec![0; slots.len()];
        Self(slots, generations)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, entity: Option<Entity>) {
        self.1.push(u64::from(entity.is_some()));
        self.0.push(entity);
    }

    pub fn resize(&mut self, new_len: usize, value: Option<Entity>) {
        let old_len = self.0.len();
        self.0.resize(new_len, value);
        self.1.resize(new_len, 0);
        if new_len > old_len {
            let initial = u64::from(self.0[old_len..].iter().any(Option::is_some));
            self.1[old_len..].fill(initial);
        }
    }

    /// Monotonic runtime generation for a slot's entity value.
    ///
    /// Deserialized entity stores start at generation zero. Any subsequent
    /// mutable access advances the addressed slot before handing out `&mut`.
    pub(crate) fn generation<I: Into<EntityId>>(&self, id: I) -> u64 {
        self.1.get(id.into().index() as usize).copied().unwrap_or(0)
    }

    fn ensure_generation_slots(&mut self) {
        self.1.resize(self.0.len(), 0);
    }

    fn bump_generation(&mut self, index: usize) {
        self.ensure_generation_slots();
        self.1[index] = self.1[index].wrapping_add(1);
    }

    fn slots_mut(&mut self) -> impl Iterator<Item = (usize, &mut Option<Entity>, &mut u64)> + '_ {
        self.ensure_generation_slots();
        self.0
            .iter_mut()
            .zip(self.1.iter_mut())
            .enumerate()
            .map(|(index, (slot, generation))| (index, slot, generation))
    }

    /// Resolve an original-game raw element-array slot to the typed ID for
    /// the entity currently occupying it.
    ///
    /// Original: `RHEngine::GetElement` in `original-code/RHEngine.h`
    /// directly indexes `marrayElements`. Mission data uses that contract,
    /// for example `RHEngine::InitializeBuildingTenantsFromMissionStream` in
    /// `original-code/RHengine.cpp`, which reads `uwElementIndex` and passes
    /// it to `GetElement`. New simulation code should carry an [`EntityId`]
    /// instead of retaining this raw slot number.
    pub fn id_at_legacy_slot(&self, slot: u32) -> Option<EntityId> {
        let entity = self.0.get(slot as usize)?.as_ref()?;
        Some(EntityId::new(slot, entity.entity_id_kind()))
    }

    /// Access an entity through an original-game raw element-array slot.
    ///
    /// The returned ID is derived from the current occupant, so callers
    /// cannot accidentally invent an ID kind for a legacy slot.
    pub fn get_legacy_slot(&self, slot: u32) -> Option<(EntityId, &Entity)> {
        let entity = self.0.get(slot as usize)?.as_ref()?;
        let id = EntityId::new(slot, entity.entity_id_kind());
        Some((id, entity))
    }

    /// Mutably access an entity through an original-game raw element-array
    /// slot. Prefer typed accessors outside legacy parsing/script boundaries.
    pub fn get_legacy_slot_mut(&mut self, slot: u32) -> Option<(EntityId, &mut Entity)> {
        let index = slot as usize;
        if self.0.get(index)?.is_none() {
            return None;
        }
        self.bump_generation(index);
        let entity = self.0.get_mut(slot as usize)?.as_mut()?;
        let id = EntityId::new(slot, entity.entity_id_kind());
        Some((id, entity))
    }

    fn slot_matches_id(slot: &Option<Entity>, id: EntityId) -> bool {
        slot.as_ref()
            .is_none_or(|entity| entity.entity_id_kind() == id.kind())
    }

    fn checked_slot(&self, id: EntityId) -> Option<&Option<Entity>> {
        let slot = self.0.get(id.index() as usize)?;
        Self::slot_matches_id(slot, id).then_some(slot)
    }

    fn checked_slot_mut(&mut self, id: EntityId) -> Option<&mut Option<Entity>> {
        let index = id.index() as usize;
        if !Self::slot_matches_id(self.0.get(index)?, id) {
            return None;
        }
        self.bump_generation(index);
        self.0.get_mut(index)
    }

    pub fn get<I: Into<EntityId>>(&self, id: I) -> Option<&Entity> {
        let id = id.into();
        self.checked_slot(id)?.as_ref()
    }

    pub fn get_mut<I: Into<EntityId>>(&mut self, id: I) -> Option<&mut Entity> {
        let id = id.into();
        self.checked_slot_mut(id)?.as_mut()
    }

    pub fn remove<I: Into<EntityId>>(&mut self, id: I) -> Option<Entity> {
        let id = id.into();
        self.checked_slot_mut(id)?.take()
    }

    typed_entity_accessors!(get_pc, get_pc_mut, PcId, as_pc, as_pc_mut, ActorPc);
    typed_entity_accessors!(
        get_soldier,
        get_soldier_mut,
        SoldierId,
        as_soldier,
        as_soldier_mut,
        ActorSoldier
    );
    typed_entity_accessors!(
        get_civilian,
        get_civilian_mut,
        CivilianId,
        as_civilian,
        as_civilian_mut,
        ActorCivilian
    );
    typed_entity_accessors!(get_fx, get_fx_mut, FxId, as_fx, as_fx_mut, ElementFx);
    typed_entity_accessors!(
        get_target,
        get_target_mut,
        TargetId,
        as_target,
        as_target_mut,
        ElementTarget
    );
    typed_entity_accessors!(
        get_bonus,
        get_bonus_mut,
        BonusId,
        as_bonus,
        as_bonus_mut,
        ElementBonus
    );
    typed_entity_accessors!(
        get_scroll,
        get_scroll_mut,
        ScrollId,
        as_scroll,
        as_scroll_mut,
        ElementScroll
    );
    typed_entity_accessors!(
        get_projectile,
        get_projectile_mut,
        ProjectileId,
        as_projectile,
        as_projectile_mut,
        ElementProjectile
    );
    typed_entity_accessors!(get_net, get_net_mut, NetId, as_net, as_net_mut, ElementNet);

    pub fn occupied(&self) -> impl Iterator<Item = (EntityId, &Entity)> + '_ {
        self.0.iter().enumerate().filter_map(|(idx, slot)| {
            slot.as_ref()
                .map(|entity| (EntityId::new(idx as u32, entity.entity_id_kind()), entity))
        })
    }

    pub fn occupied_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut Entity)> + '_ {
        self.slots_mut().filter_map(|(idx, slot, generation)| {
            slot.as_mut().map(|entity| {
                *generation = generation.wrapping_add(1);
                (EntityId::new(idx as u32, entity.entity_id_kind()), entity)
            })
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
        self.slots_mut()
            .filter_map(|(idx, slot, generation)| match slot {
                Some(entity @ Entity::Pc(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((ActorId::Pc(PcId(idx as u32)), entity))
                }
                Some(entity @ Entity::Soldier(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((ActorId::Soldier(SoldierId(idx as u32)), entity))
                }
                Some(entity @ Entity::Civilian(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((ActorId::Civilian(CivilianId(idx as u32)), entity))
                }
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
        self.slots_mut()
            .filter_map(|(idx, slot, generation)| match slot {
                Some(entity @ Entity::Pc(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((HumanId::Pc(PcId(idx as u32)), entity))
                }
                Some(entity @ Entity::Soldier(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((HumanId::Soldier(SoldierId(idx as u32)), entity))
                }
                Some(entity @ Entity::Civilian(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((HumanId::Civilian(CivilianId(idx as u32)), entity))
                }
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
        self.npcs().map(|(id, _)| id.into())
    }

    pub fn npcs_mut(&mut self) -> impl Iterator<Item = (NpcId, &mut Entity)> + '_ {
        self.slots_mut()
            .filter_map(|(idx, slot, generation)| match slot {
                Some(entity @ Entity::Soldier(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((NpcId::Soldier(SoldierId(idx as u32)), entity))
                }
                Some(entity @ Entity::Civilian(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((NpcId::Civilian(CivilianId(idx as u32)), entity))
                }
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
        self.slots_mut()
            .filter_map(|(idx, slot, generation)| match slot {
                Some(entity @ Entity::Bonus(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((ObjectId::Bonus(BonusId(idx as u32)), entity))
                }
                Some(entity @ Entity::Scroll(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((ObjectId::Scroll(ScrollId(idx as u32)), entity))
                }
                Some(entity @ Entity::Projectile(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((ObjectId::Projectile(ProjectileId(idx as u32)), entity))
                }
                Some(entity @ Entity::Net(_)) => {
                    *generation = generation.wrapping_add(1);
                    Some((ObjectId::Net(NetId(idx as u32)), entity))
                }
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
        self.slots_mut()
            .filter_map(|(idx, slot, generation)| match slot {
                Some(Entity::Pc(entity)) => {
                    *generation = generation.wrapping_add(1);
                    Some((PcId(idx as u32), entity))
                }
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
        self.soldiers().map(|(id, _)| id.into())
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
            .filter_map(move |(id, soldier)| (soldier.camp() == camp).then_some(id.into()))
    }

    pub fn soldiers_mut(&mut self) -> impl Iterator<Item = (SoldierId, &mut ActorSoldier)> + '_ {
        self.slots_mut()
            .filter_map(|(idx, slot, generation)| match slot {
                Some(Entity::Soldier(entity)) => {
                    *generation = generation.wrapping_add(1);
                    Some((SoldierId(idx as u32), entity))
                }
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
        self.slots_mut()
            .filter_map(|(idx, slot, generation)| match slot {
                Some(Entity::Civilian(entity)) => {
                    *generation = generation.wrapping_add(1);
                    Some((CivilianId(idx as u32), entity))
                }
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
        self.slots_mut()
            .filter_map(|(idx, slot, generation)| match slot {
                Some(Entity::Fx(entity)) => {
                    *generation = generation.wrapping_add(1);
                    Some((FxId(idx as u32), entity))
                }
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

    pub fn scrolls_mut(&mut self) -> impl Iterator<Item = (ScrollId, &mut ElementScroll)> + '_ {
        self.slots_mut()
            .filter_map(|(idx, slot, generation)| match slot {
                Some(Entity::Scroll(entity)) => {
                    *generation = generation.wrapping_add(1);
                    Some((ScrollId(idx as u32), entity))
                }
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
        self.slots_mut()
            .filter_map(|(idx, slot, generation)| match slot {
                Some(Entity::Projectile(entity)) => {
                    *generation = generation.wrapping_add(1);
                    Some((ProjectileId(idx as u32), entity))
                }
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
        self.slots_mut()
            .filter_map(|(idx, slot, generation)| match slot {
                Some(Entity::Net(entity)) => {
                    *generation = generation.wrapping_add(1);
                    Some((NetId(idx as u32), entity))
                }
                _ => None,
            })
    }
}

/// An element's position as it stood at a batching boundary, in both of the
/// spaces the engine stores.
///
/// The two are related by `map = (world.x, world.y - world.z)`, but neither
/// derives the other exactly in binary32: whichever space the element last
/// wrote is its authoritative value, and recomputing the other one back rounds.
/// Recording the pair keeps a later reader on the same value the element
/// itself would have reported.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct BoundaryPosition {
    pub map: crate::coordinates::MapPoint,
    pub world: crate::coordinates::WorldPoint3D,
}

impl BoundaryPosition {
    pub fn of(element: &crate::element::ElementData) -> Self {
        Self {
            map: element.position_map(),
            world: element.position(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EntitySlots<T>(Vec<T>);

impl<T: Clone> EntitySlots<T> {
    pub fn filled(len: usize, value: T) -> Self {
        Self(vec![value; len])
    }
}

impl<T> EntitySlots<T> {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get<I: Into<EntityId>>(&self, id: I) -> Option<&T> {
        let id = id.into();
        self.0.get(id.index() as usize)
    }

    pub fn get_mut<I: Into<EntityId>>(&mut self, id: I) -> Option<&mut T> {
        let id = id.into();
        self.0.get_mut(id.index() as usize)
    }

    pub fn as_slice(&self) -> &[T] {
        &self.0
    }
}

impl<T, I: Into<EntityId>> std::ops::Index<I> for EntitySlots<T> {
    type Output = T;

    fn index(&self, index: I) -> &Self::Output {
        let index = index.into();
        &self.0[index.index() as usize]
    }
}

impl<T, I: Into<EntityId>> std::ops::IndexMut<I> for EntitySlots<T> {
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        let index = index.into();
        &mut self.0[index.index() as usize]
    }
}

impl<I: Into<EntityId>> std::ops::Index<I> for Entities {
    type Output = Option<Entity>;

    fn index(&self, index: I) -> &Self::Output {
        let id = index.into();
        self.checked_slot(id).unwrap_or_else(|| {
            panic!(
                "entity ID {id} does not address an in-bounds slot with kind {:?}",
                id.kind()
            )
        })
    }
}

impl<I: Into<EntityId>> std::ops::IndexMut<I> for Entities {
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        let id = index.into();
        self.checked_slot_mut(id).unwrap_or_else(|| {
            panic!(
                "entity ID {id} does not address an in-bounds slot with kind {:?}",
                id.kind()
            )
        })
    }
}

#[cfg(test)]
mod generation_tests {
    use super::*;
    use robin_util::state_hash::StateHash;
    use std::hash::Hasher;

    fn hash(entities: &Entities) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        entities.state_hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn mutation_generations_are_transient_cache_state() {
        let mut entities = Entities::from_legacy_slots(vec![None]);
        let id = PcId(0);
        let serialized = serde_json::to_value(&entities).expect("serialize entity slots");
        let state_hash = hash(&entities);

        let _slot = &mut entities[id];
        assert_eq!(entities.generation(id), 1);
        assert_eq!(serde_json::to_value(&entities).unwrap(), serialized);
        assert_eq!(hash(&entities), state_hash);

        let restored: Entities = serde_json::from_value(serialized).expect("restore entity slots");
        assert_eq!(restored.generation(id), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{
        ActorData, CivilianData, ElementData, FxData, HumanData, NetData, NpcData, ObjectData,
        PcData, ProjectileData, SoldierData, TargetData,
    };
    use crate::entity_id::EntityIdKind;

    const KINDS: [EntityIdKind; 9] = [
        EntityIdKind::Pc,
        EntityIdKind::Soldier,
        EntityIdKind::Civilian,
        EntityIdKind::Fx,
        EntityIdKind::Target,
        EntityIdKind::Bonus,
        EntityIdKind::Scroll,
        EntityIdKind::Projectile,
        EntityIdKind::Net,
    ];

    fn entity(kind: EntityIdKind) -> Entity {
        match kind {
            EntityIdKind::Pc => Entity::Pc(ActorPc {
                element: ElementData::default(),
                actor: ActorData::default(),
                human: HumanData::default(),
                pc: PcData::default(),
            }),
            EntityIdKind::Soldier => Entity::Soldier(ActorSoldier {
                element: ElementData::default(),
                actor: ActorData::default(),
                human: HumanData::default(),
                npc: NpcData::default(),
                soldier: SoldierData::default(),
            }),
            EntityIdKind::Civilian => Entity::Civilian(ActorCivilian {
                element: ElementData::default(),
                actor: ActorData::default(),
                human: HumanData::default(),
                npc: NpcData::default(),
                civilian: CivilianData::default(),
            }),
            EntityIdKind::Fx => Entity::Fx(ElementFx {
                element: ElementData::default(),
                fx: FxData::default(),
            }),
            EntityIdKind::Target => Entity::Target(ElementTarget {
                element: ElementData::default(),
                fx: FxData::default(),
                target: TargetData::default(),
            }),
            EntityIdKind::Bonus => Entity::Bonus(ElementBonus {
                element: ElementData::default(),
                object: ObjectData::default(),
            }),
            EntityIdKind::Scroll => Entity::Scroll(ElementScroll::default()),
            EntityIdKind::Projectile => Entity::Projectile(ElementProjectile {
                element: ElementData::default(),
                object: ObjectData::default(),
                projectile: ProjectileData::default(),
            }),
            EntityIdKind::Net => Entity::Net(ElementNet {
                element: ElementData::default(),
                object: ObjectData::default(),
                projectile: ProjectileData::default(),
                net: NetData::default(),
            }),
        }
    }

    fn all_kinds() -> Entities {
        let mut entities = Entities::new();
        for kind in KINDS {
            entities.push(Some(entity(kind)));
        }
        entities
    }

    #[test]
    fn typed_mutable_iteration_only_invalidates_matching_slots() {
        let mut entities = all_kinds();
        for (_, soldier) in entities.soldiers_mut() {
            let _ = soldier;
        }
        for (index, kind) in KINDS.into_iter().enumerate() {
            let id = EntityId::new(index as u32, kind);
            assert_eq!(
                entities.generation(id),
                1 + u64::from(kind == EntityIdKind::Soldier),
                "unexpected invalidation for {kind:?}"
            );
        }
    }

    #[test]
    fn lookups_exhaustively_reject_kind_slot_mismatches() {
        let mut entities = all_kinds();

        for expected in KINDS {
            for (slot, actual) in KINDS.into_iter().enumerate() {
                let id = EntityId::new(slot as u32, expected);
                let should_match = expected == actual;
                assert_eq!(entities.get(id).is_some(), should_match, "shared {id}");
                assert_eq!(entities.get_mut(id).is_some(), should_match, "mutable {id}");
            }
        }
    }

    #[test]
    fn removal_exhaustively_rejects_kind_slot_mismatches() {
        for actual in KINDS {
            for expected in KINDS {
                let mut entities = Entities::new();
                entities.push(Some(entity(actual)));
                let id = EntityId::new(0, expected);

                assert_eq!(entities.remove(id).is_some(), expected == actual, "{id}");
                assert_eq!(
                    entities.id_at_legacy_slot(0).map(EntityId::kind),
                    (expected != actual).then_some(actual),
                    "mismatched removal must preserve the current occupant"
                );
            }
        }
    }

    #[test]
    fn typed_accessors_cover_every_entity_kind() {
        let mut entities = all_kinds();

        assert!(entities.get_pc(PcId(0)).is_some());
        assert!(entities.get_soldier(SoldierId(1)).is_some());
        assert!(entities.get_civilian(CivilianId(2)).is_some());
        assert!(entities.get_fx(FxId(3)).is_some());
        assert!(entities.get_target(TargetId(4)).is_some());
        assert!(entities.get_bonus(BonusId(5)).is_some());
        assert!(entities.get_scroll(ScrollId(6)).is_some());
        assert!(entities.get_projectile(ProjectileId(7)).is_some());
        assert!(entities.get_net(NetId(8)).is_some());

        assert!(entities.get_pc_mut(PcId(0)).is_some());
        assert!(entities.get_soldier_mut(SoldierId(1)).is_some());
        assert!(entities.get_civilian_mut(CivilianId(2)).is_some());
        assert!(entities.get_fx_mut(FxId(3)).is_some());
        assert!(entities.get_target_mut(TargetId(4)).is_some());
        assert!(entities.get_bonus_mut(BonusId(5)).is_some());
        assert!(entities.get_scroll_mut(ScrollId(6)).is_some());
        assert!(entities.get_projectile_mut(ProjectileId(7)).is_some());
        assert!(entities.get_net_mut(NetId(8)).is_some());

        assert!(entities.get_pc(PcId(1)).is_none());
        assert!(entities.get_projectile_mut(ProjectileId(8)).is_none());
    }

    #[test]
    fn stale_empty_and_out_of_bounds_slots_do_not_resolve() {
        let mut entities = Entities::new();
        entities.push(Some(entity(EntityIdKind::Soldier)));
        entities.push(None);
        let stale = SoldierId(0);

        assert!(entities.remove(stale).is_some());
        assert!(entities.get(stale).is_none());
        assert!(entities.get_mut(stale).is_none());
        assert!(entities.remove(stale).is_none());
        assert!(entities.get(EntityId::new(1, EntityIdKind::Pc)).is_none());
        assert!(entities.get(EntityId::new(2, EntityIdKind::Pc)).is_none());
        assert!(entities.id_at_legacy_slot(0).is_none());
        assert!(entities.id_at_legacy_slot(1).is_none());
        assert!(entities.id_at_legacy_slot(2).is_none());
        assert!(entities.get_legacy_slot(0).is_none());
        assert!(entities.get_legacy_slot_mut(1).is_none());
    }

    #[test]
    fn legacy_slot_access_derives_the_current_kind() {
        let mut entities = all_kinds();

        for (slot, kind) in KINDS.into_iter().enumerate() {
            let (id, entity) = entities.get_legacy_slot(slot as u32).unwrap();
            assert_eq!(id, EntityId::new(slot as u32, kind));
            assert_eq!(entity.entity_id_kind(), kind);
        }

        let (id, entity) = entities.get_legacy_slot_mut(4).unwrap();
        assert_eq!(id, TargetId(4));
        assert!(entity.as_target_mut().is_some());
    }

    #[test]
    #[should_panic(expected = "does not address an in-bounds slot")]
    fn indexing_rejects_a_kind_mismatch() {
        let mut entities = Entities::new();
        entities.push(Some(entity(EntityIdKind::Pc)));
        let _ = &entities[SoldierId(0)];
    }

    #[test]
    fn indexing_preserves_an_empty_matching_slot() {
        let mut entities = Entities::new();
        entities.push(None);
        assert!(entities[PcId(0)].is_none());
    }
}
