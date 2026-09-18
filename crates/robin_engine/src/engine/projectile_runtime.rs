//! Exact-owner projectile movement, collision, and immediate impact callbacks.

use super::archery::{ArrowHitOutcome, projectile_trajectory_origin};
use super::*;
use crate::bow_shot;
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::element::{Animation, Command, Entity, EntityId, ObjectType};
use crate::engine::TickCtx;
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(crate) fn tick_existing_projectile(&mut self, tcx: TickCtx<'_>, id: EntityId) -> bool {
        self.tick_projectile(tcx, id, false)
    }

    pub(crate) fn tick_new_projectile_once(&mut self, tcx: TickCtx<'_>, id: EntityId) -> bool {
        self.tick_projectile(tcx, id, true)
    }

    fn tick_projectile(
        &mut self,
        tcx: TickCtx<'_>,
        id: EntityId,
        primed_segment_already_advanced: bool,
    ) -> bool {
        let Entity::Projectile(projectile) = self
            .world
            .entities
            .get_mut(id)
            .expect("projectile update lost its owner")
        else {
            panic!("projectile update requires a projectile");
        };
        if !projectile.element.active {
            return false;
        }
        projectile.element.sprite.position_iface.new_move();
        if !projectile.projectile.flying {
            return true;
        }
        assert!(
            matches!(
                projectile.object.object_type,
                ObjectType::Arrow | ObjectType::Apple | ObjectType::Stone
            ),
            "projectile has a different concrete update owner"
        );
        let primed_start = projectile.projectile.launch_segment_start.take();
        let old = primed_start.unwrap_or(projectile.element.position());
        if primed_start.is_some() {
            if !primed_segment_already_advanced {
                let increment = projectile.projectile.velocity_increment;
                let mut position = projectile.element.position();
                position.x += increment.x;
                position.y += increment.y;
                position.z += increment.z;
                projectile.element.set_position(position);
                projectile
                    .element
                    .set_position_map_preserving_3d(position.to_map());
                projectile
                    .element
                    .finish_projectile_position_update(increment);
                projectile.projectile.frame_count =
                    projectile.projectile.frame_count.saturating_add(1);
            }
        } else {
            projectile.advance_trajectory_one_frame();
        }
        if projectile.projectile.falling {
            if !projectile.projectile.flying {
                return self.finish_projectile_landing(tcx, id);
            }
            return true;
        }
        let shield = bow_shot::projectile_shield_holder(
            &self.world.entities,
            &self.world.actor_registry_ids,
            old,
            self.expect_entity(id, "projectile collision")
                .element_data()
                .position(),
            match self.expect_entity(id, "projectile collision") {
                Entity::Projectile(projectile) => projectile.projectile.velocity_increment,
                _ => unreachable!(),
            },
        );
        if let Some(holder) = shield {
            let (entities, sight_obstacles, fast_find_grid, _) =
                self.world.entities_mut_with_sight(tcx.assets);
            let Entity::Projectile(projectile) =
                entities.get_mut(id).expect("shield impact lost projectile")
            else {
                unreachable!()
            };
            let fx = projectile_impact_fx(projectile.object.object_type);
            if projectile.object.object_type == ObjectType::Arrow {
                let check = bow_shot::TrajectoryObstacleCheck {
                    fast_find_grid,
                    sight_obstacles,
                    water_zones: Some(&tcx.assets.environment.water_zones),
                };
                if bow_shot::make_arrow_falling_down(projectile, true, Some(&check)) {
                    self.finish_projectile_landing(tcx, id);
                }
            }
            self.on_projectile_shield_hit(tcx, holder, fx);
            return true;
        }
        if let Some(victim) = bow_shot::projectile_human_victim(
            &self.world.entities,
            &self.world.actor_registry_ids,
            &self.mission_domain.diplomacy,
            id,
            old,
        ) {
            if !self.hit_projectile_human(tcx, id, victim, old)
                && matches!(self.expect_entity(id, "projectile impact continuation"),
                    Entity::Projectile(projectile) if !projectile.projectile.flying)
            {
                return self.finish_projectile_landing(tcx, id);
            }
        } else if let Some((target, command)) =
            bow_shot::projectile_target_victim(&self.world.entities, id, old)
        {
            self.burst_projectile(id);
            let shooter = match self.expect_entity(id, "projectile target impact") {
                Entity::Projectile(projectile) => projectile.projectile.shooter,
                _ => unreachable!(),
            };
            let mut element = crate::sequence::SequenceElement::new(1, command, Some(target));
            element.data = crate::sequence::SequenceElementData::Interaction {
                antagonist: shooter,
            };
            self.launch_element(tcx, element);
            self.stop_projectile(id);
            let impact = self
                .expect_entity(target, "projectile target impact")
                .element_data()
                .position_map();
            self.projectile_impact_feedback(tcx, id, impact);
        } else if matches!(self.expect_entity(id, "projectile landing"),
            Entity::Projectile(projectile) if !projectile.projectile.flying)
        {
            return self.finish_projectile_landing(tcx, id);
        }
        true
    }

    pub(super) fn on_projectile_shield_hit(
        &mut self,
        tcx: TickCtx<'_>,
        holder: EntityId,
        impact_fx: Option<u32>,
    ) {
        if let Some(fx_id) = impact_fx {
            let position = self
                .expect_entity(holder, "projectile shield impact")
                .element_data()
                .position_map();
            self.feedback
                .pending_side_effects
                .sounds
                .push(SoundCommand::Fx {
                    fx_id,
                    position,
                    material: None,
                });
        }
        let already_parrying = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, holder)
            .is_some_and(|(_, _, order)| {
                order.order_type == crate::order::OrderType::ParryingShield
            });
        if !already_parrying {
            self.launch_element(
                tcx,
                crate::sequence::SequenceElement::new(1, Command::ParryShield, Some(holder)),
            );
        }
    }

    fn hit_projectile_human(
        &mut self,
        tcx: TickCtx<'_>,
        id: EntityId,
        victim: EntityId,
        old: WorldPoint3D,
    ) -> bool {
        let Entity::Projectile(projectile) = self.expect_entity(id, "projectile human impact")
        else {
            unreachable!()
        };
        let kind = projectile.object.object_type;
        let shooter = projectile
            .projectile
            .shooter
            .expect("human impact requires a shooter");
        match kind {
            ObjectType::Apple => {
                self.burst_projectile(id);
                self.on_apple_hit_human(tcx, id, victim);
            }
            ObjectType::Stone => {
                self.burst_projectile(id);
                self.on_stone_hit_human(tcx, id, victim, shooter);
            }
            ObjectType::Arrow => {
                match self.classify_arrow_hit(tcx, victim, shooter) {
                    ArrowHitOutcome::PassThrough => return false,
                    ArrowHitOutcome::Ricochet => {
                        self.start_arrow_ricochet(tcx, id);
                        return false;
                    }
                    ArrowHitOutcome::Damage => {}
                }
                if !self.is_scroll_protected_civilian(victim) {
                    let Entity::Projectile(projectile) = self.expect_entity(id, "arrow damage")
                    else {
                        unreachable!()
                    };
                    let damage = projectile.projectile.damage;
                    self.queue_projectile_damage(
                        tcx,
                        victim,
                        shooter,
                        Command::ReceiveArrowDamage,
                        damage,
                        0,
                        Some(id),
                    );
                    if self
                        .expect_entity(victim, "arrow damage follow-up")
                        .is_npc()
                    {
                        let origin = projectile_trajectory_origin(
                            self.expect_entity(id, "arrow damage origin"),
                        )
                        .expect("arrow hit requires its trajectory origin");
                        self.dispatch_event_get_arrow(tcx, victim, origin);
                    }
                }
            }
            _ => unreachable!("human impact for unsupported projectile"),
        }
        self.rewind_projectile_to_human_hit_old_position(id, old);
        self.stop_projectile(id);
        let impact = self
            .expect_entity(victim, "projectile human impact")
            .element_data()
            .position_map();
        self.projectile_impact_feedback(tcx, id, impact);
        true
    }

    fn burst_projectile(&mut self, id: EntityId) {
        let Entity::Projectile(projectile) = self
            .world
            .entities
            .get_mut(id)
            .expect("projectile burst lost owner")
        else {
            unreachable!()
        };
        if matches!(
            projectile.object.object_type,
            ObjectType::Apple | ObjectType::Stone
        ) {
            bow_shot::set_projectile_animation(projectile, Animation::ObjectBursting);
        }
    }

    fn stop_projectile(&mut self, id: EntityId) {
        let Entity::Projectile(projectile) = self
            .world
            .entities
            .get_mut(id)
            .expect("projectile stop lost owner")
        else {
            unreachable!()
        };
        projectile.projectile.flying = false;
        projectile.projectile.trajectory.clear();
        projectile.projectile.trajectory_runtime.clear();
    }

    fn projectile_impact_feedback(&mut self, tcx: TickCtx<'_>, id: EntityId, position: MapPoint) {
        let was_distraction = self.emit_noise_distraction_impact(tcx, id, position);
        let Entity::Projectile(projectile) = self.expect_entity(id, "projectile impact sound")
        else {
            unreachable!()
        };
        if let Some(fx_id) = projectile_impact_fx(projectile.object.object_type)
            && (!was_distraction || self.control.sim_config.noise_distraction_feedback)
        {
            self.feedback
                .pending_side_effects
                .sounds
                .push(SoundCommand::Fx {
                    fx_id,
                    position,
                    material: None,
                });
        }
    }

    pub(super) fn finish_projectile_landing(&mut self, tcx: TickCtx<'_>, id: EntityId) -> bool {
        let Entity::Projectile(projectile) = self
            .world
            .entities
            .get_mut(id)
            .expect("projectile landing lost owner")
        else {
            unreachable!()
        };
        if projectile.projectile.dive {
            projectile.projectile.trajectory_frame_count = 0;
            projectile.projectile.trajectory.clear();
            projectile.projectile.trajectory_runtime.clear();
            self.maybe_splash_on_landing(tcx, id);
            return false;
        }
        if projectile.projectile.disappear {
            return false;
        }
        self.burst_projectile(id);
        let (entities, sight_obstacles, _, _) = self.world.entities_mut_with_sight(tcx.assets);
        let Entity::Projectile(projectile) =
            entities.get_mut(id).expect("projectile landing lost owner")
        else {
            unreachable!()
        };
        let noise_position = projectile.element.position_map();
        let noise_layer = projectile.element.optional_layer();
        {
            let position = projectile.element.position();
            let ground_z = projectile.element.obstacle_index().map(|handle| {
                let obstacle = sight_obstacles
                    .get(usize::from(handle))
                    .expect("projectile terminal obstacle is missing");
                crate::position_interface::PlaneZCoeffs::from_plane_points(
                    &obstacle.top_plane_points,
                )
                .compute_z(position.x, position.y)
            });
            let elevation = match ground_z {
                None => Some(0.001),
                Some(z)
                    if projectile.object.object_type != ObjectType::Arrow
                        && projectile.element.optional_layer().is_some() =>
                {
                    Some(z + 0.001)
                }
                _ => None,
            };
            if let Some(z) = elevation {
                let mut position = projectile.element.position();
                position.z = z;
                projectile.element.set_position(position);
                projectile
                    .element
                    .set_position_map_preserving_3d(position.to_map());
            }
        }
        projectile
            .element
            .finish_projectile_position_update(projectile.projectile.velocity_increment);
        let impact = projectile.element.position_map();
        if projectile.object.object_type == ObjectType::Arrow {
            let elevation = projectile.element.position().z.max(0.0) as u16;
            self.broadcast_noise_synchronously(
                tcx,
                crate::ai::NoiseType::Zonk,
                noise_position,
                noise_layer,
                crate::parameters_ai::NOISE_VOLUME_ZONK as u16,
                elevation,
                Some(id),
            );
        }
        self.projectile_impact_feedback(tcx, id, impact);
        true
    }
}

fn projectile_impact_fx(kind: ObjectType) -> Option<u32> {
    match kind {
        ObjectType::Apple => Some(509),
        ObjectType::Stone => Some(508),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::coordinates::{WorldPoint3D, WorldVec3D};
    use crate::element::{ElementData, Posture, ProjectileData, TrajectoryPoint};

    #[test]
    fn nonpositive_signed_waypoint_time_uses_whole_delta_and_advances_again() {
        for time in [0, 0x8000, u16::MAX] {
            let mut element = ElementData::from_initial_posture(Posture::Upright);
            element.set_position(WorldPoint3D::new(10.0, 20.0, 30.0));
            let first = WorldPoint3D::new(20.0, 30.0, 40.0);
            let second = WorldPoint3D::new(30.0, 40.0, 50.0);
            let mut projectile = ProjectileData {
                flying: true,
                trajectory_frame_count: u16::MAX,
                trajectory: vec![
                    TrajectoryPoint {
                        position: first,
                        time,
                    },
                    TrajectoryPoint {
                        position: second,
                        time: 1,
                    },
                ],
                ..Default::default()
            };
            assert!(!crate::element::advance_trajectory_one_frame(
                &mut element,
                &mut projectile
            ));
            assert_eq!(element.position(), first);
            assert_eq!(projectile.trajectory_frame_count, time.wrapping_sub(1));
            // A time of 0x8000 decrements to the largest positive signed timer.
            if time != 0x8000 {
                assert!(!crate::element::advance_trajectory_one_frame(
                    &mut element,
                    &mut projectile
                ));
                assert_eq!(element.position(), second);
                assert!(crate::element::advance_trajectory_one_frame(
                    &mut element,
                    &mut projectile
                ));
                assert_eq!(element.position(), second);
                assert_eq!(projectile.velocity_increment, WorldVec3D::ZERO);
                assert!(!projectile.flying);
            }
        }
    }
}
