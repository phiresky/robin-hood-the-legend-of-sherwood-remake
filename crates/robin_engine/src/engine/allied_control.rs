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
const FORMATION_SPACING: f32 = 19.0;
const STAGGERED_SPACING: f32 = 27.0;
const LINE_TO_COLUMN_DISTANCE: f32 = 240.0;

fn distance_squared(a: MapPoint, b: MapPoint) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn march_column_is_two_soldiers_wide() {
        let offsets = march_column_offsets(5);
        assert_eq!(offsets[0].y, offsets[1].y);
        assert_eq!(offsets[0].x, -offsets[1].x);
        assert_eq!(offsets[2].y, offsets[3].y);
        assert!(offsets[2].y < offsets[0].y);
        assert_eq!(offsets[4].x, 0.0);
    }

    #[test]
    fn line_is_wider_than_deep() {
        let offsets = row_offsets(12, 5, FORMATION_SPACING, false);
        let width = offsets.iter().map(|p| p.x).fold(f32::MIN, f32::max)
            - offsets.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        let depth = offsets.iter().map(|p| p.y).fold(f32::MIN, f32::max)
            - offsets.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        assert!(width > depth);
    }

    #[test]
    fn staggered_rows_are_laterally_offset() {
        let offsets = row_offsets(6, 3, STAGGERED_SPACING, true);
        assert_eq!(offsets[3].x - offsets[0].x, STAGGERED_SPACING * 0.5);
    }

    #[test]
    fn flank_leaves_a_center_gap() {
        let offsets = flank_offsets(8);
        assert!(
            offsets
                .iter()
                .all(|point| point.x.abs() >= FORMATION_SPACING)
        );
        assert!(offsets.iter().any(|point| point.x < 0.0));
        assert!(offsets.iter().any(|point| point.x > 0.0));
    }
}

fn march_column_offsets(count: usize) -> Vec<MapPoint> {
    (0..count)
        .map(|index| {
            let row = index / 2;
            let members_in_row = (count - row * 2).min(2);
            let column = index % 2;
            MapPoint::new(
                (column as f32 - (members_in_row.saturating_sub(1) as f32 / 2.0))
                    * FORMATION_SPACING,
                -(row as f32) * FORMATION_SPACING,
            )
        })
        .collect()
}

fn row_offsets(count: usize, columns: usize, spacing: f32, staggered: bool) -> Vec<MapPoint> {
    (0..count)
        .map(|index| {
            let row = index / columns;
            let members_in_row = (count - row * columns).min(columns);
            let column = index % columns;
            let stagger = if staggered && row % 2 == 1 {
                spacing * 0.5
            } else {
                0.0
            };
            MapPoint::new(
                (column as f32 - (members_in_row.saturating_sub(1) as f32 / 2.0)) * spacing
                    + stagger,
                -(row as f32) * spacing,
            )
        })
        .collect()
}

fn flank_offsets(count: usize) -> Vec<MapPoint> {
    let left_count = count.div_ceil(2);
    let right_count = count / 2;
    let wing = |index: usize, members: usize, side: f32| {
        let row = index / 2;
        let members_in_row = (members - row * 2).min(2);
        let column = index % 2;
        let from_center = FORMATION_SPACING * 1.2
            + (column as f32 + (2 - members_in_row) as f32 * 0.5) * FORMATION_SPACING;
        MapPoint::new(side * from_center, -(row as f32) * FORMATION_SPACING)
    };
    let mut offsets = Vec::with_capacity(count);
    for rank in 0..left_count.max(right_count) {
        if rank < left_count {
            offsets.push(wing(rank, left_count, -1.0));
        }
        if rank < right_count {
            offsets.push(wing(rank, right_count, 1.0));
        }
    }
    offsets
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
        use_marching_column: bool,
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
                    .unwrap_or_else(|| panic!("allied formation member {id:?} is not a soldier"));
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

        let travel_distance = distance_squared(centroid, destination).sqrt();
        let local_slots = match formation {
            AlliedFormation::Line
                if use_marching_column && travel_distance > LINE_TO_COLUMN_DISTANCE =>
            {
                march_column_offsets(soldiers.len())
            }
            AlliedFormation::Line => {
                let columns = ((soldiers.len() as f32 * 2.0).sqrt().ceil() as usize).max(1);
                row_offsets(soldiers.len(), columns, FORMATION_SPACING, false)
            }
            AlliedFormation::Box => {
                let columns = (soldiers.len() as f32).sqrt().ceil() as usize;
                let rows = soldiers.len().div_ceil(columns);
                let mut slots: Vec<_> = (0..soldiers.len())
                    .map(|index| {
                        let row = index / columns;
                        let members_in_row = (soldiers.len() - row * columns).min(columns);
                        let column = index % columns;
                        MapPoint::new(
                            (column as f32 - (members_in_row.saturating_sub(1) as f32 / 2.0))
                                * FORMATION_SPACING,
                            (rows.saturating_sub(1) as f32 / 2.0 - row as f32) * FORMATION_SPACING,
                        )
                    })
                    .collect();
                // The rank-to-slot assignment below places shield and melee
                // troops first. Give them the outer cells, leaving the safest
                // central cells for ranged troops.
                slots.sort_by(|a, b| {
                    let a_radius = a.x * a.x + a.y * a.y;
                    let b_radius = b.x * b.x + b.y * b.y;
                    b_radius
                        .total_cmp(&a_radius)
                        .then_with(|| a.y.total_cmp(&b.y))
                        .then_with(|| a.x.total_cmp(&b.x))
                });
                slots
            }
            AlliedFormation::Staggered => {
                let columns = ((soldiers.len() as f32 * 2.0).sqrt().ceil() as usize).max(1);
                row_offsets(soldiers.len(), columns, STAGGERED_SPACING, true)
            }
            AlliedFormation::Flank => flank_offsets(soldiers.len()),
        };

        let mut by_id = std::collections::BTreeMap::new();
        for ((_, _, id), offset) in ranked.into_iter().zip(local_slots) {
            by_id.insert(id, rotate_formation_offset(offset, forward));
        }
        let offsets: Vec<_> = soldiers
            .iter()
            .map(|id| {
                *by_id
                    .get(id)
                    .unwrap_or_else(|| panic!("formation lost allied soldier {id:?}"))
            })
            .collect();
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
        let slots = self.allied_formation_slots(assets, &valid, destination, formation, true);
        for &(id, _) in &slots {
            self.set_allied_ai_locked(id, true);
        }
        let actor_ids: Vec<_> = slots.iter().map(|(id, _)| *id).collect();
        let destinations: Vec<_> = slots.iter().map(|(_, point)| *point).collect();
        self.perform_group_move_to_slots(
            sim,
            assets,
            &actor_ids,
            destination,
            &destinations,
            running,
            true,
        );
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
        let valid: Vec<_> = soldiers
            .iter()
            .copied()
            .filter(|id| self.is_controllable_allied_soldier(*id))
            .collect();
        let deployed_slots =
            self.allied_formation_slots(assets, &valid, destination, formation, false);
        let deployed_by_id: std::collections::BTreeMap<_, _> = deployed_slots.into_iter().collect();
        for (id, slot) in
            self.move_allied_to_slots(sim, assets, &valid, destination, running, formation)
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
                    path_fallback: (distance_squared(slot, destination) > 1.0)
                        .then_some(destination),
                    deploy_destination: deployed_by_id
                        .get(&id)
                        .copied()
                        .filter(|deployed| distance_squared(*deployed, slot) > 1.0),
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
                    path_fallback: None,
                    deploy_destination: None,
                });
            order.stance = stance;
            if stance == AlliedStance::Hold {
                order.duty = AlliedDuty::Hold { anchor: position };
                order.deploy_destination = None;
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
                    path_fallback: None,
                    deploy_destination: None,
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
                    path_fallback: (distance_squared(slot, destination) > 1.0)
                        .then_some(destination),
                    deploy_destination: None,
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
        let slots = self.allied_formation_slots(assets, &valid, hero_position, formation, false);
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
                    path_fallback: None,
                    deploy_destination: None,
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

    /// Consume the one-shot center fallback for a controlled soldier whose
    /// formation slot has no path. Returning `Some(None)` identifies a
    /// controlled ally with no fallback, which must still fail immediately
    /// instead of retaining the generic PC-style 100-frame waiting pose.
    pub(super) fn allied_path_failure_fallback(
        &mut self,
        id: EntityId,
    ) -> Option<Option<MapPoint>> {
        if !self.is_controllable_allied_soldier(id) {
            return None;
        }
        let order = self.players.allied.orders.get_mut(&id).unwrap_or_else(|| {
            panic!("controllable allied soldier {id:?} has no control order after path failure")
        });
        let fallback = order.path_fallback.take();
        if let Some(point) = fallback {
            match &mut order.duty {
                AlliedDuty::Hold { anchor } => *anchor = point,
                AlliedDuty::Patrol { points, next } => points[usize::from(*next)] = point,
                AlliedDuty::Follow { .. } => {
                    // Follow targets move continuously. Suppress an immediate
                    // retry loop; the next meaningful hero displacement will
                    // request a fresh path.
                }
            }
            order.last_destination = point;
        }
        Some(fallback)
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
            let deploy_destination = order.deploy_destination.filter(|_| {
                matches!(
                    &order.duty,
                    AlliedDuty::Hold { anchor }
                        if distance_squared(position, *anchor)
                            <= ARRIVAL_DISTANCE * ARRIVAL_DISTANCE
                )
            });
            if ai_locked && let Some(destination) = deploy_destination {
                self.perform_group_move(sim, assets, &[id], destination, false, true, None);
                order.duty = AlliedDuty::Hold {
                    anchor: destination,
                };
                order.last_destination = destination;
                order.path_fallback = None;
                order.deploy_destination = None;
                self.players.allied.orders.insert(id, order);
                continue;
            }
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
