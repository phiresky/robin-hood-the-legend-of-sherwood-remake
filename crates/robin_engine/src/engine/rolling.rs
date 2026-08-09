//! Source-faithful owner-local execution of `RHANIMATION_ROLLING`.

use std::collections::BTreeMap;

use crate::coordinates::MapPoint;
use crate::element::{Entity, EntityId};
use crate::order::OrderType;
use crate::position_interface::vector_to_sector_0_to_15;
use crate::sprite::{FrameProgression, MotionMethod, MotionOrderContext, MotionState};

use super::animation::{ActorExecuteResult, AnimCompletionOutcomes};
use super::{EngineInner, LevelAssets};

fn rolling_initial_direction(position: MapPoint, goal: MapPoint) -> i16 {
    vector_to_sector_0_to_15(goal.x - position.x, goal.y - position.y)
}

impl EngineInner {
    /// `RHElementActorHuman::Execute(RHANIMATION_ROLLING)`.
    pub(super) fn tick_rolling_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> (
        Vec<EntityId>,
        AnimCompletionOutcomes,
        Option<ActorExecuteResult>,
    ) {
        let Some((seq_id, elem_idx, order, next_order)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .filter(|(_, _, order)| order.order_type == OrderType::Rolling)
            .map(|(seq_id, elem_idx, order)| {
                let element = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .expect("selected Rolling element disappeared");
                (
                    seq_id,
                    elem_idx,
                    order.clone(),
                    element.orders.get(1).cloned(),
                )
            })
        else {
            return (Vec::new(), AnimCompletionOutcomes::default(), None);
        };

        let initialising = self.world.entities[owner]
            .as_ref()
            .and_then(Entity::actor_data)
            .expect("Rolling owner must be an actor")
            .execute_order_initialising;
        let goal = MapPoint::new(order.target_x, order.target_y);

        // Snapshot everything before borrowing the owner mutably. Original
        // performs the collision scan at this creation-order slot.
        let snapshots =
            super::anti_collision::snapshot_all(&self.world.entities, &assets.profile_manager);
        let mut mobile_points: BTreeMap<u16, Vec<crate::repulsive::RepulsivePoint>> =
            BTreeMap::new();
        let mut mobile_lines: BTreeMap<u16, Vec<crate::fast_find_grid::GridLine>> = BTreeMap::new();
        let mut mobile_polygons: BTreeMap<u16, Vec<Vec<MapPoint>>> = BTreeMap::new();
        for mobile in &self.world.mobile_elements {
            if !mobile.active {
                continue;
            }
            mobile_points
                .entry(mobile.layer)
                .or_default()
                .extend(mobile.repulsive_points());
            mobile_lines
                .entry(mobile.layer)
                .or_default()
                .extend(mobile.repulsive_lines());
            mobile_polygons
                .entry(mobile.layer)
                .or_default()
                .push(mobile.motion_polygon.clone());
        }

        let old_pos;
        let layer;
        let motion;
        let speed;
        {
            let entity = self.world.entities[owner]
                .as_mut()
                .expect("Rolling owner disappeared");
            old_pos = entity.element_data().position_map();
            layer = entity.element_data().layer();
            if initialising {
                let direction = rolling_initial_direction(old_pos, goal);
                entity.element_data_mut().set_direction_instantly(direction);
                // Retire the old synthetic-flight side channel. The order is
                // the authoritative destination, just as in Original.
                entity.actor_data_mut().unwrap().pending_roll = None;
            }

            let _ = entity.position_iface_mut().turn();
            let direction = entity.element_data().direction() as u16;
            let context = MotionOrderContext {
                order_id: order.order_id,
                destination: goal,
                reverse: order.reverse,
                tolerance: order.tolerance,
                directional_tolerance: false,
                compute_direction: order.compute_direction,
                next_destination_same_action: next_order
                    .filter(|next| next.order_type == OrderType::Rolling)
                    .map(|next| MapPoint::new(next.target_x, next.target_y)),
                target_element: order.antagonist,
            };
            let (sprite_motion, frame_distance) = entity.sprite_mut().perform_motion(
                sim,
                Some(context),
                OrderType::Rolling,
                direction,
                FrameProgression::Default,
                false,
                MotionMethod::Walk,
                old_pos == goal,
            );
            // PerformMotion initializes the order increment and direction
            // goal before deciding whether turning slows this frame to 60%.
            let direction_differs = entity.element_data().direction()
                != i16::from(entity.position_iface().get_direction_goal());
            motion = sprite_motion;
            speed = super::movement::scaled_motion_distance(
                frame_distance,
                1.0,
                false,
                direction_differs,
            );
        }

        let mut effective_motion = motion;
        if speed != 0.0 {
            let mut mover = snapshots[owner]
                .as_ref()
                .expect("Rolling actor missing anti-collision snapshot")
                .clone();
            // PerformMotion has just installed the current order antagonist.
            mover.target_element = order.antagonist;
            let entity = self.world.entities[owner]
                .as_mut()
                .expect("Rolling owner disappeared before movement commit");
            let cached = entity.position_iface().get_increment_map();
            let anti_on = entity.position_iface().is_anti_collision_on();
            let (move_box, half_diagonal) = {
                let pi = entity.position_iface();
                (*pi.get_move_box(), pi.get_half_diagonal())
            };
            let was_deviated = entity.position_iface().is_deviated();
            let mut state = super::anti_collision::AntiCollisionState {
                pi: entity.position_iface_mut(),
                move_box,
                half_diagonal,
                goal_map: goal,
            };
            let (dx, dy) = super::anti_collision::apply_anti_collision_step(
                &mover,
                snapshots.as_slice(),
                &self.ai.global.repulsive_points,
                mobile_points.get(&layer).map(Vec::as_slice).unwrap_or(&[]),
                mobile_lines.get(&layer).map(Vec::as_slice).unwrap_or(&[]),
                mobile_polygons
                    .get(&layer)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                Some(&self.world.fast_grid),
                Some(&mut state),
                cached.x,
                cached.y,
                speed,
                anti_on,
            );
            let rebuild_after_deviation = state.pi.is_deviated() && state.pi.blocked_count == 0;
            let recovered = was_deviated && !state.pi.is_deviated();
            if rebuild_after_deviation && (dx != 0.0 || dy != 0.0) {
                let direction = vector_to_sector_0_to_15(dx, dy);
                entity
                    .element_data_mut()
                    .set_direction_goal(if order.reverse {
                        direction ^ 8
                    } else {
                        direction
                    });
            }
            let new_pos = MapPoint::new(old_pos.x + dx, old_pos.y + dy);
            entity.element_data_mut().set_position_map(new_pos);
            if rebuild_after_deviation && (dx != 0.0 || dy != 0.0) {
                entity.position_iface_mut().reset_increment_computed();
                entity.position_iface_mut().compute_increment_all(false);
            } else if recovered {
                entity.position_iface_mut().reset_increment_computed();
                entity.position_iface_mut().compute_increment_all(true);
            }
            let wait = entity
                .sprite()
                .wait_time(entity.sprite().current_row, entity.sprite().current_frame);
            entity
                .position_iface_mut()
                .update_forecasted_movement(speed, wait + 1);
            if entity
                .position_iface()
                .is_goal_reached(&self.world.fast_grid, None)
            {
                if !entity.position_iface().is_deviated()
                    && entity.position_iface().get_tolerance() == 0.0
                {
                    entity.element_data_mut().set_position_map(goal);
                }
                effective_motion = MotionState::Terminated;
                entity.element_data_mut().sprite.last_motion_state = Some(effective_motion);
            }
            entity.element_data_mut().update_grid_cell();
        }

        let new_pos = self.world.entities[owner]
            .as_ref()
            .unwrap()
            .element_data()
            .position_map();
        if self.check_for_line_crossing(assets, owner, old_pos, new_pos, layer) {
            self.update_roll_after_crossing(assets, owner);
        }
        self.check_for_non_elevation_line_crossing(sim, assets, owner, old_pos, new_pos, layer);

        let mut outcomes = AnimCompletionOutcomes::default();
        if motion == MotionState::Start {
            outcomes.non_interruptable_lifts.push((seq_id, elem_idx));
        }
        (
            Vec::new(),
            outcomes,
            Some(ActorExecuteResult {
                order_type: OrderType::Rolling,
                entry_seq_id: seq_id,
                entry_elem_idx: elem_idx,
                motion: effective_motion,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task194_roll_initialises_direction_before_selecting_sprite_row() {
        let position = MapPoint::new(1215.356_1, 1141.408_7);
        let goal = MapPoint::new(1215.505_4, 1168.039_6);

        let direction = rolling_initial_direction(position, goal);

        assert_eq!(direction, 8);
        assert_eq!(1232 + direction as u16, 1240);
        assert_ne!(1232 + 7, 1240, "the pre-roll direction selects row 1239");
    }

    #[test]
    fn task194_deviation_step_selects_original_direction_goal() {
        let direct = rolling_initial_direction(
            MapPoint::new(1215.356_1, 1141.408_7),
            MapPoint::new(1215.505_4, 1168.039_6),
        );
        let original_step = MapPoint::new(-2.106_323_2, 2.136_230_5);

        assert_eq!(direct, 8);
        assert_eq!(
            vector_to_sector_0_to_15(original_step.x, original_step.y),
            10
        );
    }

    #[test]
    fn translated_roll_retains_order_direction_computation() {
        let order = crate::order::Order::new(
            OrderType::Rolling,
            1215.505_4,
            1168.039_6,
            std::num::NonZeroU32::new(1).unwrap(),
        );

        assert!(order.compute_direction);
    }
}
