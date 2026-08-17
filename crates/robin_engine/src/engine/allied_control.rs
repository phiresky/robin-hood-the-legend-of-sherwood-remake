//! Direct-control orders for Royalist soldier NPCs.

use crate::allied_control::{
    AlliedDuty, AlliedFormation, AlliedPinnedGroup, AlliedSoldierOrder, AlliedStance,
};
use crate::coordinates::MapPoint;
use crate::element::{Camp, Entity, EntityId};

use super::{EngineInner, LevelAssets};

const ARRIVAL_DISTANCE: f32 = 32.0;
const FOLLOW_DISTANCE: f32 = 85.0;
const FOLLOW_REPATH_DISTANCE: f32 = 24.0;
const FORMATION_SPACING: f32 = 24.0;
const COMPACT_MAX_RADIUS: f32 = 54.0;

fn distance_squared(a: MapPoint, b: MapPoint) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patrol_column_uses_tight_staggered_pairs() {
        let offsets = patrol_column_offsets(5);
        assert_eq!(offsets[0], MapPoint::ZERO);
        assert_eq!(offsets[1].y, offsets[2].y);
        assert_eq!(offsets[1].x, -offsets[2].x);
        assert!(offsets[3].y < offsets[1].y);
    }
}

fn patrol_column_offsets(count: usize) -> Vec<MapPoint> {
    (0..count)
        .map(|index| {
            if index == 0 {
                MapPoint::ZERO
            } else {
                let rank = index.div_ceil(2) as f32;
                let side = if index.is_multiple_of(2) { 1.0 } else { -1.0 };
                MapPoint::new(side * FORMATION_SPACING * 0.65, -rank * FORMATION_SPACING)
            }
        })
        .collect()
}

fn rotate_formation_offset(local: MapPoint, forward: MapPoint) -> MapPoint {
    let right = MapPoint::new(-forward.y, forward.x);
    MapPoint::new(
        right.x * local.x + forward.x * local.y,
        right.y * local.x + forward.y * local.y,
    )
}

fn normalized_direction(from: MapPoint, to: MapPoint) -> MapPoint {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length = (dx * dx + dy * dy).sqrt();
    if length > f32::EPSILON {
        MapPoint::new(dx / length, dy / length)
    } else {
        MapPoint::new(0.0, -1.0)
    }
}

impl EngineInner {
    pub fn allied_selection(&self, player_id: crate::player_command::PlayerId) -> &[EntityId] {
        self.players
            .allied
            .seats
            .get(player_id.0 as usize)
            .map(|state| state.selection.as_slice())
            .unwrap_or(&[])
    }

    pub fn allied_pinned_groups(
        &self,
        player_id: crate::player_command::PlayerId,
    ) -> &[AlliedPinnedGroup] {
        self.players
            .allied
            .seats
            .get(player_id.0 as usize)
            .map(|state| state.pinned_groups.as_slice())
            .unwrap_or(&[])
    }

    pub fn allied_first_visible_portrait(
        &self,
        player_id: crate::player_command::PlayerId,
    ) -> usize {
        self.players
            .allied
            .seats
            .get(player_id.0 as usize)
            .map(|state| state.first_visible_portrait)
            .unwrap_or(0)
    }

    pub fn allied_order(&self, soldier: EntityId) -> Option<&AlliedSoldierOrder> {
        self.players.allied.orders.get(&soldier)
    }

    pub fn find_controllable_allied_soldier(
        &self,
        assets: &LevelAssets,
        draw_order: &[EntityId],
        mouse_map: MapPoint,
    ) -> Option<EntityId> {
        draw_order.iter().rev().copied().find(|id| {
            if !self.is_controllable_allied_soldier(*id) {
                return false;
            }
            let entity = self
                .get_entity(*id)
                .expect("controllable allied soldier disappeared during hit-test");
            self.is_point_on_sprite(assets, entity, mouse_map, entity.element_data().blipped)
        })
    }

    pub(crate) fn is_controllable_allied_soldier(&self, id: EntityId) -> bool {
        matches!(self.get_entity(id), Some(Entity::Soldier(s))
            if s.soldier.cached_camp == Camp::Royalists
                && s.element.active
                && s.npc.life_points > 0
                && !s.human.unconscious)
    }

    pub(crate) fn select_allied_soldiers(
        &mut self,
        seat: usize,
        soldiers: &[EntityId],
        append: bool,
    ) {
        let valid: Vec<_> = soldiers
            .iter()
            .copied()
            .filter(|id| self.is_controllable_allied_soldier(*id))
            .collect();
        let newly_selected = {
            let state = self.players.allied.ensure_seat(seat);
            let previous = state.selection.clone();
            if !append {
                state.selection.clear();
            }
            for &id in &valid {
                if !state.selection.contains(&id) {
                    state.selection.push(id);
                }
            }
            state
                .selection
                .iter()
                .copied()
                .filter(|id| !previous.contains(id))
                .collect::<Vec<_>>()
        };

        for id in newly_selected {
            let entity = self
                .get_entity_mut(id)
                .unwrap_or_else(|| panic!("newly selected allied soldier {id:?} disappeared"));
            entity
                .human_data_mut()
                .expect("controllable allied soldier has no human data")
                .start_hulk(true, 1.0);
        }
    }

    /// Advance selection-outline fades on controllable allied soldiers.
    ///
    /// PCs use `PcData::already_selected` to detect the selection edge.
    /// Allied selection seeds the fade directly in `select_allied_soldiers`,
    /// so this pass only has to advance an animation already in flight.
    pub(super) fn refresh_allied_selection_hulks(&mut self) {
        let soldier_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for id in soldier_ids {
            if !self.is_controllable_allied_soldier(id) {
                continue;
            }
            let entity = self
                .get_entity_mut(id)
                .unwrap_or_else(|| panic!("controllable allied soldier {id:?} disappeared"));
            let human = entity
                .human_data_mut()
                .expect("controllable allied soldier has no human data");
            if human.running_hulk == 0 {
                continue;
            }
            assert!(
                human.time_hulk > 0,
                "running allied selection hulk has zero duration"
            );
            human.running_hulk -= 1;
            if human.running_hulk > 0 {
                let ratio = human.running_hulk as f32 / human.time_hulk as f32;
                human.hulk_level = if human.hulk_direction {
                    40 + (60.0 * ratio) as u16
                } else {
                    40 + (60.0 * (1.0 - ratio)) as u16
                };
            } else {
                human.hulk_direction = true;
            }
        }
    }

    pub(crate) fn box_select_allied_soldiers(
        &mut self,
        seat: usize,
        pt1: MapPoint,
        pt2: MapPoint,
        shift: bool,
    ) {
        let selection_box = crate::sprite::BBox::new(
            crate::coordinates::ScreenPoint::new(pt1.x.min(pt2.x), pt1.y.min(pt2.y)),
            crate::coordinates::ScreenPoint::new(pt1.x.max(pt2.x), pt1.y.max(pt2.y)),
        );
        let selected: Vec<_> = self
            .world
            .entities
            .npc_ids()
            .filter(|id| self.is_controllable_allied_soldier(*id))
            .filter(|id| {
                let entity = self
                    .get_entity(*id)
                    .expect("controllable allied soldier disappeared during box selection");
                let pos = entity.element_data().position_map();
                let sprite_box = entity
                    .sprite()
                    .bounding_box_at(entity.cxx_position_sprite());
                selection_box.is_intersecting(&sprite_box)
                    || selection_box
                        .contains_point(crate::coordinates::ScreenPoint::new(pos.x, pos.y))
            })
            .collect();
        self.select_allied_soldiers(seat, &selected, shift);
    }

    pub(crate) fn pin_allied_selection(&mut self, seat: usize) {
        let members = self.players.allied.ensure_seat(seat).selection.clone();
        if members.is_empty() {
            return;
        }
        let already_pinned = self.players.allied.seats[seat]
            .pinned_groups
            .iter()
            .any(|group| group.members == members);
        if already_pinned {
            return;
        }
        let id = self.players.allied.next_group_id;
        self.players.allied.next_group_id = id
            .checked_add(1)
            .expect("allied pinned-group id space exhausted");
        self.players.allied.seats[seat]
            .pinned_groups
            .push(AlliedPinnedGroup { id, members });
    }

    pub(crate) fn select_allied_group(&mut self, seat: usize, group_id: u32, append: bool) {
        let members = self
            .players
            .allied
            .ensure_seat(seat)
            .pinned_groups
            .iter()
            .find(|group| group.id == group_id)
            .unwrap_or_else(|| panic!("selected missing allied pinned group {group_id}"))
            .members
            .clone();
        self.select_allied_soldiers(seat, &members, append);
    }

    pub(crate) fn unpin_allied_group(&mut self, seat: usize, group_id: u32) {
        let groups = &mut self.players.allied.ensure_seat(seat).pinned_groups;
        let index = groups
            .iter()
            .position(|group| group.id == group_id)
            .unwrap_or_else(|| panic!("unpin requested for missing allied group {group_id}"));
        groups.remove(index);
    }

    pub(crate) fn page_allied_portraits(&mut self, seat: usize, delta: i8) {
        let hero_count = self.displayed_pc_ids().len();
        let state = self.players.allied.ensure_seat(seat);
        let transient_count = usize::from(
            !state.selection.is_empty()
                && !state
                    .pinned_groups
                    .iter()
                    .any(|group| group.members == state.selection),
        );
        let count = hero_count + state.pinned_groups.len() + transient_count;
        if count == 0 {
            state.first_visible_portrait = 0;
            return;
        }
        state.first_visible_portrait = if delta < 0 {
            (state.first_visible_portrait + count - 1) % count
        } else {
            (state.first_visible_portrait + 1) % count
        };
    }

    fn set_allied_ai_locked(&mut self, id: EntityId, locked: bool) {
        let entity = self
            .get_entity_mut(id)
            .unwrap_or_else(|| panic!("controlled allied soldier {id:?} disappeared"));
        let Some(ai) = entity.ai_controller_mut() else {
            panic!("controlled allied soldier {id:?} has no AI controller");
        };
        ai.script_locked = locked;
        ai.remember_events = locked;
    }

    fn allied_formation_slots(
        &self,
        assets: &LevelAssets,
        soldiers: &[EntityId],
        destination: MapPoint,
        formation: AlliedFormation,
    ) -> Vec<(EntityId, MapPoint)> {
        if soldiers.is_empty() {
            return Vec::new();
        }
        let positions: Vec<_> = soldiers
            .iter()
            .map(|id| {
                self.get_entity(*id)
                    .unwrap_or_else(|| panic!("forming allied soldier {id:?} disappeared"))
                    .element_data()
                    .position_map()
            })
            .collect();
        let count = positions.len() as f32;
        let centroid = MapPoint::new(
            positions.iter().map(|point| point.x).sum::<f32>() / count,
            positions.iter().map(|point| point.y).sum::<f32>() / count,
        );
        let forward = normalized_direction(centroid, destination);

        let offsets = match formation {
            AlliedFormation::Compact => {
                let mut offsets: Vec<_> = positions
                    .iter()
                    .map(|point| MapPoint::new(point.x - centroid.x, point.y - centroid.y))
                    .collect();
                let radius = offsets
                    .iter()
                    .map(|point| (point.x * point.x + point.y * point.y).sqrt())
                    .fold(0.0, f32::max);
                if radius > COMPACT_MAX_RADIUS {
                    let scale = COMPACT_MAX_RADIUS / radius;
                    for offset in &mut offsets {
                        offset.x *= scale;
                        offset.y *= scale;
                    }
                }
                offsets
            }
            AlliedFormation::PatrolColumn => patrol_column_offsets(soldiers.len())
                .into_iter()
                .map(|offset| rotate_formation_offset(offset, forward))
                .collect(),
            AlliedFormation::Battle => {
                #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
                enum Role {
                    Shield,
                    Melee,
                    Ranged,
                }
                let mut ranked: Vec<_> = soldiers
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(original_index, id)| {
                        let soldier = self
                            .get_entity(id)
                            .and_then(Entity::soldier_data)
                            .unwrap_or_else(|| {
                                panic!("allied formation member {id:?} is not a soldier")
                            });
                        let profile = assets
                            .profile_manager
                            .get_soldier(soldier.soldier_profile_index)
                            .unwrap_or_else(|| {
                                panic!(
                                    "allied soldier {id:?} requires missing profile {:?}",
                                    soldier.soldier_profile_index
                                )
                            });
                        let weapon = assets
                            .profile_manager
                            .get_hth_weapon(profile.hth_weapon_id)
                            .unwrap_or_else(|| {
                                panic!(
                                    "allied soldier {id:?} requires missing HtH weapon {}",
                                    profile.hth_weapon_id
                                )
                            });
                        let role = if weapon.shield || profile.formation {
                            Role::Shield
                        } else if profile.shooting > 0 && profile.shooting_weapon_id != 0 {
                            Role::Ranged
                        } else {
                            Role::Melee
                        };
                        (role, original_index, id)
                    })
                    .collect();
                ranked.sort_by_key(|&(role, original_index, _)| (role, original_index));
                let front_count = ranked
                    .iter()
                    .filter(|(role, _, _)| *role != Role::Ranged)
                    .count();
                let ranged_count = ranked.len() - front_count;
                let row_width = soldiers.len().min(5).max(1);
                let mut by_id = std::collections::BTreeMap::new();
                let mut front_index = 0;
                let mut rear_index = 0;
                for (role, _, id) in ranked {
                    let (index, depth) = if role == Role::Ranged {
                        let index = rear_index;
                        rear_index += 1;
                        let occupied_front_rows = front_count.div_ceil(row_width);
                        let first_rear_depth = -(occupied_front_rows as f32) * FORMATION_SPACING;
                        (
                            index,
                            first_rear_depth - (index / row_width) as f32 * FORMATION_SPACING,
                        )
                    } else {
                        let index = front_index;
                        front_index += 1;
                        (index, (index / row_width) as f32 * -FORMATION_SPACING)
                    };
                    let members_in_row = if role == Role::Ranged {
                        ranged_count
                            .saturating_sub((index / row_width) * row_width)
                            .min(row_width)
                    } else {
                        front_count
                            .saturating_sub((index / row_width) * row_width)
                            .min(row_width)
                    };
                    let column = index % row_width;
                    let lateral = (column as f32 - (members_in_row.saturating_sub(1) as f32 / 2.0))
                        * FORMATION_SPACING;
                    by_id.insert(
                        id,
                        rotate_formation_offset(MapPoint::new(lateral, depth), forward),
                    );
                }
                soldiers
                    .iter()
                    .map(|id| {
                        *by_id.get(id).unwrap_or_else(|| {
                            panic!("battle formation lost allied soldier {id:?}")
                        })
                    })
                    .collect()
            }
        };
        soldiers
            .iter()
            .copied()
            .zip(offsets)
            .map(|(id, offset)| {
                (
                    id,
                    MapPoint::new(destination.x + offset.x, destination.y + offset.y),
                )
            })
            .collect()
    }

    fn move_allied_to_slots(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        soldiers: &[EntityId],
        destination: MapPoint,
        running: bool,
        formation: AlliedFormation,
    ) -> Vec<(EntityId, MapPoint)> {
        let valid: Vec<_> = soldiers
            .iter()
            .copied()
            .filter(|id| self.is_controllable_allied_soldier(*id))
            .collect();
        let slots = self.allied_formation_slots(assets, &valid, destination, formation);
        for &(id, slot) in &slots {
            self.set_allied_ai_locked(id, true);
            self.perform_group_move(sim, assets, &[id], slot, running, true, None);
        }
        slots
    }

    pub(crate) fn command_allied_move(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        soldiers: &[EntityId],
        destination: MapPoint,
        running: bool,
        formation: AlliedFormation,
    ) {
        for (id, slot) in
            self.move_allied_to_slots(sim, assets, soldiers, destination, running, formation)
        {
            let stance = self
                .players
                .allied
                .orders
                .get(&id)
                .map(|order| order.stance)
                .unwrap_or_default();
            self.players.allied.orders.insert(
                id,
                AlliedSoldierOrder {
                    stance,
                    formation,
                    duty: AlliedDuty::Hold { anchor: slot },
                    last_destination: slot,
                },
            );
        }
    }

    pub(crate) fn set_allied_stance(&mut self, soldiers: &[EntityId], stance: AlliedStance) {
        for &id in soldiers {
            if !self.is_controllable_allied_soldier(id) {
                continue;
            }
            let position = self
                .get_entity(id)
                .expect("validated allied soldier disappeared")
                .element_data()
                .position_map();
            let order = self
                .players
                .allied
                .orders
                .entry(id)
                .or_insert(AlliedSoldierOrder {
                    stance,
                    formation: AlliedFormation::default(),
                    duty: AlliedDuty::Hold { anchor: position },
                    last_destination: position,
                });
            order.stance = stance;
            if stance == AlliedStance::Hold {
                order.duty = AlliedDuty::Hold { anchor: position };
                self.stop_owner(id, crate::sequence::SequencePriority::Normal);
            }
            self.set_allied_ai_locked(id, stance != AlliedStance::Aggressive);
        }
    }

    pub(crate) fn set_allied_formation(
        &mut self,
        soldiers: &[EntityId],
        formation: AlliedFormation,
    ) {
        for &id in soldiers {
            if !self.is_controllable_allied_soldier(id) {
                continue;
            }
            let position = self
                .get_entity(id)
                .expect("validated allied soldier disappeared")
                .element_data()
                .position_map();
            let order = self
                .players
                .allied
                .orders
                .entry(id)
                .or_insert(AlliedSoldierOrder {
                    stance: AlliedStance::default(),
                    formation,
                    duty: AlliedDuty::Hold { anchor: position },
                    last_destination: position,
                });
            order.formation = formation;
        }
    }

    pub(crate) fn set_allied_patrol(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        soldiers: &[EntityId],
        destination: MapPoint,
        formation: AlliedFormation,
    ) {
        let slots = self.move_allied_to_slots(sim, assets, soldiers, destination, false, formation);
        for (id, slot) in slots {
            let origin = self
                .get_entity(id)
                .expect("patrolling allied soldier disappeared")
                .element_data()
                .position_map();
            let stance = self
                .players
                .allied
                .orders
                .get(&id)
                .map(|order| order.stance)
                .unwrap_or_default();
            self.players.allied.orders.insert(
                id,
                AlliedSoldierOrder {
                    stance,
                    formation,
                    duty: AlliedDuty::Patrol {
                        points: [origin, slot],
                        // The initial movement is already heading to point 1.
                        // Once it arrives the tick flips back to point 0.
                        next: 1,
                    },
                    last_destination: slot,
                },
            );
        }
    }

    pub(crate) fn set_allied_follow(
        &mut self,
        assets: &LevelAssets,
        soldiers: &[EntityId],
        hero: EntityId,
        formation: AlliedFormation,
    ) {
        if self.get_entity(hero).and_then(Entity::pc_data).is_none() {
            panic!("allied follow target {hero:?} is not a PC");
        }
        let valid: Vec<_> = soldiers
            .iter()
            .copied()
            .filter(|id| self.is_controllable_allied_soldier(*id))
            .collect();
        let hero_position = self
            .get_entity(hero)
            .expect("validated allied follow hero disappeared")
            .element_data()
            .position_map();
        let slots = self.allied_formation_slots(assets, &valid, hero_position, formation);
        for (id, slot) in slots {
            let offset = MapPoint::new(slot.x - hero_position.x, slot.y - hero_position.y);
            let current_position = self
                .get_entity(id)
                .expect("validated allied follower disappeared")
                .element_data()
                .position_map();
            let stance = self
                .players
                .allied
                .orders
                .get(&id)
                .map(|order| order.stance)
                .unwrap_or_default();
            self.set_allied_ai_locked(id, stance != AlliedStance::Aggressive);
            self.players.allied.orders.insert(
                id,
                AlliedSoldierOrder {
                    stance,
                    formation,
                    duty: AlliedDuty::Follow { hero, offset },
                    // Causes the next allied-control tick to issue an initial
                    // route when this soldier is outside the follow radius.
                    last_destination: current_position,
                },
            );
        }
    }

    pub(crate) fn release_allied_control(&mut self) {
        let controlled: Vec<_> = self.players.allied.orders.keys().copied().collect();
        for id in controlled {
            if self.get_entity(id).is_some() {
                self.set_allied_ai_locked(id, false);
            }
        }
        self.players.allied = Default::default();
    }

    pub(super) fn tick_allied_control(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        if !self.control.frame_counter.is_multiple_of(5) {
            return;
        }
        let orders: Vec<_> = self
            .players
            .allied
            .orders
            .iter()
            .map(|(&id, order)| (id, order.clone()))
            .collect();
        let mut remove = Vec::new();
        for (id, mut order) in orders {
            if !self.is_controllable_allied_soldier(id) {
                remove.push(id);
                continue;
            }
            let (position, threatened) = match self.get_entity(id) {
                Some(Entity::Soldier(s)) => (
                    s.element.position_map(),
                    s.npc.alerted || s.npc.maximal_detection_suspect > 0,
                ),
                _ => unreachable!("validated allied soldier changed entity kind"),
            };
            let ai_locked = match order.stance {
                AlliedStance::Hold => true,
                AlliedStance::Defensive => !threatened,
                AlliedStance::Aggressive => false,
            };
            self.set_allied_ai_locked(id, ai_locked);
            if !ai_locked || order.stance == AlliedStance::Hold {
                continue;
            }

            let destination = match &mut order.duty {
                AlliedDuty::Hold { .. } => None,
                AlliedDuty::Patrol { points, next } => {
                    let target = points[usize::from(*next)];
                    if distance_squared(position, target) <= ARRIVAL_DISTANCE * ARRIVAL_DISTANCE {
                        *next = (*next + 1) % 2;
                        Some(points[usize::from(*next)])
                    } else {
                        None
                    }
                }
                AlliedDuty::Follow { hero, offset } => {
                    let Some(hero_position) = self.get_entity(*hero).and_then(|hero| {
                        hero.pc_data().map(|_| hero.element_data().position_map())
                    }) else {
                        tracing::warn!(?id, ?hero, "allied follow target disappeared; holding");
                        order.duty = AlliedDuty::Hold { anchor: position };
                        self.players.allied.orders.insert(id, order);
                        continue;
                    };
                    let target =
                        MapPoint::new(hero_position.x + offset.x, hero_position.y + offset.y);
                    (distance_squared(position, target) > FOLLOW_DISTANCE * FOLLOW_DISTANCE
                        && distance_squared(order.last_destination, target)
                            > FOLLOW_REPATH_DISTANCE * FOLLOW_REPATH_DISTANCE)
                        .then_some(target)
                }
            };
            if let Some(destination) = destination {
                self.perform_group_move(sim, assets, &[id], destination, false, true, None);
                order.last_destination = destination;
            }
            self.players.allied.orders.insert(id, order);
        }
        for id in remove {
            self.players.allied.orders.remove(&id);
            for seat in &mut self.players.allied.seats {
                seat.selection.retain(|member| *member != id);
                for group in &mut seat.pinned_groups {
                    group.members.retain(|member| *member != id);
                }
                seat.pinned_groups.retain(|group| !group.members.is_empty());
            }
        }
    }
}
