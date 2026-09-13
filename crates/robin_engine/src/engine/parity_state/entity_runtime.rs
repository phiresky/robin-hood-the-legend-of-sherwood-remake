//! Typed per-entity runtime parity envelope.
//!
//! Split out of the former monolithic `parity_entity_runtime_state` (review
//! 01/F3): one method per serialized component, sequenced by
//! [`EntityRuntimeProjector::project`] in the original evaluation order so
//! invariant panics fire in the same order as before.

use super::*;
use crate::ai::{AiController, AiEntityHandle};
use crate::ai_enemy::EnemyAi;
use crate::ai_friendly::FriendlyAi;
use crate::element::Entity;
use crate::position_interface::{Layer, PositionInterfaceV48State, SectorHandle};
use std::borrow::Cow;

fn point2(x: f32, y: f32) -> projections::Point2 {
    projections::Point2 {
        x: typed_float(x),
        y: typed_float(y),
    }
}

fn point3(x: f32, y: f32, z: f32) -> projections::Point3 {
    projections::Point3 {
        x: typed_float(x),
        y: typed_float(y),
        z: typed_float(z),
    }
}

fn bounds(bbox: crate::coordinates::MapBBox) -> Option<projections::Bounds2> {
    bbox.0.map(|rect| projections::Bounds2 {
        min: point2(rect.min().x, rect.min().y),
        max: point2(rect.max().x, rect.max().y),
    })
}

fn door_state(door: &crate::gate::Door) -> projections::Door {
    let kind = match door.gate_type {
        crate::gate::GateType::Door => "door",
        crate::gate::GateType::Jump => "jump",
        crate::gate::GateType::None => "gate",
    };
    projections::Door {
        kind: kind.to_owned(),
        sector_out: door.sector_out.get(),
        sector_in: door.sector_in.get(),
        layer_out: door.layer_out,
        layer_in: door.layer_in,
        point_out: point2(door.point_out.x, door.point_out.y),
        point_in: point2(door.point_in.x, door.point_in.y),
    }
}

fn known_strike_command(strike: Option<crate::weapons::SwordStrike>) -> i32 {
    use crate::{element::Command, weapons::SwordStrike};
    match strike {
        None => Command::Null as i32,
        Some(SwordStrike::A) => Command::SwordstrikeThrustA as i32,
        Some(SwordStrike::B) => Command::SwordstrikeThrustB as i32,
        Some(SwordStrike::C) => Command::SwordstrikeThrustC as i32,
        Some(SwordStrike::D) => Command::SwordstrikeThrustD as i32,
        Some(SwordStrike::E) => Command::SwordstrikeThrustE as i32,
        Some(SwordStrike::F) => Command::SwordstrikeThrustF as i32,
        Some(SwordStrike::G) => Command::SwordstrikeThrustG as i32,
        Some(SwordStrike::H) => Command::SwordstrikeThrustH as i32,
        Some(SwordStrike::I) => Command::SwordstrikeThrustI as i32,
        Some(other) => {
            panic!("parity enemy known-strike slot contains invalid strike {other:?}")
        }
    }
}

fn plane_state(value: &crate::element::HumanPlaneState) -> human_projections::Plane {
    human_projections::Plane {
        a: point3(value.a.x, value.a.y, value.a.z),
        b: point3(value.b.x, value.b.y, value.b.z),
        normal: point3(value.normal.x, value.normal.y, value.normal.z),
        origin: point3(value.origin.x, value.origin.y, value.origin.z),
        u: point3(value.u.x, value.u.y, value.u.z),
        v: point3(value.v.x, value.v.y, value.v.z),
        az: typed_float(value.az),
        bz: typed_float(value.bz),
        dz: typed_float(value.dz),
        d: typed_float(value.d),
    }
}

fn bounding_box_state(
    value: crate::element::HumanBoundingBox2State,
) -> human_projections::BoundingBox {
    human_projections::BoundingBox {
        top_left: point2(value.top_left.x, value.top_left.y),
        bottom_right: point2(value.bottom_right.x, value.bottom_right.y),
        bounds_are_set: value.bounds_are_set,
    }
}

fn projectile_state(
    projectile: &crate::element::ProjectileData,
) -> projectile_projections::Projectile {
    let trajectory = projectile
        .trajectory
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let runtime = projectile.trajectory_runtime.get(index);
            projectile_projections::TrajectoryPoint {
                position: point3(point.position.x, point.position.y, point.position.z),
                time: point.time,
                // `null` is an explicit incomplete runtime mirror, not a fabricated
                // non-bounce/material value. Fresh trajectory construction must
                // populate these fields before v29 can pass dynamic-projectile traces.
                bounce: runtime.map(|runtime| runtime.bounce),
                material: runtime.map(|runtime| runtime.material),
            }
        })
        .collect::<Vec<_>>();
    projectile_projections::Projectile {
        flying: projectile.flying,
        dive: projectile.dive,
        magic_bullet: projectile.magic_bullet,
        frame_count: projectile.frame_count,
        trajectory_origin: projectile_projections::TrajectoryOrigin {
            map: point2(
                projectile.start_of_trajectory_x,
                projectile.start_of_trajectory_y,
            ),
            sector: projectile.trajectory_origin_sector,
            layer: projectile.trajectory_origin_layer.map(Layer::get),
        },
        flight_direction: projectile.flight_direction,
        start: point3(projectile.start.x, projectile.start.y, projectile.start.z),
        end: point3(projectile.end.x, projectile.end.y, projectile.end.z),
        shooter: projectile.shooter.map(typed_entity_reference),
        trajectory,
    }
}

/// Read-only projector for one live entity's serialized runtime frontier.
pub(super) struct EntityRuntimeProjector<'a> {
    engine: &'a Engine,
    id: EntityId,
    entity: &'a Entity,
    assets: &'a LevelAssets,
}

impl<'a> EntityRuntimeProjector<'a> {
    pub(super) fn new(engine: &'a Engine, id: EntityId, assets: &'a LevelAssets) -> Self {
        let entity = engine.inner.world.entities.get(id).unwrap_or_else(|| {
            panic!("parity runtime projection references missing entity {id:?}")
        });
        Self {
            engine,
            id,
            entity,
            assets,
        }
    }

    /// Sequencing shell for the complete entity envelope.
    pub(super) fn project(&self) -> projections::EntityRuntime<'a> {
        let position = self
            .entity
            .element_data()
            .sprite
            .position_iface
            .v48_serialized_state();
        let target = position.target_element.map(typed_entity_reference);
        let door = position.door.map(|handle| self.position_door(handle));
        let obstacle = position
            .obstacle
            .map(|handle| self.position_obstacle(&position, handle));
        let replacements = self.sprite_replacements();
        let position_state = self.position(&position, target, door, obstacle);
        let sprite_state = self.sprite(replacements);
        let npc_ai = self.npc_ai();
        let human_continuation = self.human_continuation();
        let human_structure = self.human_structure();
        let pc_core = self.pc_core();
        let pc_qa = self.pc_qa();
        let pc_interface = self.pc_interface();
        let pc_portrait = self.pc_portrait();
        let pc_tail = self.pc_tail();
        let subtype = self.subtype();
        projections::EntityRuntime {
            position: position_state,
            sprite: sprite_state,
            subtype,
            npc_ai,
            human_continuation,
            human_structure,
            pc_tail,
            pc_core,
            pc_qa,
            pc_interface,
            pc_portrait,
        }
    }

    fn sector(&self, handle: Option<SectorHandle>) -> Option<i16> {
        let id = self.id;
        handle.map(|handle| {
            let level = &self.engine.inner.world.fast_grid.level;
            let arena_index = handle.arena_index().map_or_else(
                || {
                    let public = crate::sector::SectorNumber::new(i16::from(handle));
                    level.sector_number_map.get(&public).copied().unwrap_or_else(|| {
                        panic!(
                            "parity position for {id:?} references missing public sector {handle}"
                        )
                    })
                },
                usize::from,
            );
            let sector = level.sectors.get(arena_index).unwrap_or_else(|| {
                panic!(
                    "parity position for {id:?} references missing sector arena index {arena_index} (public {handle})"
                )
            });
            assert_eq!(
                u16::from(sector.sector_number),
                handle.get(),
                "parity position for {id:?} sector arena index {arena_index} has public number {}, expected {handle}",
                sector.sector_number.get(),
            );
            sector.sector_number.get()
        })
    }

    fn jump_line(&self, index: Option<u32>) -> Option<projections::Line> {
        let index = index?;
        let line = self
            .engine
            .inner
            .world
            .fast_grid
            .level
            .jump_lines
            .get(usize::try_from(index).expect("parity enemy jump-line index exceeds usize"))
            .unwrap_or_else(|| panic!("parity enemy references missing jump line {index}"));
        Some(projections::Line {
            a: point2(line.point_a.x, line.point_a.y),
            b: point2(line.point_b.x, line.point_b.y),
        })
    }

    fn position_door(&self, door_handle: crate::gate::DoorIndex) -> projections::Door {
        let index = usize::from(door_handle);
        let door = self
            .engine
            .inner
            .script_domains
            .interactables
            .doors
            .get(index)
            .unwrap_or_else(|| panic!("parity position references missing door {index}"));
        door_state(door)
    }

    fn position_obstacle(
        &self,
        position: &PositionInterfaceV48State,
        handle: crate::position_interface::ObstacleHandle,
    ) -> projections::Obstacle {
        let handle = usize::from(handle);
        let obstacles = &self.assets.environment.static_sight_obstacles;
        let obstacle = obstacles.get(handle).unwrap_or_else(|| {
            panic!("parity position references missing static obstacle {handle}")
        });
        if let Some(layer) = position.layer {
            let index = obstacles[..handle]
                .iter()
                .filter(|candidate| candidate.is_projection_area())
                .count()
                // Original inserts its synthetic default-ground
                // projection area at ordinal zero before authored
                // obstacles. Rust represents that ground implicitly.
                + 1;
            if !obstacle.is_projection_area() {
                panic!(
                    "parity position on layer {} references non-projection obstacle {handle}",
                    layer.get()
                );
            }
            projections::Obstacle {
                kind: "projection".to_owned(),
                index,
            }
        } else {
            projections::Obstacle {
                kind: "sight".to_owned(),
                index: usize::try_from(obstacle.id).expect("obstacle ID exceeds usize"),
            }
        }
    }

    fn sprite_replacements(&self) -> Vec<projections::AnimationReplacement> {
        let sprite = &self.entity.element_data().sprite;
        let id = self.id;
        if sprite.anims_to_be_replaced.len() != sprite.replacing_anims.len() {
            panic!(
                "sprite replacement list length {} differs from replacement value length {} for {id:?}",
                sprite.anims_to_be_replaced.len(),
                sprite.replacing_anims.len()
            );
        }
        sprite
            .anims_to_be_replaced
            .iter()
            .zip(&sprite.replacing_anims)
            .map(|(&from, &to)| projections::AnimationReplacement {
                from: from as u32,
                to: to as u32,
            })
            .collect::<Vec<_>>()
    }

    fn position(
        &self,
        position: &PositionInterfaceV48State,
        target: Option<ParityEntityReference>,
        door: Option<projections::Door>,
        obstacle: Option<projections::Obstacle>,
    ) -> projections::Position {
        // Original's runtime frontier has the current mpointSprite projection
        // for ordinary entities. In Rust a map move invalidates the derived
        // cache without overwriting its raw serialized slot, which can still
        // hold the previous top-left. Targets are intentionally different:
        // their authored action point can differ from the visible sprite
        // anchor, so preserve their exact cached value.
        let current_sprite = if self.entity.is_fx_target() {
            crate::coordinates::SpriteTopLeft::new(position.sprite.x, position.sprite.y)
        } else {
            self.entity.gameplay_sprite_position()
        };
        projections::Position {
            computed_position: position.computed_position.bits(),
            computed_increment: position.computed_increment.bits(),
            material: position.material,
            posture: position.posture as u32,
            old_posture: position.old_posture as u32,
            direction: i16::from(position.direction),
            direction_goal: i16::from(position.direction_goal),
            slow_turn_count: position.slow_turn_count,
            direction_count: position.direction_count,
            layer: position.layer.map(Layer::get),
            layer_goal: position.layer_goal.map(Layer::get),
            tolerance: typed_float(position.tolerance),
            directional_tolerance: position.directional_tolerance,
            accumulate_movement_map: position.accumulate_movement_map,
            anti_collision_on: position.anti_collision_on,
            goal_next_valid: position.goal_next_valid,
            deviated: position.deviated,
            door_direction: position.door_direction,
            reversed_movement: position.reversed_movement,
            blocked_count: position.blocked_count,
            radius: typed_float(position.radius),
            emergency_lying_box: position.use_emergency_lying_box,
            sector: self.sector(position.sector),
            sector_goal: self.sector(position.sector_goal),
            door,
            obstacle,
            target,
            world: point3(
                position.position.x,
                position.position.y,
                position.position.z,
            ),
            map: point2(position.map.x, position.map.y),
            sprite: point2(current_sprite.x, current_sprite.y),
            old_world: point3(
                position.old_position.x,
                position.old_position.y,
                position.old_position.z,
            ),
            old_map: point2(position.old_map.x, position.old_map.y),
            old_sprite: point2(position.old_sprite.x, position.old_sprite.y),
            goal_map: point2(position.goal_map.x, position.goal_map.y),
            goal_next_map: point2(position.goal_next_map.x, position.goal_next_map.y),
            goal_world: point3(position.goal.x, position.goal.y, position.goal.z),
            increment: point3(
                position.increment.x,
                position.increment.y,
                position.increment.z,
            ),
            increment_map: point2(position.increment_map.x, position.increment_map.y),
            accumulated_movement_map: point2(
                position.accumulated_movement_map.x,
                position.accumulated_movement_map.y,
            ),
            forecasted_movement: point3(
                position.forecasted_movement.x,
                position.forecasted_movement.y,
                position.forecasted_movement.z,
            ),
            move_box: bounds(position.move_box_map),
            blocked_box: bounds(position.blocked_box),
        }
    }

    fn sprite(&self, replacements: Vec<projections::AnimationReplacement>) -> projections::Sprite {
        let sprite = &self.entity.element_data().sprite;
        projections::Sprite {
            row: sprite.current_row,
            frame: sprite.current_frame,
            frame_count: sprite.frame_count,
            flight_countdown: sprite.flight_frame_countdown,
            width: sprite.current_width,
            height: sprite.current_height,
            last_action: sprite.last_action as u32,
            last_processed_order_id: sprite.last_processed_order_id,
            masked: sprite.masked,
            alternate_profile: sprite.use_alternate_profile,
            action_done_frame: sprite.action_done_frame,
            action_done_counter: sprite.action_done_counter,
            last_sound_id: sprite.last_sound_id,
            behind_display_order_reference: sprite.behind_display_order_ref,
            display_order_reference: sprite.display_order_ref.map(typed_entity_reference),
            replacements,
        }
    }

    fn resolve_ai_handle(&self, handle: u32) -> ParityEntityReference {
        let resolved = self
            .engine
            .inner
            .world
            .entities
            .occupied()
            .find_map(|(candidate, _)| (candidate.index() == handle).then_some(candidate))
            .unwrap_or_else(|| panic!("parity local AI references missing handle {handle}"));
        typed_entity_reference(resolved)
    }

    fn resolve_optional_ai_handle(
        &self,
        handle: Option<AiEntityHandle>,
    ) -> Option<ParityEntityReference> {
        handle.map(|handle| self.resolve_ai_handle(handle.get()))
    }

    fn ai_handles(&self, values: &[u32]) -> Vec<ParityEntityReference> {
        values
            .iter()
            .copied()
            .map(|handle| self.resolve_ai_handle(handle))
            .collect::<Vec<_>>()
    }

    fn ai_position(&self, position: crate::ai::Position) -> projections::AiPosition {
        projections::AiPosition {
            map: point2(position.x, position.y),
            sector: self.sector(position.sector),
            layer: position.level,
        }
    }

    fn stimulus(&self, stimulus: &crate::ai::Stimulus) -> projections::Stimulus {
        use crate::ai::{StimulusInfo, StimulusType};
        assert_ne!(
            stimulus.stimulus_type,
            StimulusType::ForceBattleDecision,
            "parity local-AI stimulus contains Rust-only non-serializable type",
        );
        let (info_type, info) = match stimulus.info {
            StimulusInfo::None => (0, projections::StimulusInfo::None),
            StimulusInfo::Noise(noise) => (
                1,
                projections::StimulusInfo::Noise {
                    origin: projections::NoiseOrigin {
                        map: point2(noise.origin.x, noise.origin.y),
                        sector: self.sector(noise.origin.sector),
                        layer: noise.origin.layer.map(Layer::get),
                    },
                    noise_type: noise.noise_type as u32,
                    volume: noise.volume,
                    elevation: noise.elevation,
                },
            ),
            StimulusInfo::Position(position) => (
                2,
                projections::StimulusInfo::Position {
                    position: self.ai_position(position),
                },
            ),
            StimulusInfo::Human(entity) => (
                3,
                projections::StimulusInfo::Human {
                    entity: self.resolve_ai_handle(entity.get()),
                },
            ),
            StimulusInfo::Hint(hint) => (
                4,
                projections::StimulusInfo::Hint {
                    position: self.ai_position(hint.seek_point),
                    teller: self.resolve_ai_handle(hint.who_tells_me.get()),
                    seek_flags: hint.seek_flags,
                },
            ),
            StimulusInfo::Object(entity) => (
                5,
                projections::StimulusInfo::Object {
                    entity: self.resolve_ai_handle(entity.get()),
                },
            ),
            StimulusInfo::Stolen(stolen) => (
                6,
                projections::StimulusInfo::Stolen {
                    object: self.resolve_ai_handle(stolen.object.get()),
                    thief: self.resolve_ai_handle(stolen.thief.get()),
                },
            ),
            StimulusInfo::Combat(combat) => (
                7,
                projections::StimulusInfo::Combat {
                    actor: self.resolve_ai_handle(combat.actor_npc.get()),
                    enemy_position: self.ai_position(combat.enemy_position),
                },
            ),
            StimulusInfo::DoorCombat(combat) => (
                8,
                projections::StimulusInfo::DoorCombat {
                    delay: combat.delay,
                    direction: combat.direction,
                    goal: self.ai_position(combat.goal),
                    adversary: self.resolve_optional_ai_handle(combat.adversary),
                },
            ),
            StimulusInfo::Index(value) => (9, projections::StimulusInfo::Index { value }),
            StimulusInfo::LegacyInvalidType(raw) => {
                panic!("parity local-AI stimulus retains active invalid type word {raw}")
            }
        };
        projections::Stimulus {
            stimulus_type: stimulus.stimulus_type as u32,
            info_type,
            owner: self.resolve_optional_ai_handle(stimulus.owner),
            to_whole_patrol: stimulus.to_whole_patrol,
            info,
        }
    }

    fn patrol_stimulus(
        &self,
        stimulus: Option<&crate::ai::Stimulus>,
    ) -> Option<projections::Stimulus> {
        use crate::ai::{StimulusInfo, StimulusType};
        let stimulus = stimulus?;
        let is_default = stimulus.stimulus_type == StimulusType::NoEvent
            && matches!(
                stimulus.info,
                StimulusInfo::None | StimulusInfo::LegacyInvalidType(_)
            )
            && stimulus.owner.is_none()
            && !stimulus.to_whole_patrol;
        if is_default {
            None
        } else {
            Some(self.stimulus(stimulus))
        }
    }

    fn npc_ai(&self) -> Option<projections::NpcAi<'a>> {
        let entity: &'a Entity = self.entity;
        let npc = entity.npc_data()?;
        let ai = npc.ai_brain.base()?;
        let mut state = self.npc_ai_base(ai);
        let subclass = match &npc.ai_brain {
            crate::element::AiBrain::Friendly(friendly) => Some(
                projections::NpcSubclass::Friendly(self.friendly_ai(friendly)),
            ),
            crate::element::AiBrain::Enemy(enemy) => {
                Some(projections::NpcSubclass::Enemy(self.enemy_ai(enemy)))
            }
            crate::element::AiBrain::None => None,
        };
        state.subclass = subclass;
        Some(state)
    }

    fn npc_ai_door(&self, index: Option<crate::gate::DoorIndex>) -> Option<projections::Door> {
        let index = index?;
        let door = self
            .engine
            .inner
            .script_domains
            .interactables
            .doors
            .get(usize::from(index))
            .unwrap_or_else(|| panic!("parity AI references missing door {index}"));
        Some(door_state(door))
    }

    fn patrol_path_status(&self, ai: &AiController) -> projections::PatrolPathStatus {
        if let Some(path) = &ai.patrol_path {
            projections::PatrolPathStatus {
                current_waypoint_index: path.current_waypoint_index,
                last_waypoint_index: path.last_waypoint_index,
                forward: path.forward,
                hiking_path_index: Some(path.hiking_path_index.get()),
                history: path
                    .history
                    .iter()
                    .map(|entry| projections::PathHistory {
                        position: self.ai_position(entry.position),
                        direction: entry.direction,
                        distance: entry.distance,
                    })
                    .collect::<Vec<_>>(),
            }
        } else {
            let path = &ai.detached_patrol_path_status;
            projections::PatrolPathStatus {
                current_waypoint_index: path.current_waypoint_index,
                last_waypoint_index: path.last_waypoint_index,
                forward: path.forward,
                hiking_path_index: path.hiking_path_index.map(|id| id.get()),
                history: path
                    .history
                    .iter()
                    .map(|entry| projections::PathHistory {
                        position: self.ai_position(entry.position),
                        direction: entry.direction,
                        distance: entry.distance,
                    })
                    .collect::<Vec<_>>(),
            }
        }
    }

    /// Shared AI-controller base; the subclass is attached by [`Self::npc_ai`].
    fn npc_ai_base(&self, ai: &'a AiController) -> projections::NpcAi<'a> {
        let patrol_path_status = self.patrol_path_status(ai);
        projections::NpcAi {
            subclass: None,
            last_goto: projections::NpcAiLastGoto {
                destination: self.ai_position(ai.last_goto_destination),
                flags: ai.last_goto_flags.bits(),
                stuck_counter: ai.stuck_counter,
            },
            forbidden_remarks: Cow::Borrowed(&ai.forbidden_remark_ids),
            current_remark_flags: ai.current_remark_flags,
            owner: ai.owner_entity_id.map(typed_entity_reference),
            state: ai.current_state as u32,
            old_state: ai.old_state,
            substate: ai.current_substate as u32,
            music_alert: ai.current_music_alert_status as u32,
            timer_launch_substate: ai.substate_at_last_timer_launch as u32,
            attitude: ai.attitude as u32,
            blood_alcohol: ai.blood_alcohol,
            initial_action: ai.initial_action,
            number_of_looks: ai.number_of_looks,
            can_move: ai.can_move,
            path_control: projections::NpcAiPathControl {
                stop_before_end: ai.stop_before_end_of_path,
                use_max_norm: ai.use_max_norm_to_stop_before_end_of_path,
                stop_distance: ai.stop_before_end_of_path_distance,
                status: patrol_path_status,
                has_patrol_path: ai.has_patrol_path,
                macro_cursor: ai.has_patrol_path.then_some(ai.macro_command_offset),
            },
            r#macro: projections::NpcAiMacro {
                remaining_bytes: ai.number_of_remaining_macro_bytes,
                in_progress: ai.macro_in_progress,
                started_this_frame: ai.macro_started_in_this_frame,
                next_rand: ai.next_macro_rand,
                next_rand_forecasted: ai.next_macro_rand_forecasted,
            },
            targets: projections::NpcAiTargets {
                primary: self.resolve_optional_ai_handle(ai.primary_target),
                friend_in_trouble: self.resolve_optional_ai_handle(ai.friend_in_trouble),
                detected_body: self.resolve_optional_ai_handle(ai.detected_body),
                interesting_object: self.resolve_optional_ai_handle(ai.interesting_object),
                antagonist: self.resolve_optional_ai_handle(ai.antagonist),
                last_stimulus_actor: self.resolve_optional_ai_handle(ai.last_stimulus_actor),
            },
            timers: projections::NpcAiTimers {
                running: ai.timer_is_running,
                ring: ai.when_does_timer_ring,
                macro_running: ai.macro_timer_is_running,
                macro_ring: ai.when_does_macro_timer_ring,
                standing_around: ai.standing_around_timer,
            },
            sorrow: ai.sorrow_level,
            last_stimuli: ai.last_stimulus.map(|stimulus| stimulus as u32),
            last_stimulus_multiplicities: ai.last_stimulus_multiplicity,
            group: projections::NpcAiGroup {
                is_master: ai.is_master,
                master: self.resolve_optional_ai_handle(ai.master),
                us: self.ai_handles(&ai.list_us),
                alerted_us: self.ai_handles(&ai.list_alerted_us),
                staying_us: self.ai_handles(&ai.list_staying_us),
            },
            seek_position: self.ai_position(ai.seek_position),
            alert_soldiers_point: self.ai_position(ai.alert_soldiers_point),
            first_try: ai.first_try,
            panic: projections::NpcAiPanic {
                center: point2(ai.panic_center_x, ai.panic_center_y),
                lasting_runs: ai.lasting_panic_runs,
                directed: ai.directed_panic,
            },
            movement_failures: projections::NpcAiMovementFailures {
                could_not_reach: ai.couldnt_reachpoint,
                already_on_point: ai.already_on_point,
                already_turned: ai.already_turned,
            },
            likes_to_sit: ai.likes_to_sit_around,
            special_action: ai.special_action,
            friends_alerted: ai.friends_are_alerted,
            stay_at_home: ai.is_stay_at_home,
            locks: ai.locks_flag_field.bits(),
            was_busy: ai.was_busy,
            stimulus_queue: ai
                .stimulus_queue
                .iter()
                .map(|stimulus| self.stimulus(stimulus))
                .collect::<Vec<_>>(),
            script_locked: ai.script_locked,
            remember_events: ai.remember_events,
            leave_house_number: ai.leave_house_number,
            legacy_continuation: projections::NpcAiLegacyContinuation {
                remaining_tequila_gulps: ai.remaining_tequila_gulps,
                last_hint_actuality: ai.last_hint_actuality,
                last_hint_subject: ai.last_hint_subject as u32,
                current_door: self.npc_ai_door(ai.my_door_index),
                looking_for_help_because_enemy_seen: ai.looking_for_help_because_enemy_seen,
            },
            object_memory: projections::NpcAiObjectMemory {
                forgotten: self.ai_handles(&ai.forgotten_objects),
                desire: self.resolve_optional_ai_handle(ai.object_of_desire),
                checkpoint_charly: self.resolve_optional_ai_handle(ai.checkpoint_charly),
                synchronize_charly: self.resolve_optional_ai_handle(ai.synchronize_charly),
            },
            inside_halt: ai.inside_halt_method,
            synchronizing_actors: self.ai_handles(&ai.synchronizing_actors),
            default_path_flags: ai.default_path_walking_flags.bits(),
            current_remark: ai.current_remark as u32,
            emoticon: projections::NpcAiEmoticon {
                r#type: ai.current_emoticon_type as u32,
                expiration: ai.emoticon_expiration_date,
                has_expiration: ai.emoticon_has_expiration_date,
            },
            knocked_out_in_money_fight: ai.knocked_out_in_money_fight,
            got_beggar_trick: ai.got_the_beggar_trick,
            reconnaissance: projections::NpcAiReconnaissance {
                report_type: ai.my_reconnaissance_report.report_type as u32,
                seek_position: self.ai_position(ai.my_reconnaissance_report.seek_position),
                seen_bodies: self.ai_handles(&ai.my_reconnaissance_report.seen_bodies),
                charly: self.resolve_optional_ai_handle(ai.my_reconnaissance_report.charly),
                charly_seen: ai.my_reconnaissance_report.charly_seen,
            },
            patrol: projections::NpcAiPatrol {
                chief: ai.patrol_chief.map(typed_entity_reference),
                active: ai
                    .patrol
                    .iter()
                    .copied()
                    .map(typed_entity_reference)
                    .collect::<Vec<_>>(),
                missed: ai
                    .missed_patrol_members
                    .iter()
                    .copied()
                    .map(typed_entity_reference)
                    .collect::<Vec<_>>(),
                theoretical: ai
                    .theoretical_patrol
                    .iter()
                    .copied()
                    .map(typed_entity_reference)
                    .collect::<Vec<_>>(),
                stopped: ai.patrol_stopped,
                direction: ai.patrol_direction,
            },
        }
    }

    fn friendly_ai(&self, friendly: &FriendlyAi) -> projections::FriendlyAi {
        projections::FriendlyAi {
            kind: "friendly".to_owned(),
            fleeing_seen_enemy_counter: friendly.fleeing_seen_enemy_counter,
            beggar_dont_talk_counter: friendly.beggar_dont_talk_counter,
            wants_to_talk: friendly.wants_to_talk,
            last_talk_partner: self.resolve_optional_ai_handle(friendly.last_talk_partner),
            can_go_away: friendly.can_go_away,
        }
    }

    fn enemy_ai(&self, enemy: &'a EnemyAi) -> projections::EnemyAi<'a> {
        projections::EnemyAi {
            kind: "enemy".to_owned(),
            frame_when_missed_charly: enemy.frame_when_missed_charly,
            frame_when_enemy_detected: enemy.base.frame_when_enemy_detected,
            fleeing_seen_enemy_counter: enemy.fleeing_seen_enemy_counter,
            pc_gone_direction: enemy.pc_gone_away_in_this_direction,
            detected_something_there: self.ai_position(enemy.detected_something_there),
            missed_pc: self.resolve_optional_ai_handle(enemy.missed_pc),
            last_seek_direction_index: enemy.last_seek_direction_index,
            beggar_to_examine: self.resolve_optional_ai_handle(enemy.beggar_to_examine),
            pc_missed: enemy.pc_missed,
            task_priorities: projections::EnemyAiTaskPriorities {
                current: enemy.current_task_priority,
                minimal: enemy.minimal_task_priority,
                new: enemy.new_task_priority,
            },
            different_checkpoints: enemy.number_of_different_checkpoints,
            delta_sorrow: enemy.base.delta_sorrow_level,
            thirsty: enemy.thirsty,
            old_life_points: enemy.old_life_points,
            initial_life_points: enemy.initial_life_points,
            old_odds: enemy.old_odds,
            position_change_locked_for_test: enemy.position_change_locked_for_test,
            heard_nets: self.ai_handles(&enemy.heard_nets),
            other_seen_ale: self.ai_handles(&enemy.other_seen_ale),
            search_charly_way: enemy
                .search_charly_way
                .iter()
                .map(|position| self.ai_position(*position))
                .collect::<Vec<_>>(),
            missed_in_action: self.ai_handles(&enemy.base.missed_in_action),
            other_bodies_to_examine: self.ai_handles(&enemy.other_bodies_to_examine),
            beggars_to_control: self.ai_handles(&enemy.beggars_to_control),
            them: self.ai_handles(&enemy.list_them),
            ambush_point_array_reset: enemy.ambush_point_array_reset,
            ambush_point_status: enemy
                .ambush_point_status
                .iter()
                .map(|status| *status as u32)
                .collect::<Vec<_>>(),
            my_seek_points: Cow::Borrowed(&enemy.my_seek_points),
            personal_seek_point_1: enemy
                .personal_seek_point_1
                .as_ref()
                .map(|point| typed_seek_point(point, self.ai_position(point.position))),
            personal_seek_point_2: enemy
                .personal_seek_point_2
                .as_ref()
                .map(|point| typed_seek_point(point, self.ai_position(point.position))),
            seek_center: self.ai_position(enemy.seek_center),
            actual_seek_point: enemy.actual_seek_point,
            seek_point_view_directions: Cow::Borrowed(&enemy.seek_point_view_directions),
            positions_of_beggars_to_control: enemy
                .positions_of_beggars_to_control
                .iter()
                .map(|position| self.ai_position(*position))
                .collect::<Vec<_>>(),
            seek_flags: enemy.seek_flags.bits(),
            seen_dead_body: enemy.seen_dead_body,
            seeking_charly: enemy.seeking_charly,
            forced_next_battle_decision: enemy.forced_next_battle_decision as u32,
            reset_battle_decision: enemy.reset_battle_decision,
            synchronize_index: enemy.base.synchronize_index,
            initial_view_cone: enemy.base.initial_view_cone as u32,
            company_number: enemy.company_number,
            left_combat_neighbour: self.resolve_optional_ai_handle(enemy.left_combat_neighbour),
            right_combat_neighbour: self.resolve_optional_ai_handle(enemy.right_combat_neighbour),
            attentive: enemy.attentive,
            will_be_attentive: enemy.will_be_attentive,
            forced_attentive: enemy.forced_attentive,
            guarded_pc: enemy
                .guarded_pc
                .map(|id| typed_entity_reference(EntityId::Pc(id))),
            tower_guard: enemy.tower_guard,
            combat_trainer: enemy.combat_trainer,
            gather_position: self.ai_position(enemy.gather_position),
            gather_direction: enemy.gather_direction,
            gather_position_instructed: enemy.gather_position_instructed,
            officers_position: self.ai_position(enemy.officers_position),
            previous_state: enemy.previous_state,
            previous_substate: enemy.previous_substate,
            reported_to_officer: enemy.reported_to_officer,
            missed_soldier_timer: enemy.missed_soldier_timer,
            old_money: enemy.old_money,
            other_seen_money: self.ai_handles(&enemy.other_seen_money),
            money_fight_enemies: self.ai_handles(&enemy.money_fight_enemies),
            money_fight_victims: self.ai_handles(&enemy.money_fight_victims),
            archer_behind_me: self.resolve_optional_ai_handle(enemy.archer_behind_me),
            shield_bearer_before_me: self.resolve_optional_ai_handle(enemy.shield_bearer_before_me),
            already_seen_bodies: self.ai_handles(&enemy.already_seen_bodies),
            my_line_jump: self.jump_line(enemy.my_line_jump),
            shield_bearer_direction: enemy.shield_bearer_direction,
            phalanx_aborted: enemy.phalanx_aborted,
            changed_to_alert_path: enemy.changed_to_alert_path,
            shooting_point: enemy.my_shooting_point.map(|(sector_index, point_index)| {
                projections::ShootingPoint {
                    sector_index,
                    point_index,
                }
            }),
            archery_sector: enemy.my_archery_sector,
            archery_sector_index: enemy.my_archery_sector_index,
            archery_point_index: enemy.my_archery_point_index.0,
            archery_point_increment: enemy.my_archery_point_increment,
            enemy_seen_below: enemy.enemy_seen_below,
            enemy_had_this_elevation: enemy.enemy_had_this_elevation,
            known_enemy_strike_commands: [
                known_strike_command(enemy.known_enemy_strike_1),
                known_strike_command(enemy.known_enemy_strike_2),
                known_strike_command(enemy.known_enemy_strike_3),
            ],
            last_stimulus_dispatched_to_patrol: self
                .patrol_stimulus(enemy.last_stimulus_dispatched_to_patrol.as_ref()),
        }
    }

    fn human_continuation(&self) -> Option<human_projections::HumanContinuation<'a>> {
        let entity: &'a Entity = self.entity;
        entity
            .human_data()
            .map(|human| human_projections::HumanContinuation {
                already_detectable_body: human.already_detectable_body,
                concussion_healing_timeout: human.concussion_healing_timeout,
                tiredness: human.tiredness,
                concussion: human.concussion_of_the_brain,
                parry_counter: human.parry_counter,
                detectable_list_index: human.detectable_list_index,
                invulnerable: human.invulnerable,
                last_motion_was_step_back: human.last_motion_was_step_back_in_combat,
                smalltalk_initiative: human.smalltalk_initiative,
                received_smalltalk_initiative: human.received_smalltalk_initiative,
                smalltalk_hint: human.smalltalk_hint as u32,
                smalltalk_hint_opponent: human.smalltalk_hint_opponent.map(typed_entity_reference),
                relative_fighting_ability: human.relative_fighting_ability,
                hollow_man: human.hollow_man,
                killed_by_accident: human.killed_by_accident,
                stuck_under_nets_counter: human.stuck_under_nets_counter,
                sword_strike_boredom: Cow::Borrowed(&human.sword_strike_boredom),
                carrier: human.carrier.map(typed_entity_reference),
                small_repulsive_radius: human.small_repulsive_radius,
                hulk: human_projections::Hulk {
                    running: human.running_hulk,
                    time: human.time_hulk,
                    level: human.hulk_level,
                    direction: human.hulk_direction,
                    speed: typed_float(human.hulk_speed),
                },
            })
    }

    fn human_structure(&self) -> Option<human_projections::HumanStructure> {
        let human = self.entity.human_data()?;
        let opponents = human
            .opponents
            .iter_with_jump_lines()
            .map(|(opponent, line)| human_projections::Opponent {
                entity: typed_entity_reference(opponent),
                jump_line: self.jump_line(line.map(u32::from)),
            })
            .collect::<Vec<_>>();
        let repulsive = &human.repulsive_point;
        let shield = &human.shield;
        let sequence_ordinals: std::collections::BTreeMap<_, _> = self
            .engine
            .inner
            .orders
            .sequence_manager
            .sequences_iter()
            .enumerate()
            .map(|(ordinal, sequence)| (sequence.id, ordinal))
            .collect();
        let sequence_ref = |value: crate::sequence::SequenceElementRef| {
            let sequence = sequence_ordinals
                .get(&value.sequence_id)
                .copied()
                .unwrap_or_else(|| {
                    panic!("parity human pending shoot points outside sequence manager: {value:?}")
                });
            human_projections::SequenceReference {
                sequence,
                element: value.element_index,
            }
        };
        Some(human_projections::HumanStructure {
            opponents,
            repulsive_point: human_projections::RepulsivePoint {
                position: point2(repulsive.position.x, repulsive.position.y),
                concave: repulsive.concave,
                limit_left: point2(repulsive.limit_left.x, repulsive.limit_left.y),
                limit_right: point2(repulsive.limit_right.x, repulsive.limit_right.y),
                action_radius: typed_float(repulsive.action_radius),
                force_a: typed_float(repulsive.force_a),
                force_b: typed_float(repulsive.force_b),
                radius: typed_float(repulsive.radius),
                id: repulsive.id,
                affects_pcs: repulsive.affects_pcs,
                affects_soldiers: repulsive.affects_soldiers,
                affects_civilians: repulsive.affects_civilians,
                affects_animals: repulsive.affects_animals,
            },
            building: self.sector(human.building_sector),
            shield: human_projections::Shield {
                points: shield
                    .points
                    .iter()
                    .map(|value| human_projections::ShieldPoint {
                        obstacle: value.obstacle.map(typed_float),
                        polygon: point2(value.polygon.x, value.polygon.y),
                    })
                    .collect::<Vec<_>>(),
                top_plane: plane_state(&shield.top_plane),
                bottom_plane: plane_state(&shield.bottom_plane),
                box_3d: shield.box_3d.map(typed_float),
                ground_box: bounding_box_state(shield.ground_box),
                screen_box: bounding_box_state(shield.screen_box),
                on_ground: shield.on_ground,
            },
            sword_sweep: human_projections::SwordSweep {
                victims: human
                    .sword_sweep
                    .victims
                    .iter()
                    .copied()
                    .map(typed_entity_reference)
                    .collect::<Vec<_>>(),
                initial_angle: typed_float(human.sword_sweep.initial_angle),
                current_angle: typed_float(human.sword_sweep.current_angle),
                final_angle: typed_float(human.sword_sweep.final_angle),
            },
            pending_shoots: human
                .pending_shoots
                .iter()
                .copied()
                .map(sequence_ref)
                .collect::<Vec<_>>(),
        })
    }

    fn pc_core(&self) -> Option<human_projections::PcCore<'a>> {
        let entity: &'a Entity = self.entity;
        let id = self.id;
        entity.pc_data().map(|pc| {
            const ACTIONS: usize = 3;
            assert_eq!(
                pc.disabled_actions.len(),
                ACTIONS,
                "PC {id:?} parity projection has {} permanent action flags, expected {ACTIONS}",
                pc.disabled_actions.len()
            );
            assert_eq!(
                pc.disabled_actions_temp.len(),
                ACTIONS,
                "PC {id:?} parity projection has {} temporary action flags, expected {ACTIONS}",
                pc.disabled_actions_temp.len()
            );
            let campaign_description_index = pc.campaign_description_index.unwrap_or_else(|| {
                panic!("PC {id:?} parity projection has no campaign description index")
            });
            human_projections::PcCore {
                work_icon: pc.work_icon as u32,
                campaign_description_index,
                playable: pc.playable,
                beam_me_index: pc.beam_me_index,
                already_selected: pc.already_selected,
                belt_seen: pc.belt_seen,
                feet_seen: pc.feet_seen,
                head_seen: pc.head_seen,
                immortal: pc.immortal,
                fried_psykokwack: pc.fried_psykokwack,
                list_index: pc.list_index,
                teleport_counter: pc.teleport_counter,
                current_action: pc.current_action as u32,
                saved_action: pc.saved_action as u32,
                disabled_actions: Cow::Borrowed(&pc.disabled_actions),
                disabled_actions_temp: Cow::Borrowed(&pc.disabled_actions_temp),
                position_before_teleport: point2(
                    pc.position_before_teleport.x,
                    pc.position_before_teleport.y,
                ),
            }
        })
    }

    fn pc_qa(&self) -> Option<Vec<human_projections::PcQa>> {
        let id = self.id;
        self.entity.pc_data().map(|pc| {
            const QA_SLOTS: usize = crate::macro_store::NUMBER_OF_QA_MEMORY;
            for (name, length) in [
                ("types", pc.quick_action_types.len()),
                ("actions", pc.quick_action_sequences.len()),
                ("seeks", pc.quick_seek_sequences.len()),
                ("special-counts", pc.quick_action_special_counts.len()),
                ("buttons", pc.quick_action_buttons.len()),
                ("interactors", pc.quick_action_interactors.len()),
                ("titbits", pc.titbits.len()),
            ] {
                assert_eq!(
                    length, QA_SLOTS,
                    "PC {id:?} parity projection has {length} {name}, expected {QA_SLOTS}"
                );
            }
            (0..QA_SLOTS)
                .map(|slot| human_projections::PcQa {
                    special_count: pc.quick_action_special_counts[slot],
                    quickito: pc.quick_action_types[slot] as u32,
                    titbit: pc.titbits[slot].map(crate::titbit::TitbitId::get),
                    button: pc.quick_action_buttons[slot],
                    interactor: pc.quick_action_interactors[slot].map(typed_entity_reference),
                    action_size: pc.quick_action_sequences[slot]
                        .as_ref()
                        .map(|sequence| sequence.len()),
                    seek_size: pc.quick_seek_sequences[slot]
                        .as_ref()
                        .map(|sequence| sequence.len()),
                })
                .collect::<Vec<_>>()
        })
    }

    fn pc_interface(&self) -> Option<human_projections::PcInterface> {
        self.entity
            .pc_data()
            .map(|pc| human_projections::PcInterface {
                playable: pc.playable,
                displayed: !pc.interface_hidden,
            })
    }

    fn pc_portrait(&self) -> Option<human_projections::PcPortrait> {
        let id = self.id;
        self.entity.pc_data().map(|pc| {
            let profile = self
                .assets
                .profile_manager
                .get_character(pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "PC {id:?} portrait has missing profile {}",
                        pc.profile_index
                    )
                });
            let description = self
                .engine
                .inner
                .pc_description_for_pc_data(pc)
                .unwrap_or_else(|| panic!("PC {id:?} portrait has no campaign description"));
            let quantities = profile
                .actions
                .map(|action| description.status.get_ammo(action));
            human_projections::PcPortrait {
                quantities,
                two_buttons_mode: profile.actions[2] == crate::profiles::Action::NoAction,
                displayed: !pc.interface_hidden,
                burned: pc.portrait.burned,
                open: pc.portrait.open,
                life_level: typed_float(f32::from(pc.life_points)),
                trumpet_enabled: pc.trumpet_enabled,
                quick_icons: pc
                    .portrait
                    .quick_icons
                    .iter()
                    .map(|icon| human_projections::QuickIcon {
                        titbit: icon.titbit_id.map(crate::titbit::TitbitId::get),
                        running: icon.running,
                    })
                    .collect::<Vec<_>>(),
            }
        })
    }

    fn pc_tail(&self) -> Option<human_projections::PcTail> {
        self.entity.pc_data().map(|pc| human_projections::PcTail {
            carried: pc.carried.map(typed_entity_reference),
            carried_posture: pc.carried_posture,
            shield_danger_point: point3(
                pc.shield_danger_point.x,
                pc.shield_danger_point.y,
                pc.shield_danger_point.z,
            ),
            shield_protected: pc.shield_protected.map(typed_entity_reference),
            shield_protector: pc.shield_protector.map(typed_entity_reference),
            guard: pc.guard.map(typed_entity_reference),
            time_till_reinforcement: pc.time_till_reinforcement,
            last_ammo_dropping_position: point2(
                pc.last_ammo_dropping_position.x,
                pc.last_ammo_dropping_position.y,
            ),
            last_dropped_ammo: pc.last_dropped_ammo.map(typed_entity_reference),
            update_last_dropped_ammo: pc.update_last_dropped_ammo,
            last_dropping_direction: pc.last_dropping_direction,
        })
    }

    /// Active-only subtype record for targets, scrolls, nets and projectiles.
    fn subtype(&self) -> Option<projectile_projections::Subtype> {
        if !self.entity.element_data().active {
            return None;
        }
        match self.entity {
            Entity::Target(target) => Some(projectile_projections::Subtype::Target {
                animation: target.target.animation as u32,
                progression: target.target.progression,
                linked_fx: target
                    .target
                    .linked_fx
                    .iter()
                    .copied()
                    .map(typed_entity_reference)
                    .collect::<Vec<_>>(),
                force_display: target.fx.force_display,
                restore_background: target.fx.restore_background,
            }),
            Entity::Scroll(scroll) => Some(projectile_projections::Subtype::Scroll {
                status: self.engine.inner.scroll_status(self.id) as i32,
                script_hourglass_timeout: scroll.script_hourglass_timeout,
            }),
            Entity::Net(net) => Some(projectile_projections::Subtype::Net {
                projectile: projectile_state(&net.projectile),
                victims: net
                    .net
                    .victims
                    .iter()
                    .copied()
                    .map(typed_entity_reference)
                    .collect::<Vec<_>>(),
                time_till_unfolding: net.net.time_till_unfolding,
                crumpled: net.net.crumpled,
                was_flying: net.net.was_flying,
            }),
            Entity::Projectile(projectile) => Some(Self::projectile_subtype(projectile)),
            _ => None,
        }
    }

    fn projectile_subtype(
        projectile: &crate::element::ElementProjectile,
    ) -> projectile_projections::Subtype {
        use crate::element_kinds::ObjectType;
        use projectile_projections::Subtype;
        let common = projectile_state(&projectile.projectile);
        let data = &projectile.projectile;
        match projectile.object.object_type {
            ObjectType::Arrow => Subtype::Arrow {
                projectile: common,
                bow_profile: data.arrow_bow_profile.flatten(),
                flat_shot: data.arrow_flat_shot,
                falling: data.falling,
                falling_direction: data.falling_direction,
                last_sector: data.last_orientation_sector,
                last_azimuth: data.last_orientation_azimuth,
                play_impact: data.arrow_play_impact,
            },
            ObjectType::Purse => Subtype::Purse {
                projectile: common,
                number_of_coins: data.purse.number_of_coins,
            },
            ObjectType::Coin => Subtype::Coin {
                projectile: common,
                source_purse: data.purse.source_purse.map(typed_entity_reference),
            },
            ObjectType::Wasp => Subtype::Wasp {
                nest: data.wasp.source_nest.map(typed_entity_reference),
                victim: data.wasp.victim.map(typed_entity_reference),
                stinging: data.wasp.stinging,
                timeout: data.wasp.timeout,
                movement: point3(
                    data.wasp.movement.x,
                    data.wasp.movement.y,
                    data.wasp.movement.z,
                ),
            },
            ObjectType::WaspNest | ObjectType::BonusWaspNest => Subtype::WaspNest {
                projectile: common,
                flying_wasp_count: data.wasp.flying_wasp_count,
            },
            _ => Subtype::Projectile { projectile: common },
        }
    }
}
