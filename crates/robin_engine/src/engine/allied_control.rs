//! Direct-control orders for Royalist soldier NPCs.

use crate::allied_control::{
    AlliedDuty, AlliedFormation, AlliedPinnedGroup, AlliedSoldierOrder, AlliedStance,
};
use crate::coordinates::MapPoint;
use crate::element::{Camp, Entity, EntityId};
use crate::profiles::ProfileRank;

use super::movement::{CIRCULAR_DISPATCH_RADIUS, GROUP_LIMIT_MAX};
use super::{EngineInner, LevelAssets};

const ARRIVAL_DISTANCE: f32 = 32.0;
const FOLLOW_DISTANCE: f32 = 85.0;
const FOLLOW_REPATH_DISTANCE: f32 = 24.0;
const FORMATION_SPACING: f32 = 22.0;
const STAGGERED_SPACING: f32 = 31.0;
const LINE_TO_COLUMN_DISTANCE: f32 = 240.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FormationRole {
    Officer,
    Knight,
    Shield,
    Melee,
    Ranged,
}

fn distance_squared(a: MapPoint, b: MapPoint) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

fn allied_order_locks_ai(
    order: &AlliedSoldierOrder,
    threatened: bool,
    reached_hold_anchor: bool,
) -> bool {
    match order.stance {
        AlliedStance::Hold => true,
        AlliedStance::Defensive => !threatened,
        // Aggressive soldiers remain under direct movement control while
        // carrying out an explicit move/patrol/follow duty. Their normal AI
        // takes over when a threat is detected, then the duty resumes
        // afterwards. A completed move releases the soldier so the default
        // aggressive stance can resume autonomous combat without restoring
        // an authored mission patrol before the player destination is reached.
        AlliedStance::Aggressive => !threatened && !reached_hold_anchor,
    }
}

fn allied_stance_allows_normal_strikes(stance: AlliedStance) -> bool {
    !matches!(stance, AlliedStance::Hold)
}

fn allied_stance_allows_combat_movement(stance: AlliedStance) -> bool {
    matches!(stance, AlliedStance::Aggressive)
}

fn slot_preference(role: FormationRole, formation: AlliedFormation, point: MapPoint) -> [f32; 3] {
    let radius_squared = point.x * point.x + point.y * point.y;
    match (role, formation) {
        (FormationRole::Officer, AlliedFormation::Box) => [radius_squared, -point.y, point.x.abs()],
        (FormationRole::Officer, _) => [-point.y, point.x.abs(), point.x],
        (FormationRole::Knight, _) => [radius_squared, -point.y, point.x.abs()],
        (FormationRole::Shield | FormationRole::Melee, AlliedFormation::Box) => {
            [-radius_squared, -point.y, point.x.abs()]
        }
        (FormationRole::Shield | FormationRole::Melee, _) => [-point.y, point.x.abs(), point.x],
        (FormationRole::Ranged, AlliedFormation::Box) => [radius_squared, point.y, point.x.abs()],
        (FormationRole::Ranged, _) => [point.y, point.x.abs(), point.x],
    }
}

fn compare_slot_preferences(a: [f32; 3], b: [f32; 3]) -> std::cmp::Ordering {
    a[0].total_cmp(&b[0])
        .then_with(|| a[1].total_cmp(&b[1]))
        .then_with(|| a[2].total_cmp(&b[2]))
}

fn assign_formation_offsets(
    roles: &[FormationRole],
    slots: Vec<MapPoint>,
    formation: AlliedFormation,
) -> Vec<MapPoint> {
    assert_eq!(
        roles.len(),
        slots.len(),
        "formation role and slot counts must match"
    );
    let mut members: Vec<_> = roles.iter().copied().enumerate().collect();
    members.sort_by_key(|&(original_index, role)| (role, original_index));

    let mut remaining = slots;
    let mut assigned = vec![None; roles.len()];
    for (original_index, role) in members {
        let best = remaining
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                compare_slot_preferences(
                    slot_preference(role, formation, **a),
                    slot_preference(role, formation, **b),
                )
            })
            .map(|(index, _)| index)
            .unwrap_or_else(|| panic!("formation ran out of slots for member {original_index}"));
        assigned[original_index] = Some(remaining.remove(best));
    }
    assigned
        .into_iter()
        .enumerate()
        .map(|(index, point)| {
            point.unwrap_or_else(|| panic!("formation did not assign member {index}"))
        })
        .collect()
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
        let offsets = flank_offsets(8, false);
        assert!(
            offsets
                .iter()
                .all(|point| point.x.abs() >= FORMATION_SPACING)
        );
        assert!(offsets.iter().any(|point| point.x < 0.0));
        assert!(offsets.iter().any(|point| point.x > 0.0));
    }

    #[test]
    fn officer_takes_the_center_front_slot() {
        let roles = [
            FormationRole::Melee,
            FormationRole::Melee,
            FormationRole::Melee,
            FormationRole::Melee,
            FormationRole::Melee,
            FormationRole::Officer,
        ];
        let offsets = assign_formation_offsets(
            &roles,
            row_offsets(roles.len(), 4, FORMATION_SPACING, false),
            AlliedFormation::Line,
        );
        let officer = offsets[5];
        assert_eq!(officer.y, 0.0);
        assert_eq!(officer.x.abs(), FORMATION_SPACING * 0.5);
        assert!(offsets[..5].iter().any(|point| point.y < officer.y));
    }

    #[test]
    fn ranged_troops_fill_the_rear_rank() {
        let roles = [
            FormationRole::Ranged,
            FormationRole::Melee,
            FormationRole::Shield,
            FormationRole::Melee,
        ];
        let offsets = assign_formation_offsets(
            &roles,
            row_offsets(roles.len(), 3, FORMATION_SPACING, false),
            AlliedFormation::Line,
        );
        assert!(offsets[0].y < offsets[1].y);
        assert!(offsets[0].y < offsets[2].y);
    }

    #[test]
    fn flank_reserves_its_center_for_an_officer() {
        let roles = [
            FormationRole::Melee,
            FormationRole::Melee,
            FormationRole::Officer,
            FormationRole::Melee,
            FormationRole::Melee,
        ];
        let offsets = assign_formation_offsets(
            &roles,
            flank_offsets(roles.len(), true),
            AlliedFormation::Flank,
        );
        assert_eq!(offsets[2], MapPoint::new(0.0, 0.0));
        assert!(offsets[..2].iter().all(|point| point.x != 0.0));
    }

    #[test]
    fn box_protects_crossbows_inside_melee_troops() {
        let roles = [
            FormationRole::Officer,
            FormationRole::Melee,
            FormationRole::Melee,
            FormationRole::Melee,
            FormationRole::Melee,
            FormationRole::Knight,
            FormationRole::Ranged,
            FormationRole::Ranged,
            FormationRole::Ranged,
        ];
        let offsets = assign_formation_offsets(&roles, box_offsets(&roles), AlliedFormation::Box);
        let minimum_melee_radius = offsets[1..5]
            .iter()
            .map(|point| point.x * point.x + point.y * point.y)
            .fold(f32::MAX, f32::min);

        for ranged in &offsets[6..9] {
            let radius = ranged.x * ranged.x + ranged.y * ranged.y;
            assert!(radius < minimum_melee_radius);
        }
    }

    #[test]
    fn selected_heroes_reserve_the_command_center() {
        let anchor = formation_anchor_behind_leaders(
            MapPoint::new(0.0, 100.0),
            MapPoint::new(0.0, 0.0),
            &[MapPoint::new(-20.0, 0.0), MapPoint::new(20.0, 0.0)],
        );
        assert_eq!(anchor.x, 0.0);
        assert_eq!(anchor.y, FORMATION_SPACING * 1.5);

        let deep_heroes = formation_anchor_behind_leaders(
            MapPoint::new(0.0, 100.0),
            MapPoint::new(0.0, 0.0),
            &[MapPoint::new(0.0, -20.0), MapPoint::new(0.0, 20.0)],
        );
        assert_eq!(deep_heroes.y, 20.0 + FORMATION_SPACING * 1.5);
    }

    #[test]
    fn aggressive_patrol_group_stays_directed_until_threatened() {
        let patrol_order = |x| AlliedSoldierOrder {
            stance: AlliedStance::Aggressive,
            formation: AlliedFormation::Line,
            duty: AlliedDuty::Patrol {
                points: [MapPoint::new(x, 0.0), MapPoint::new(x, 100.0)],
                next: 1,
            },
            last_destination: MapPoint::new(x, 100.0),
            path_fallback: None,
            deploy_destination: None,
        };
        let group = [patrol_order(-11.0), patrol_order(11.0)];

        assert!(
            group
                .iter()
                .all(|order| allied_order_locks_ai(order, false, false))
        );
        assert!(
            group
                .iter()
                .all(|order| !allied_order_locks_ai(order, true, false))
        );

        let idle_aggressive = AlliedSoldierOrder {
            duty: AlliedDuty::Hold {
                anchor: MapPoint::new(0.0, 0.0),
            },
            ..patrol_order(0.0)
        };
        assert!(!allied_order_locks_ai(&idle_aggressive, false, true));
        assert!(
            allied_order_locks_ai(&idle_aggressive, false, false),
            "an explicit move must continue suppressing the mission patrol until arrival"
        );
    }

    #[test]
    fn stances_separate_striking_from_pursuit() {
        assert!(!allied_stance_allows_normal_strikes(AlliedStance::Hold));
        assert!(!allied_stance_allows_combat_movement(AlliedStance::Hold));

        assert!(allied_stance_allows_normal_strikes(AlliedStance::Defensive));
        assert!(!allied_stance_allows_combat_movement(
            AlliedStance::Defensive
        ));

        assert!(allied_stance_allows_normal_strikes(
            AlliedStance::Aggressive
        ));
        assert!(allied_stance_allows_combat_movement(
            AlliedStance::Aggressive
        ));
    }

    #[test]
    fn direct_control_lock_discards_a_pending_bored_order() {
        let mut engine = EngineInner::new();
        let mut npc = crate::element::NpcData {
            life_points: 100,
            ai: crate::element::AiActorData {
                ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                ..Default::default()
            },
            ..Default::default()
        };
        npc.ai_brain
            .base_mut()
            .expect("test soldier has enemy AI")
            .outbox
            .actor
            .orders
            .push(crate::order::AiOrderIntent::new(
                crate::order::OrderType::WaitingUprightBored,
                0.0,
                0.0,
            ));
        let soldier = engine.add_entity(Entity::Soldier(crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                active: true,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc,
            soldier: crate::element::SoldierData {
                cached_camp: Camp::Royalists,
                ..Default::default()
            },
        }));

        engine.set_allied_ai_locked(soldier, true);

        let ai = engine
            .get_entity(soldier)
            .and_then(Entity::ai_controller)
            .expect("controlled test soldier retains AI");
        assert!(ai.script_locked);
        assert!(ai.remember_events);
        assert!(ai.outbox.actor.orders.is_empty());
        assert!(
            !ai.outbox.actor.halt,
            "direct-control lock must not queue a halt behind the new movement"
        );
    }

    #[test]
    fn completed_allied_move_replaces_the_ai_guard_post() {
        let mut engine = EngineInner::new();
        let mut element = crate::element::ElementData {
            kind: crate::element::ElementKind::ActorSoldier,
            active: true,
            ..Default::default()
        };
        let position = MapPoint::new(321.0, 654.0);
        let sector = crate::position_interface::SectorHandle::new(7);
        element.sprite.apply_placement(
            position,
            3,
            sector,
            5,
            crate::element::GameMaterial::Ground,
            None,
            None,
        );
        let soldier = engine.add_entity(Entity::Soldier(crate::element::ActorSoldier {
            element,
            actor: Default::default(),
            human: Default::default(),
            npc: crate::element::NpcData {
                life_points: 100,
                ai: crate::element::AiActorData {
                    ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                    ..Default::default()
                },
                ..Default::default()
            },
            soldier: crate::element::SoldierData {
                cached_camp: Camp::Royalists,
                ..Default::default()
            },
        }));

        engine.store_allied_current_position_as_post(soldier);

        let entity = engine.get_entity(soldier).unwrap();
        let Entity::Soldier(soldier) = entity else {
            unreachable!();
        };
        assert_eq!(soldier.npc.initial_position_x, position.x);
        assert_eq!(soldier.npc.initial_position_y, position.y);
        assert_eq!(soldier.npc.initial_position_sector, sector);
        assert_eq!(soldier.npc.initial_position_level, 3);
        let ai = soldier.npc.ai_brain.base().unwrap();
        assert_eq!(ai.initial_position.x, position.x);
        assert_eq!(ai.initial_position.y, position.y);
        assert_eq!(ai.initial_position.sector, sector);
        assert_eq!(ai.initial_position.level, 3);
    }

    #[test]
    fn selected_player_strike_discards_preexisting_ai_combat_work() {
        let mut npc = crate::element::NpcData {
            life_points: 100,
            ai: crate::element::AiActorData {
                ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                ..Default::default()
            },
            ..Default::default()
        };
        let ai = npc.ai_brain.enemy_mut().expect("test soldier has enemy AI");
        ai.pending_sword_strike_consideration = true;
        ai.base
            .outbox
            .actor
            .orders
            .push(crate::order::AiOrderIntent::new(
                crate::order::OrderType::RunningUpright,
                100.0,
                100.0,
            ));

        let mut engine = EngineInner::new();
        let soldier = engine.add_entity(Entity::Soldier(crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                active: true,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc,
            soldier: crate::element::SoldierData {
                cached_camp: Camp::Royalists,
                ..Default::default()
            },
        }));
        engine.players.allied.seats[0].selection.push(soldier);

        engine.prepare_allied_player_combat_command(soldier);

        let ai = engine
            .get_entity(soldier)
            .and_then(Entity::enemy_ai)
            .expect("test soldier retains enemy AI");
        assert!(!ai.pending_sword_strike_consideration);
        assert!(ai.base.outbox.actor.orders.is_empty());
    }

    #[test]
    fn eligible_uncontrolled_ally_uses_the_original_path_failure_flow() {
        let mut engine = EngineInner::new();
        let soldier = engine.add_entity(Entity::Soldier(crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                active: true,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: crate::element::NpcData {
                life_points: 100,
                ..Default::default()
            },
            soldier: crate::element::SoldierData {
                cached_camp: Camp::Royalists,
                ..Default::default()
            },
        }));

        assert!(engine.is_controllable_allied_soldier(soldier));
        assert_eq!(engine.allied_path_failure_fallback(soldier), None);
    }

    #[test]
    fn explicit_allied_order_replaces_authored_patrol() {
        let paths = vec![crate::level_data::RawHikingPath {
            waypoints: vec![crate::level_data::RawWaypoint {
                x: 10,
                y: 20,
                sector: 0,
                level: 0,
                command: crate::level_data::WaypointCommand::None,
            }],
        }];
        let path_id = crate::ai::PathId::new(0).unwrap();
        let mut npc = crate::element::NpcData {
            life_points: 100,
            ai: crate::element::AiActorData {
                ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                ..Default::default()
            },
            ..Default::default()
        };
        let ai = npc.ai_brain.base_mut().expect("test soldier has enemy AI");
        ai.has_patrol_path = true;
        ai.patrol_path = crate::ai::PatrolPath::new(path_id, &paths);

        let mut engine = EngineInner::new();
        let soldier = engine.add_entity(Entity::Soldier(crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                active: true,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc,
            soldier: crate::element::SoldierData {
                cached_camp: Camp::Royalists,
                ..Default::default()
            },
        }));

        engine.replace_authored_allied_patrol(soldier);

        let ai = engine
            .get_entity(soldier)
            .and_then(Entity::ai_controller)
            .expect("controlled test soldier retains AI");
        assert!(!ai.has_patrol_path);
        assert!(ai.patrol_path.is_none());
        assert!(ai.detached_patrol_path_status.hiking_path_index.is_none());
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

fn centered_grid_offsets(count: usize, columns: usize, spacing: f32) -> Vec<MapPoint> {
    if count == 0 {
        return Vec::new();
    }
    let rows = count.div_ceil(columns);
    (0..count)
        .map(|index| {
            let row = index / columns;
            let members_in_row = (count - row * columns).min(columns);
            let column = index % columns;
            MapPoint::new(
                (column as f32 - (members_in_row.saturating_sub(1) as f32 / 2.0)) * spacing,
                (rows.saturating_sub(1) as f32 / 2.0 - row as f32) * spacing,
            )
        })
        .collect()
}

fn box_offsets(roles: &[FormationRole]) -> Vec<MapPoint> {
    let outer_count = roles
        .iter()
        .filter(|role| matches!(role, FormationRole::Shield | FormationRole::Melee))
        .count();
    let inner_count = roles.len() - outer_count;
    let inner_columns = (inner_count as f32).sqrt().ceil().max(1.0) as usize;
    let mut offsets = centered_grid_offsets(inner_count, inner_columns, FORMATION_SPACING);
    if outer_count == 0 {
        return offsets;
    }

    let inner_half_width = offsets
        .iter()
        .map(|point| point.x.abs())
        .fold(0.0, f32::max);
    let inner_half_depth = offsets
        .iter()
        .map(|point| point.y.abs())
        .fold(0.0, f32::max);
    let half_width = inner_half_width + FORMATION_SPACING;
    let half_depth = inner_half_depth + FORMATION_SPACING;

    if outer_count == 1 {
        offsets.push(MapPoint::new(0.0, half_depth));
        return offsets;
    }
    for index in 0..outer_count {
        let angle =
            std::f32::consts::FRAC_PI_4 + std::f32::consts::TAU * index as f32 / outer_count as f32;
        let direction = MapPoint::new(angle.cos(), angle.sin());
        let scale = (half_width / direction.x.abs()).min(half_depth / direction.y.abs());
        offsets.push(MapPoint::new(direction.x * scale, direction.y * scale));
    }
    offsets
}

fn flank_offsets(count: usize, reserve_command_center: bool) -> Vec<MapPoint> {
    let wing_count = count.saturating_sub(usize::from(reserve_command_center && count > 0));
    let left_count = wing_count.div_ceil(2);
    let right_count = wing_count / 2;
    let wing = |index: usize, members: usize, side: f32| {
        let row = index / 2;
        let members_in_row = (members - row * 2).min(2);
        let column = index % 2;
        let from_center = FORMATION_SPACING * 1.2
            + (column as f32 + (2 - members_in_row) as f32 * 0.5) * FORMATION_SPACING;
        MapPoint::new(side * from_center, -(row as f32) * FORMATION_SPACING)
    };
    let mut offsets = Vec::with_capacity(count);
    if reserve_command_center && count > 0 {
        offsets.push(MapPoint::new(0.0, 0.0));
    }
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

fn formation_anchor_behind_leaders(
    soldier_centroid: MapPoint,
    destination: MapPoint,
    leader_positions: &[MapPoint],
) -> MapPoint {
    if leader_positions.is_empty() {
        return destination;
    }
    let forward = normalized_direction(soldier_centroid, destination);
    let count = leader_positions.len() as f32;
    let leader_centroid = MapPoint::new(
        leader_positions.iter().map(|point| point.x).sum::<f32>() / count,
        leader_positions.iter().map(|point| point.y).sum::<f32>() / count,
    );
    let max_radius_squared = leader_positions
        .iter()
        .map(|point| distance_squared(*point, leader_centroid))
        .fold(0.0, f32::max);
    let rear_extent = if max_radius_squared <= GROUP_LIMIT_MAX * GROUP_LIMIT_MAX {
        leader_positions
            .iter()
            .map(|point| {
                let dx = point.x - leader_centroid.x;
                let dy = point.y - leader_centroid.y;
                -(dx * forward.x + dy * forward.y)
            })
            .fold(0.0, f32::max)
    } else {
        // The native hero group-move fallback replaces a scattered group's
        // relative footprint with this fixed-radius circle.
        CIRCULAR_DISPATCH_RADIUS
    };
    let setback = rear_extent + FORMATION_SPACING * 1.5;
    MapPoint::new(
        destination.x - forward.x * setback,
        destination.y - forward.y * setback,
    )
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

    /// Whether stance policy permits a soldier's AI to launch an ordinary
    /// damaging sword strike. Smalltalk and reactive parries do not use this
    /// path and deliberately remain available in every stance.
    pub(super) fn allied_allows_normal_strikes(&self, soldier: EntityId) -> bool {
        self.allied_order(soldier)
            .is_none_or(|order| allied_stance_allows_normal_strikes(order.stance))
    }

    /// Whether stance policy permits AI-authored combat movement. Direct
    /// player movement and patrol/follow maintenance bypass the AI intent
    /// queue, so Defensive can reject pursuit without freezing its duty.
    pub(super) fn allied_allows_combat_movement(&self, soldier: EntityId) -> bool {
        self.allied_order(soldier)
            .is_none_or(|order| allied_stance_allows_combat_movement(order.stance))
    }

    /// True while a controllable soldier is selected by any player seat.
    pub(super) fn allied_soldier_is_selected(&self, soldier: EntityId) -> bool {
        self.players
            .allied
            .seats
            .iter()
            .any(|seat| seat.selection.contains(&soldier))
    }

    /// Give an explicit player combat command precedence over work the
    /// soldier AI planned before the gesture was received.
    pub(super) fn prepare_allied_player_combat_command(&mut self, soldier: EntityId) {
        if !self.allied_soldier_is_selected(soldier) {
            return;
        }
        self.stop_owner(soldier, crate::sequence::SequencePriority::Preference);
        let ai = self
            .get_entity_mut(soldier)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| panic!("selected allied soldier {soldier:?} has no enemy AI"));
        ai.pending_sword_strike_consideration = false;
        ai.base.outbox.actor.orders.clear();
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
                // Soldier sprites are anchored at their feet and extend
                // primarily up and left. Intersecting the whole sprite made
                // the top/left sides of a drag box select units far outside
                // it, while the bottom/right sides appeared accurate. The
                // ground point is also the center of the persistent selection
                // circle, so it is the stable selection coordinate.
                selection_box.contains_point(crate::coordinates::ScreenPoint::new(pos.x, pos.y))
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
        let is_unconscious = entity
            .human_data()
            .expect("controlled allied soldier has no human data")
            .unconscious;
        let Some(ai) = entity.ai_controller_mut() else {
            panic!("controlled allied soldier {id:?} has no AI controller");
        };
        if locked {
            if !ai.script_locked {
                // Direct-control movement is about to launch immediately.
                // Use the real script-lock cleanup so an already-queued bored
                // animation or return-to-duty order cannot dispatch after and
                // replace that movement, but suppress script_lock's halt: a
                // deferred halt would cancel the new player order instead.
                ai.script_lock(true, true);
            } else {
                ai.remember_events = true;
            }
        } else if ai.script_locked {
            ai.script_unlock(is_unconscious);
        } else {
            ai.remember_events = false;
        }
    }

    fn replace_authored_allied_patrol(&mut self, id: EntityId) {
        let ai = self
            .get_entity_mut(id)
            .unwrap_or_else(|| panic!("controlled allied soldier {id:?} disappeared"))
            .ai_controller_mut()
            .unwrap_or_else(|| panic!("controlled allied soldier {id:?} has no AI controller"));
        ai.has_patrol_path = false;
        ai.detach_patrol_path(None, false);
    }

    /// Make the soldier's reached direct-control destination its new guard
    /// post before autonomous AI resumes. Original `ReturnToDuty` walks to
    /// `RHElementActorNPC::mposInitialPosition` (`RHartificialintelligence.cpp`
    /// 2206-2238), so retaining the mission-start snapshot here would make an
    /// aggressive ally undo every completed player move.
    fn store_allied_current_position_as_post(&mut self, id: EntityId) {
        let entity = self
            .get_entity_mut(id)
            .unwrap_or_else(|| panic!("controlled allied soldier {id:?} disappeared"));
        let Entity::Soldier(soldier) = entity else {
            panic!("controlled allied soldier {id:?} changed entity kind");
        };
        let position = soldier.element.position_map();
        let sector = soldier.element.sector();
        let level = soldier.element.layer();
        let direction = soldier
            .element
            .sprite
            .position_iface
            .get_direction()
            .as_u8() as i16;
        let direction_vector = crate::shadow_polygon::sector_to_direction(direction);

        soldier.npc.initial_position_x = position.x;
        soldier.npc.initial_position_y = position.y;
        soldier.npc.initial_position_sector = sector;
        soldier.npc.initial_position_level = level;
        soldier.npc.initial_view_direction.x = direction_vector[0];
        soldier.npc.initial_view_direction.y = direction_vector[1];

        let ai = soldier
            .npc
            .ai_brain
            .base_mut()
            .expect("controlled allied soldier has no AI controller");
        ai.initial_position = crate::ai::Position {
            x: position.x,
            y: position.y,
            sector,
            level,
        };
        ai.initial_view_direction =
            u16::try_from(crate::position_interface::vector_to_sector_0_to_15(
                direction_vector[0] * crate::position_interface::ASPECT_RATIO,
                direction_vector[1],
            ))
            .expect("0-to-15 direction must fit u16");
    }

    fn allied_formation_slots(
        &self,
        assets: &LevelAssets,
        soldiers: &[EntityId],
        leaders: &[EntityId],
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
        let leader_positions: Vec<_> = leaders
            .iter()
            .map(|id| {
                let entity = self
                    .get_entity(*id)
                    .unwrap_or_else(|| panic!("selected formation leader {id:?} disappeared"));
                if entity.pc_data().is_none() {
                    panic!("selected formation leader {id:?} is not a PC");
                }
                entity.element_data().position_map()
            })
            .collect();
        let formation_anchor =
            formation_anchor_behind_leaders(centroid, destination, &leader_positions);
        let forward = normalized_direction(centroid, destination);

        let members: Vec<_> = soldiers
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
                let role = match profile.rank {
                    ProfileRank::Officer => FormationRole::Officer,
                    ProfileRank::Knight => FormationRole::Knight,
                    ProfileRank::Soldier | ProfileRank::None => {
                        if weapon.shield || profile.formation {
                            FormationRole::Shield
                        } else if profile.shooting > 0 && profile.shooting_weapon_id != 0 {
                            FormationRole::Ranged
                        } else {
                            FormationRole::Melee
                        }
                    }
                };
                (role, original_index, id)
            })
            .collect();
        let roles: Vec<_> = members.iter().map(|&(role, _, _)| role).collect();
        let has_officer = roles.contains(&FormationRole::Officer);

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
            AlliedFormation::Box => box_offsets(&roles),
            AlliedFormation::Staggered => {
                let columns = ((soldiers.len() as f32 * 2.0).sqrt().ceil() as usize).max(1);
                row_offsets(soldiers.len(), columns, STAGGERED_SPACING, true)
            }
            AlliedFormation::Flank => flank_offsets(soldiers.len(), has_officer),
        };

        let offsets = assign_formation_offsets(&roles, local_slots, formation);
        members
            .iter()
            .map(|&(_, _, id)| id)
            .zip(offsets)
            .map(|(id, offset)| {
                let offset = rotate_formation_offset(offset, forward);
                (
                    id,
                    MapPoint::new(formation_anchor.x + offset.x, formation_anchor.y + offset.y),
                )
            })
            .collect()
    }

    fn move_allied_to_slots(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        soldiers: &[EntityId],
        leaders: &[EntityId],
        destination: MapPoint,
        running: bool,
        formation: AlliedFormation,
    ) -> Vec<(EntityId, MapPoint)> {
        let valid: Vec<_> = soldiers
            .iter()
            .copied()
            .filter(|id| self.is_controllable_allied_soldier(*id))
            .collect();
        let slots =
            self.allied_formation_slots(assets, &valid, leaders, destination, formation, true);
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
        leaders: &[EntityId],
        destination: MapPoint,
        running: bool,
        formation: AlliedFormation,
    ) {
        let valid: Vec<_> = soldiers
            .iter()
            .copied()
            .filter(|id| self.is_controllable_allied_soldier(*id))
            .collect();
        if valid.is_empty() {
            return;
        }
        for &id in &valid {
            self.replace_authored_allied_patrol(id);
        }
        let deployed_slots =
            self.allied_formation_slots(assets, &valid, leaders, destination, formation, false);
        let deployed_by_id: std::collections::BTreeMap<_, _> = deployed_slots.into_iter().collect();
        let leader_positions: Vec<_> = leaders
            .iter()
            .map(|id| {
                let entity = self
                    .get_entity(*id)
                    .unwrap_or_else(|| panic!("selected formation leader {id:?} disappeared"));
                if entity.pc_data().is_none() {
                    panic!("selected formation leader {id:?} is not a PC");
                }
                entity.element_data().position_map()
            })
            .collect();
        let count = valid.len() as f32;
        let centroid = MapPoint::new(
            valid
                .iter()
                .map(|id| {
                    self.get_entity(*id)
                        .unwrap_or_else(|| panic!("forming allied soldier {id:?} disappeared"))
                        .element_data()
                        .position_map()
                        .x
                })
                .sum::<f32>()
                / count,
            valid
                .iter()
                .map(|id| {
                    self.get_entity(*id)
                        .unwrap_or_else(|| panic!("forming allied soldier {id:?} disappeared"))
                        .element_data()
                        .position_map()
                        .y
                })
                .sum::<f32>()
                / count,
        );
        let formation_anchor =
            formation_anchor_behind_leaders(centroid, destination, &leader_positions);
        for (id, slot) in self.move_allied_to_slots(
            sim,
            assets,
            &valid,
            leaders,
            destination,
            running,
            formation,
        ) {
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
                    path_fallback: (distance_squared(slot, formation_anchor) > 1.0)
                        .then_some(formation_anchor),
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
                // Preference is the priority used by ordinary sword strikes.
                // Stopping only at Normal left an already-planned AI strike
                // alive after switching to Hold.
                self.stop_owner(id, crate::sequence::SequencePriority::Preference);
                if let Some(ai) = self.get_entity_mut(id).and_then(Entity::enemy_ai_mut) {
                    ai.pending_sword_strike_consideration = false;
                }
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
        for &id in soldiers {
            if self.is_controllable_allied_soldier(id) {
                self.replace_authored_allied_patrol(id);
            }
        }
        let slots =
            self.move_allied_to_slots(sim, assets, soldiers, &[], destination, false, formation);
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
        let slots = self.allied_formation_slots(
            assets,
            &valid,
            std::slice::from_ref(&hero),
            hero_position,
            formation,
            false,
        );
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

    /// Consume the one-shot center fallback for a soldier with an active
    /// direct-control order whose formation slot has no path. Returning
    /// `Some(None)` identifies an ordered ally with no fallback, which must
    /// still fail immediately instead of retaining the original 100-frame
    /// failed-path wait (`original-code/RHengine.cpp:8481-8503`).
    pub(super) fn allied_path_failure_fallback(
        &mut self,
        id: EntityId,
    ) -> Option<Option<MapPoint>> {
        // Eligibility for allied selection is deliberately broader than being
        // under direct control. Ordinary Royalist AI must keep the original
        // failed-request behavior unless the player actually issued an order.
        let order = self.players.allied.orders.get_mut(&id)?;
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
            let was_ai_locked = self
                .get_entity(id)
                .and_then(Entity::ai_controller)
                .expect("controlled allied soldier has no AI controller")
                .script_locked;
            let reached_hold_anchor = match &order.duty {
                AlliedDuty::Hold { anchor } => {
                    distance_squared(position, *anchor) <= ARRIVAL_DISTANCE * ARRIVAL_DISTANCE
                }
                AlliedDuty::Patrol { .. } | AlliedDuty::Follow { .. } => false,
            };
            let ai_locked = allied_order_locks_ai(&order, threatened, reached_hold_anchor);
            if was_ai_locked && !ai_locked && reached_hold_anchor {
                self.store_allied_current_position_as_post(id);
            }
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
                self.perform_group_move(
                    sim,
                    assets,
                    &[id],
                    destination,
                    false,
                    true,
                    None,
                    None,
                    None,
                    &[],
                    &[],
                );
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
                    } else if ai_locked && !was_ai_locked {
                        // Combat may have interrupted the route and moved the
                        // soldier away from it. Re-issue the current patrol
                        // leg when direct control resumes.
                        Some(target)
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
                    ((ai_locked && !was_ai_locked)
                        || (distance_squared(position, target) > FOLLOW_DISTANCE * FOLLOW_DISTANCE
                            && distance_squared(order.last_destination, target)
                                > FOLLOW_REPATH_DISTANCE * FOLLOW_REPATH_DISTANCE))
                        .then_some(target)
                }
            };
            if let Some(destination) = destination {
                self.perform_group_move(
                    sim,
                    assets,
                    &[id],
                    destination,
                    false,
                    true,
                    None,
                    None,
                    None,
                    &[],
                    &[],
                );
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
