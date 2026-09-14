//! Local combat proposals scored against borrowed live fighters.

use super::*;
use crate::ai::{AiEntityHandle, Position, Substate};
use crate::ai_enemy::{
    AiMapVec, CombatFighterAccess, CombatPosition, combat, evaluate_combat_position_full,
};
use crate::coordinates::MapVec;
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};
use crate::profiles::ProfileRank;
use crate::weapons::WeaponDistance;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_support::actors::make_test_ai_soldier;

    fn add(engine: &mut EngineInner, x: f32, y: f32, z: f32, direction: i16) -> EntityId {
        let id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let entity = engine.world.entities.get_mut(id).unwrap();
        entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(x, y + z, z));
        entity.element_data_mut().set_direction_instantly(direction);
        entity.enemy_ai_mut().unwrap().soldier_profile_rank = ProfileRank::Soldier;
        id
    }

    #[test]
    fn neighbour_ranking_uses_literal_world_coordinates() {
        let mut engine = EngineInner::new();
        let owner = add(&mut engine, 1231.5779, 1845.2806, 0.0, 8);
        let first = add(&mut engine, 1159.4979, 1829.3608, 1.4621211, 7);
        let second = add(&mut engine, 1220.2673, 1886.527, 0.0, 8);
        engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("test allies"))
            .list_us = vec![owner.index(), first.index(), second.index()];
        assert_eq!(
            engine.live_combat_neighbour(&LevelAssets::new(), owner, None, true),
            None
        );
        assert_eq!(
            engine.live_combat_neighbour(&LevelAssets::new(), owner, None, false),
            Some(AiEntityHandle::new(second.index()))
        );
    }

    #[test]
    fn neighbour_distance_truncation_preserves_first_registered_tie() {
        let mut engine = EngineInner::new();
        let owner = add(&mut engine, 431.41672, 1755.0808, 4.6533546, 13);
        let first = add(&mut engine, 420.24728, 1758.1467, 4.186_21, 12);
        let second = add(&mut engine, 420.18027, 1757.8624, 4.382771, 12);
        engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("test allies"))
            .list_us = vec![owner.index(), first.index(), second.index()];
        assert_eq!(
            engine.live_combat_neighbour(&LevelAssets::new(), owner, None, true),
            Some(AiEntityHandle::new(first.index()))
        );
        assert_eq!(
            engine.live_combat_neighbour(&LevelAssets::new(), owner, None, false),
            None
        );
        engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("test reorder"))
            .list_us = vec![owner.index(), second.index(), first.index()];
        assert_eq!(
            engine.live_combat_neighbour(&LevelAssets::new(), owner, None, true),
            Some(AiEntityHandle::new(second.index()))
        );
    }
}

#[derive(Clone, Copy)]
pub(super) struct LiveCombatFighters<'a> {
    pub(super) engine: &'a EngineInner,
    pub(super) assets: &'a LevelAssets,
    pub(super) owner: EntityId,
}

impl<'a> LiveCombatFighters<'a> {
    pub(super) fn id(self, handle: u32) -> EntityId {
        self.engine
            .expect_human_id_for_ai_handle(handle, "live combat fighter")
    }
    fn entity(self, handle: u32) -> &'a Entity {
        self.engine
            .expect_entity(self.id(handle), "live combat fighter")
    }
    pub(super) fn range(self, handle: u32, range: WeaponDistance) -> u16 {
        self.assets
            .profile_manager
            .get_hth_weapon(self.hth_weapon_id(handle))
            .expect("live combat sword profile is missing")
            .distance[range as usize]
    }
    pub(super) fn principal(self, handle: u32) -> Option<AiEntityHandle> {
        self.entity(handle)
            .human_data()
            .expect("combat fighter must be human")
            .opponents
            .first()
            .map(|id| AiEntityHandle::new(id.index()))
    }
}

impl CombatFighterAccess for LiveCombatFighters<'_> {
    fn position(self, handle: u32) -> Position {
        self.engine.live_ai_position(self.id(handle))
    }
    fn protection_ground_position(self, handle: u32) -> crate::coordinates::GroundPoint {
        let position = self.entity(handle).element_data().position();
        crate::coordinates::GroundPoint::new(position.x, position.y)
    }
    fn elevation(self, handle: u32) -> f32 {
        self.entity(handle).element_data().position().z
    }
    fn direction(self, handle: u32) -> u16 {
        self.entity(handle).element_data().direction() as u16
    }
    fn hth_weapon_id(self, handle: u32) -> u32 {
        match self.entity(handle) {
            Entity::Soldier(soldier) => {
                soldier
                    .npc
                    .ai_brain
                    .enemy()
                    .expect("soldier brain")
                    .hth_weapon_id
            }
            Entity::Pc(pc) => {
                self.assets
                    .profile_manager
                    .get_character(pc.pc.profile_index)
                    .expect("combat character profile is missing")
                    .hth_weapon_id
            }
            _ => panic!("combat weapon owner is not a fighter"),
        }
    }
    fn sword_range_maximal(self, handle: u32) -> u16 {
        self.range(handle, WeaponDistance::Maximal)
    }
    fn fighting_ability(self, handle: u32) -> u16 {
        match self.entity(handle) {
            Entity::Soldier(soldier) => {
                self.engine
                    .soldier_profile_facts(self.assets, soldier, self.id(handle))
                    .1
            }
            Entity::Pc(pc) => {
                self.assets
                    .profile_manager
                    .get_character(pc.pc.profile_index)
                    .expect("combat character profile is missing")
                    .fighting
            }
            _ => panic!("combat ability owner is not a fighter"),
        }
    }
    fn rank(self, handle: u32) -> ProfileRank {
        self.entity(handle)
            .enemy_ai()
            .map(|ai| ai.soldier_profile_rank)
            .unwrap_or(ProfileRank::Soldier)
    }
    fn is_pc(self, handle: u32) -> bool {
        matches!(self.entity(handle), Entity::Pc(_))
    }
    fn is_friendly(self, handle: u32) -> bool {
        self.engine.camps_are_allied(
            self.engine
                .expect_entity(self.owner, "combat evaluator")
                .camp(),
            self.entity(handle).camp(),
        )
    }
}

impl EngineInner {
    pub(in crate::engine) fn propose_live_combat_position(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> CombatPosition {
        let fighters = LiveCombatFighters {
            engine: self,
            assets,
            owner,
        };
        let primary = fighters
            .principal(owner.index())
            .expect("combat proposal requires principal opponent");
        assert!(
            !fighters.is_friendly(primary.get()),
            "combat proposal primary is friendly"
        );
        let me = fighters.position(owner.index());
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("combat proposal primary"))
            .primary_target = Some(primary);
        let mut possible =
            vec![self.live_combat_position(assets, owner, owner.index(), me, Some(primary))];
        self.generate_live_combat_positions(assets, owner, &mut possible);

        let them = &self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("combat enemies"))
            .list_them;
        let mut enemies = Vec::with_capacity(them.len());
        for &handle in them {
            let fighters = LiveCombatFighters {
                engine: self,
                assets,
                owner,
            };
            let position = fighters.position(handle);
            let target = fighters.principal(handle);
            enemies.push(self.live_combat_position(assets, owner, handle, position, target));
        }
        for enemy in &enemies {
            self.ai
                .global
                .primary_target_multiplicity_scratch
                .insert(enemy.attacker.unwrap().get(), 0);
        }
        let us = &self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("combat allies"))
            .list_us;
        let mut friends = Vec::with_capacity(us.len());
        for &handle in us {
            if handle == owner.index() {
                continue;
            }
            let fighters = LiveCombatFighters {
                engine: self,
                assets,
                owner,
            };
            let entity = fighters.entity(handle);
            let (position, target) = if let Entity::Soldier(soldier) = entity {
                let ai = soldier.npc.ai_brain.enemy().expect("combat ally brain");
                if matches!(
                    ai.base.current_substate,
                    Substate::AttackingApproachingNewEnemy
                        | Substate::AttackingMovingAroundOldEnemy
                ) {
                    (fighters.position(handle), fighters.principal(handle))
                } else {
                    (ai.base.seek_position, ai.base.primary_target)
                }
            } else {
                (fighters.position(handle), fighters.principal(handle))
            };
            friends.push(self.live_combat_position(assets, owner, handle, position, target));
        }
        for friend in &friends {
            if let Some(target) = friend.target {
                let count = self
                    .ai
                    .global
                    .primary_target_multiplicity_scratch
                    .entry(target.get())
                    .or_insert(0);
                *count = u32::from((*count as u16).wrapping_add(1));
            }
        }
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("combat evaluator"));
        let iq = ai.iq_for_difficulty(
            self.control.sim_config.difficulty,
            self.is_hostile_to_player_camp(
                self.expect_entity(owner, "combat evaluator camp").camp(),
            ),
        );
        let mut best_index = 0;
        let mut best_score = -0x7fff_ffff;
        for (index, candidate) in possible.iter_mut().enumerate() {
            let score = evaluate_combat_position_full(
                owner.index(),
                &me,
                &ai.list_them,
                candidate,
                &mut friends,
                &mut enemies,
                LiveCombatFighters {
                    engine: self,
                    assets,
                    owner,
                },
                &assets.profile_manager,
                iq,
            );
            if score > best_score {
                best_score = score;
                best_index = index;
            }
        }
        let mut best = possible.swap_remove(best_index);
        if best.line_jump.is_none() && best.target.is_some() {
            best.line_jump = crate::engine::melee::table_swordfight_jump_line(
                &self.world.fast_grid,
                me.sector.map(i16::from).unwrap_or(-1),
                best.target_position.sector.map(i16::from).unwrap_or(-1),
                best.target_position.map_point(),
                LiveCombatFighters {
                    engine: self,
                    assets,
                    owner,
                }
                .sword_range_maximal(owner.index()) as f32,
            );
        }
        let ai = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("combat proposal neighbours"));
        let old_left = ai.left_combat_neighbour;
        let old_right = ai.right_combat_neighbour;
        ai.left_combat_neighbour = best.left_neighbour;
        ai.right_combat_neighbour = best.right_neighbour;
        ai.base.outbox.reentrant.cross_npc_actions.push(
            crate::ai::CrossNpcAction::UpdateLeftCombatNeighbour {
                target: owner.index(),
                old_left,
                new_left: best.left_neighbour,
            },
        );
        ai.base.outbox.reentrant.cross_npc_actions.push(
            crate::ai::CrossNpcAction::UpdateRightCombatNeighbour {
                target: owner.index(),
                old_right,
                new_right: best.right_neighbour,
            },
        );
        best
    }

    fn live_combat_position(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        attacker: u32,
        position: Position,
        target: Option<AiEntityHandle>,
    ) -> CombatPosition {
        let mut candidate = CombatPosition {
            attacker: Some(AiEntityHandle::new(attacker)),
            attacker_position: position,
            target,
            ..CombatPosition::default()
        };
        if let Some(target) = target {
            let fighters = LiveCombatFighters {
                engine: self,
                assets,
                owner,
            };
            candidate.target_position = fighters.position(target.get());
            candidate.target_direction = fighters.direction(target.get());
        }
        candidate
    }

    fn live_combat_neighbour(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        cached: Option<AiEntityHandle>,
        left: bool,
    ) -> Option<AiEntityHandle> {
        let fighters = LiveCombatFighters {
            engine: self,
            assets,
            owner,
        };
        let me = fighters.position(owner.index());
        let nose = MapVec::from_sector(fighters.direction(owner.index()));
        let eligible = |handle: u32| {
            if fighters.is_pc(handle) || fighters.rank(handle) != ProfileRank::Soldier {
                return false;
            }
            if nose.dot(MapVec::from_sector(fighters.direction(handle))) < 0.0 {
                return false;
            }
            let mut delta = fighters.position(handle).map_point() - me.map_point();
            delta.y *= INVERSE_ASPECT_RATIO;
            if left {
                nose.det(delta) < 0.0
            } else {
                nose.det(delta) > 0.0
            }
        };
        if let Some(cached) = cached
            && eligible(cached.get())
        {
            return Some(cached);
        }
        let mut best = None;
        let mut nearest = u32::MAX;
        let world_me = self
            .expect_entity(owner, "combat neighbour owner")
            .element_data()
            .position();
        for &handle in &self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("combat neighbours"))
            .list_us
        {
            if handle == owner.index() || !eligible(handle) {
                continue;
            }
            let other = fighters.entity(handle).element_data().position();
            let dx = other.x - world_me.x;
            let dy = (other.y - world_me.y) * INVERSE_ASPECT_RATIO;
            let dz = other.z - world_me.z;
            let distance =
                crate::ai_enemy::combat_neighbour_distance_ulong(dx * dx + dy * dy + dz * dz);
            if distance < nearest {
                nearest = distance;
                best = Some(AiEntityHandle::new(handle));
            }
        }
        best
    }

    fn generate_live_combat_positions(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        list: &mut Vec<CombatPosition>,
    ) {
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("combat generation"));
        if ai.base.blood_alcohol > 0 {
            return;
        }
        let formation = match self.expect_entity(owner, "combat formation owner") {
            Entity::Soldier(soldier) => {
                self.soldier_profile_facts(assets, soldier, owner)
                    .0
                    .formation
            }
            _ => false,
        };
        let mut surround = true;
        if ai.get_rank() == ProfileRank::Soldier && ai.base.list_us.len() > 2 && formation {
            let left = self.live_combat_neighbour(assets, owner, ai.left_combat_neighbour, true);
            let right = self.live_combat_neighbour(assets, owner, ai.right_combat_neighbour, false);
            if left.is_some() || right.is_some() {
                self.propose_live_combat_line(assets, owner, list, left, right);
                surround = false;
            }
        }
        if surround {
            for &enemy in &ai.list_them {
                self.propose_live_combat_ring(assets, owner, list, enemy);
            }
        }
        self.clean_live_combat_positions(assets, owner, list);
    }

    fn propose_live_combat_line(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        list: &mut Vec<CombatPosition>,
        left: Option<AiEntityHandle>,
        right: Option<AiEntityHandle>,
    ) {
        let fighters = LiveCombatFighters {
            engine: self,
            assets,
            owner,
        };
        let me = fighters.position(owner.index());
        let (there, direction) = match (left, right) {
            (Some(left), Some(right)) => {
                let left = fighters.position(left.get());
                let right = fighters.position(right.get());
                if left.sector != me.sector || right.sector != me.sector {
                    return;
                }
                let side = right.map_point() - left.map_point();
                (
                    Position {
                        x: left.x + side.x * 0.5,
                        y: left.y + side.y * 0.5,
                        ..left
                    },
                    side.normal_right(),
                )
            }
            (left, right) => {
                let friend = left.or(right).unwrap();
                let position = fighters.position(friend.get());
                if position.sector != me.sector {
                    return;
                }
                let mut nose = MapVec::from_sector(fighters.direction(friend.get()));
                let mut side = nose.normal_left();
                side.x *= combat::STANDARD_LINE_DISTANCE as f32;
                side.y *= combat::STANDARD_LINE_DISTANCE as f32;
                nose.y *= ASPECT_RATIO;
                side.y *= ASPECT_RATIO;
                let sign = if left.is_some() { 1.0 } else { -1.0 };
                (
                    Position {
                        x: position.x + sign * side.x,
                        y: position.y + sign * side.y,
                        ..position
                    },
                    nose,
                )
            }
        };
        let move_box = self
            .expect_entity(owner, "combat movement owner")
            .position_iface()
            .get_move_box();
        if !self.world.fast_grid.is_straight_movement_authorized(
            me.map_point(),
            there.map_point(),
            there.level,
            move_box,
        ) {
            return;
        }
        let range = fighters.range(owner.index(), WeaponDistance::Uber) as f32;
        for &enemy in &self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("combat line enemies"))
            .list_them
        {
            let position = fighters.position(enemy);
            let delta = position.map_point() - there.map_point();
            if delta.max_norm() >= range
                || delta.square_norm() >= range * range
                || delta.dot(direction) <= 0.0
            {
                continue;
            }
            let mut candidate = self.live_combat_position(
                assets,
                owner,
                owner.index(),
                there,
                Some(AiEntityHandle::new(enemy)),
            );
            candidate.change_position = (there.map_point() - me.map_point()).max_norm() > 3.0;
            candidate.line_position = true;
            candidate.left_neighbour = left;
            candidate.right_neighbour = right;
            candidate.bonus = combat::LINE_FORMATION_BONUS as i16;
            list.push(candidate);
        }
    }

    fn propose_live_combat_ring(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        list: &mut Vec<CombatPosition>,
        enemy: u32,
    ) {
        let fighters = LiveCombatFighters {
            engine: self,
            assets,
            owner,
        };
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("combat ring owner"));
        let me = fighters.position(owner.index());
        let position = fighters.position(enemy);
        let is_opponent = fighters
            .entity(enemy)
            .human_data()
            .unwrap()
            .opponents
            .contains(&owner);
        let mut forbidden = None;
        if is_opponent {
            if Some(AiEntityHandle::new(enemy)) != ai.base.primary_target {
                list.push(self.live_combat_position(
                    assets,
                    owner,
                    owner.index(),
                    me,
                    Some(AiEntityHandle::new(enemy)),
                ));
                return;
            }
            if ai.my_line_jump.is_some() {
                return;
            }
            forbidden = Some(fighters.direction(owner.index()));
        }
        let grid = &self.world.fast_grid;
        if ai.my_line_jump.is_some()
            && let Some(line_index) = crate::engine::melee::table_swordfight_jump_line(
                grid,
                me.sector.map(i16::from).unwrap_or(-1),
                position.sector.map(i16::from).unwrap_or(-1),
                position.map_point(),
                fighters.sword_range_maximal(owner.index()) as f32,
            )
        {
            let line = &grid.level.jump_lines[line_index as usize];
            let victim = &grid.level.jump_lines[line
                .associated_line_index
                .expect("table fight associated line")
                as usize];
            let offset = victim.compute_nearest_point_param(position.map_point()) * victim.norm();
            let vector = line.vector();
            let inverse = 1.0 / line.norm();
            let sector_index = line.sector_index.expect("table fight line sector");
            let sector = &grid.level.sectors[usize::from(sector_index)];
            let there = Position {
                x: line.point_b.x - offset * vector.x * inverse,
                y: line.point_b.y - offset * vector.y * inverse,
                level: line.layer,
                sector: crate::position_interface::SectorHandle::new(u16::from(
                    sector.sector_number,
                ))
                .map(|s| s.with_arena_index(sector_index)),
            };
            let mut candidate = self.live_combat_position(
                assets,
                owner,
                owner.index(),
                there,
                Some(AiEntityHandle::new(enemy)),
            );
            candidate.change_position = true;
            candidate.line_jump = Some(line_index);
            list.push(candidate);
            return;
        }
        let distance = fighters.range(owner.index(), WeaponDistance::Default) as f32;
        let move_box = self
            .expect_entity(owner, "combat ring mover")
            .position_iface()
            .get_move_box();
        for direction in 0..16 {
            if Some(direction) == forbidden {
                continue;
            }
            let mut offset = MapVec::from_sector(direction);
            offset.x *= distance;
            offset.y *= distance;
            offset.y *= ASPECT_RATIO;
            let there = Position {
                x: position.x - offset.x,
                y: position.y - offset.y,
                ..position
            };
            if !grid.is_straight_movement_authorized(
                me.map_point(),
                there.map_point(),
                there.level,
                move_box,
            ) {
                continue;
            }
            let mut candidate = self.live_combat_position(
                assets,
                owner,
                owner.index(),
                there,
                Some(AiEntityHandle::new(enemy)),
            );
            candidate.change_position = true;
            list.push(candidate);
        }
    }

    fn clean_live_combat_positions(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        list: &mut Vec<CombatPosition>,
    ) {
        let fighters = LiveCombatFighters {
            engine: self,
            assets,
            owner,
        };
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("combat cleanup"));
        let principal = fighters
            .principal(owner.index())
            .expect("combat cleanup principal");
        let lock = fighters
            .entity(principal.get())
            .human_data()
            .unwrap()
            .opponents
            .len()
            <= 1;
        let me = fighters.position(owner.index());
        let mut index = 0;
        while index < list.len() {
            let candidate = &mut list[index];
            candidate.change_adversary = candidate.target != ai.base.primary_target;
            let target = candidate.target.expect("combat candidate target");
            let mut reject = (lock && target != principal)
                || (candidate.attacker_position.map_point() - me.map_point()).square_norm()
                    > combat::SQR_MAX_NEW_POS_DIST as f32
                || (candidate.change_adversary
                    && !self.sleeping_enemy_attack_allowed(owner, fighters.id(target.get())));
            if !reject {
                for &enemy in &ai.list_them {
                    let distance = (fighters.position(enemy).map_point()
                        - candidate.attacker_position.map_point())
                    .max_norm() as u16;
                    if distance < combat::MIN_ENEMY_DIST as u16 {
                        reject = true;
                        break;
                    }
                    if target.get() != enemy && distance < fighters.sword_range_maximal(enemy) {
                        candidate.bonus = candidate
                            .bonus
                            .wrapping_sub(combat::ENEMY_NEAR_MALUS as i16);
                    }
                }
                if !reject && candidate.line_jump.is_none() {
                    reject = ai.base.list_us.iter().any(|&friend| {
                        friend != owner.index()
                            && (fighters.position(friend).map_point()
                                - candidate.attacker_position.map_point())
                            .max_norm()
                                < combat::MIN_FRIEND_DIST as f32
                    });
                }
            }
            if reject {
                if index == 0 {
                    candidate.bonus = candidate
                        .bonus
                        .wrapping_sub(combat::BAD_POSITION_MALUS as i16);
                } else {
                    list.remove(index);
                    continue;
                }
            }
            index += 1;
        }
    }
}
