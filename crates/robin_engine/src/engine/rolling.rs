//! Behavior-faithful owner-local execution of the rolling animation.

use std::collections::BTreeMap;

use crate::coordinates::MapPoint;
use crate::element::{Entity, EntityId};
use crate::order::OrderType;
use crate::position_interface::vector_to_sector_0_to_15;
use crate::sprite::{FrameProgression, MotionMethod, MotionOrderContext, MotionState};

use super::{EngineInner, LevelAssets};

fn rolling_initial_direction(position: MapPoint, goal: MapPoint) -> i16 {
    vector_to_sector_0_to_15(goal.x - position.x, goal.y - position.y)
}

/// The original game's ordinary movement anti-collision recovery and arrival
/// snap use the position interface's live goal, not the order's authored
/// destination. Roll updates can stop a roll by replacing only that live goal
/// with the current point.
fn rolling_terminal_snap_point(
    position: &crate::position_interface::PositionInterface,
) -> MapPoint {
    position.map_goal()
}

/// Original-game rolling lands only when the
/// motion reports `TERMINATED`; `DONE` remains in the rolling posture until the
/// following owner slot turns that result into termination.
fn rolling_terminal_posture(motion: MotionState, is_dead: bool) -> Option<crate::element::Posture> {
    (motion == MotionState::Terminated).then_some(if is_dead {
        crate::element::Posture::Dead
    } else {
        crate::element::Posture::Lying
    })
}

impl EngineInner {
    /// original-game rolling action.
    pub(super) fn tick_rolling_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> Option<MotionState> {
        let Some((seq_id, elem_idx, order, next_order)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, owner)
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
            return None;
        };

        let initialising = self.world.entities[owner]
            .as_ref()
            .and_then(Entity::actor_data)
            .expect("Rolling owner must be an actor")
            .execute_order_initialising;
        let goal = MapPoint::new(order.target_x, order.target_y);

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
            // Motion processing initializes the order increment and direction
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
            let (entity, neighbours) = self
                .world
                .entities
                .split_owner(owner)
                .expect("Rolling owner disappeared before movement commit");
            let mut mover = super::anti_collision::CollisionMover::new(owner, entity);
            // Motion processing has just installed the current order antagonist.
            mover.target_element = order.antagonist;
            let cached = entity.position_iface().get_increment_map();
            let anti_on = entity.position_iface().is_anti_collision_on();
            let (move_box, half_diagonal, live_goal) = {
                let pi = entity.position_iface();
                (
                    *pi.get_move_box(),
                    pi.get_half_diagonal(),
                    rolling_terminal_snap_point(pi),
                )
            };
            let was_deviated = entity.position_iface().is_deviated();
            let mut state = super::anti_collision::AntiCollisionState {
                pi: entity.position_iface_mut(),
                move_box,
                half_diagonal,
                goal_map: live_goal,
            };
            let (dx, dy) = super::anti_collision::apply_anti_collision_step(
                &mover,
                super::anti_collision::CollisionWorld {
                    neighbours,
                    profiles: &assets.profile_manager,
                },
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
                    let live_goal = rolling_terminal_snap_point(entity.position_iface());
                    entity.element_data_mut().set_position_map(live_goal);
                }
                effective_motion = MotionState::Terminated;
                entity.element_data_mut().sprite.last_motion_state = Some(effective_motion);
            }
            entity.element_data_mut().update_grid_cell();
        }

        // The original game commits the landing posture before the actor update pops
        // the rolling order and resumes its postponed successor. That successor
        // must therefore generate its posture transition from Lying (normally a
        // StandingUp order), not from the stale pre-roll Upright posture.
        let landing_posture = {
            let entity = self
                .world
                .entities
                .get(owner)
                .expect("Rolling owner disappeared before landing");
            rolling_terminal_posture(effective_motion, entity.is_dead())
        };
        if let Some(posture) = landing_posture {
            // The original-game rolling action only calls
            // posture assignment here. Posture assignment
            // does not alter the position interface's deviated flag, so preserve the
            // roll's anti-vibration latch for the following stand-up/turn.
            self.world
                .entities
                .get_mut(owner)
                .expect("Rolling owner disappeared during landing")
                .set_posture(posture);
        }

        if motion == MotionState::Start {
            self.execute_non_interruptable_lifts((seq_id, elem_idx));
        }
        Some(effective_motion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task194_roll_initialises_direction_before_selecting_sprite_row() {
        let position = MapPoint::new(1_215.356_1, 1_141.408_7);
        let goal = MapPoint::new(1_215.505_4, 1_168.039_6);

        let direction = rolling_initial_direction(position, goal);

        assert_eq!(direction, 8);
        assert_eq!(1232 + direction as u16, 1240);
        assert_ne!(1232 + 7, 1240, "the pre-roll direction selects row 1239");
    }

    #[test]
    fn task194_deviation_step_selects_original_direction_goal() {
        let direct = rolling_initial_direction(
            MapPoint::new(1_215.356_1, 1_141.408_7),
            MapPoint::new(1_215.505_4, 1_168.039_6),
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
            1_215.505_4,
            1_168.039_6,
            std::num::NonZeroU32::new(1).unwrap(),
        );

        assert!(order.compute_direction);
    }

    #[test]
    fn stopped_rolling_uses_rewritten_live_goal_not_order_destination() {
        let authored_order_destination = MapPoint::new(1_213.184_1, 1_172.210_4);
        let stopped_here = MapPoint::new(1_208.699_5, 1_156.473_4);
        let mut position = crate::position_interface::PositionInterface::new();
        position.set_map_position(stopped_here);
        position.set_map_goal(authored_order_destination);

        // Roll updating's rejected-slope branch changes only the map goal.
        position.set_map_goal(stopped_here);

        assert_ne!(position.map_goal(), authored_order_destination);
        assert_eq!(
            rolling_terminal_snap_point(&position),
            stopped_here,
            "both anti-collision recovery and terminal snapping consume this live goal"
        );
    }

    #[test]
    fn rolling_lands_only_on_terminated_and_uses_life_state() {
        assert_eq!(
            rolling_terminal_posture(MotionState::Done, false),
            None,
            "Done is observed before the actor update converts it to termination"
        );
        assert_eq!(
            rolling_terminal_posture(MotionState::Terminated, false),
            Some(crate::element::Posture::Lying)
        );
        assert_eq!(
            rolling_terminal_posture(MotionState::Terminated, true),
            Some(crate::element::Posture::Dead)
        );
    }
}
