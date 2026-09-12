//! Read-only parity projections, separate from the mutation facade.

use super::*;

#[path = "parity_state/projections.rs"]
mod projections;

#[cfg(test)]
#[path = "parity_state/tests.rs"]
mod tests;

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ParityEntityKind {
    Pc,
    Soldier,
    Civilian,
    Fx,
    Target,
    Bonus,
    Scroll,
    Projectile,
    Net,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ParityEntityReference {
    kind: ParityEntityKind,
    index: u32,
}

fn parity_entity_reference(id: EntityId) -> serde_json::Value {
    serde_json::to_value(typed_entity_reference(id))
        .expect("typed parity entity reference must serialize")
}

fn typed_entity_reference(id: EntityId) -> ParityEntityReference {
    use crate::element::EntityIdKind;
    let kind = match id.kind() {
        EntityIdKind::Pc => ParityEntityKind::Pc,
        EntityIdKind::Soldier => ParityEntityKind::Soldier,
        EntityIdKind::Civilian => ParityEntityKind::Civilian,
        EntityIdKind::Fx => ParityEntityKind::Fx,
        EntityIdKind::Target => ParityEntityKind::Target,
        EntityIdKind::Bonus => ParityEntityKind::Bonus,
        EntityIdKind::Scroll => ParityEntityKind::Scroll,
        EntityIdKind::Projectile => ParityEntityKind::Projectile,
        EntityIdKind::Net => ParityEntityKind::Net,
    };
    ParityEntityReference {
        kind,
        index: id.index(),
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ParityFloat {
    bits: u32,
    value: f32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ParityGameUiState {
    campaign_map: bool,
    campaign_map_displayed: bool,
    post_initialized: bool,
    start_mission_disabled_temp: bool,
    quit_mission_disabled_temp: bool,
    start_mission_enabled: bool,
    quit_mission_enabled: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ParityMessengerState {
    view_locked: bool,
    selected_action: u32,
}

fn parity_float(value: f32) -> serde_json::Value {
    serde_json::to_value(typed_float(value)).expect("typed parity float must serialize")
}

fn typed_float(value: f32) -> ParityFloat {
    ParityFloat {
        bits: value.to_bits(),
        value,
    }
}

fn into_projection_fields(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let serde_json::Value::Object(fields) = value else {
        panic!("parity projection fragment must be an object");
    };
    fields
}

impl Engine {
    /// Complete serialized position and sprite frontier for one entity.
    #[doc(hidden)]
    pub fn parity_entity_runtime_state(
        &self,
        id: EntityId,
        assets: &LevelAssets,
    ) -> serde_json::Value {
        use serde_json::{Value, json};

        let entity_ref = parity_entity_reference;
        let entity = self.inner.world.entities.get(id).unwrap_or_else(|| {
            panic!("parity runtime projection references missing entity {id:?}")
        });
        let sprite = &entity.element_data().sprite;
        let position = sprite.position_iface.v48_serialized_state();
        // Original's runtime frontier has the current mpointSprite projection
        // for ordinary entities. In Rust a map move invalidates the derived
        // cache without overwriting its raw serialized slot, which can still
        // hold the previous top-left. Targets are intentionally different:
        // their authored action point can differ from the visible sprite
        // anchor, so preserve their exact cached value.
        let current_sprite = if entity.is_fx_target() {
            crate::coordinates::SpriteTopLeft::new(position.sprite.x, position.sprite.y)
        } else {
            entity.gameplay_sprite_position()
        };
        let float = typed_float;
        let point2 = |x: f32, y: f32| projections::Point2 {
            x: float(x),
            y: float(y),
        };
        let point3 = |x: f32, y: f32, z: f32| projections::Point3 {
            x: float(x),
            y: float(y),
            z: float(z),
        };
        let jump_line = |index: Option<u32>| -> Value {
            let Some(index) = index else {
                return Value::Null;
            };
            let line = self
                .inner
                .world
                .fast_grid
                .level
                .jump_lines
                .get(usize::try_from(index).expect("parity enemy jump-line index exceeds usize"))
                .unwrap_or_else(|| panic!("parity enemy references missing jump line {index}"));
            json!({
                "a": point2(line.point_a.x, line.point_a.y),
                "b": point2(line.point_b.x, line.point_b.y),
            })
        };
        let bbox = |bbox: crate::coordinates::MapBBox| {
            bbox.0.map(|rect| projections::Bounds2 {
                min: point2(rect.min().x, rect.min().y),
                max: point2(rect.max().x, rect.max().y),
            })
        };
        let sector = |handle: Option<crate::position_interface::SectorHandle>| {
            handle.map(|handle| {
                let level = &self.inner.world.fast_grid.level;
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
        };
        let target = position.target_element.map(typed_entity_reference);
        let door = position.door.map(|door_handle| {
            let index = usize::from(door_handle);
            let door = self
                .inner
                .script_domains
                .interactables
                .doors
                .get(index)
                .unwrap_or_else(|| panic!("parity position references missing door {index}"));
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
        });
        let obstacle = position.obstacle.map(|handle| {
            let handle = usize::from(handle);
            let obstacle = assets
                .environment
                .static_sight_obstacles
                .get(handle)
                .unwrap_or_else(|| {
                    panic!("parity position references missing static obstacle {handle}")
                });
            if let Some(layer) = position.layer {
                let index = assets.environment.static_sight_obstacles[..handle]
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
        });
        if sprite.anims_to_be_replaced.len() != sprite.replacing_anims.len() {
            panic!(
                "sprite replacement list length {} differs from replacement value length {} for {id:?}",
                sprite.anims_to_be_replaced.len(),
                sprite.replacing_anims.len()
            );
        }
        let replacements = sprite
            .anims_to_be_replaced
            .iter()
            .zip(&sprite.replacing_anims)
            .map(|(&from, &to)| projections::AnimationReplacement {
                from: from as u32,
                to: to as u32,
            })
            .collect::<Vec<_>>();

        let position_state = projections::Position {
            computed_position: position.computed_position.bits(),
            computed_increment: position.computed_increment.bits(),
            material: position.material,
            posture: position.posture as u32,
            old_posture: position.old_posture as u32,
            direction: i16::from(position.direction),
            direction_goal: i16::from(position.direction_goal),
            slow_turn_count: position.slow_turn_count,
            direction_count: position.direction_count,
            layer: position.layer.map(crate::position_interface::Layer::get),
            layer_goal: position
                .layer_goal
                .map(crate::position_interface::Layer::get),
            tolerance: float(position.tolerance),
            directional_tolerance: position.directional_tolerance,
            accumulate_movement_map: position.accumulate_movement_map,
            anti_collision_on: position.anti_collision_on,
            goal_next_valid: position.goal_next_valid,
            deviated: position.deviated,
            door_direction: position.door_direction,
            reversed_movement: position.reversed_movement,
            blocked_count: position.blocked_count,
            radius: float(position.radius),
            emergency_lying_box: position.use_emergency_lying_box,
            sector: sector(position.sector),
            sector_goal: sector(position.sector_goal),
            door: door,
            obstacle: obstacle,
            target: target,

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
            move_box: bbox(position.move_box_map),
            blocked_box: bbox(position.blocked_box),
        };
        let sprite_state = projections::Sprite {
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
            replacements: replacements,
        };

        let projectile_state = |projectile: &crate::element::ProjectileData| {
            let trajectory = projectile
                .trajectory
                .iter()
                .enumerate()
                .map(|(index, point)| {
                    let runtime = projectile.trajectory_runtime.get(index);
                    json!({
                        "position": point3(point.position.x, point.position.y, point.position.z),
                        "time": point.time,
                        // `null` is an explicit incomplete runtime mirror, not a fabricated
                        // non-bounce/material value. Fresh trajectory construction must
                        // populate these fields before v29 can pass dynamic-projectile traces.
                        "bounce": runtime.map(|runtime| runtime.bounce),
                        "material": runtime.map(|runtime| runtime.material),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "flying": projectile.flying,
                "dive": projectile.dive,
                "magic_bullet": projectile.magic_bullet,
                "frame_count": projectile.frame_count,
                "trajectory_origin": {
                    "map": point2(projectile.start_of_trajectory_x, projectile.start_of_trajectory_y),
                    "sector": projectile.trajectory_origin_sector,
                    "layer": projectile
                        .trajectory_origin_layer
                        .map(crate::position_interface::Layer::get),
                },
                "flight_direction": projectile.flight_direction,
                "start": point3(projectile.start.x, projectile.start.y, projectile.start.z),
                "end": point3(projectile.end.x, projectile.end.y, projectile.end.z),
                "shooter": projectile.shooter.map_or(Value::Null, entity_ref),
                "trajectory": trajectory,
            })
        };
        let resolve_ai_handle = |handle: u32| -> Value {
            let resolved = self
                .inner
                .world
                .entities
                .occupied()
                .find_map(|(candidate, _)| (candidate.index() == handle).then_some(candidate))
                .unwrap_or_else(|| panic!("parity local AI references missing handle {handle}"));
            entity_ref(resolved)
        };
        let resolve_optional_ai_handle = |handle: Option<crate::ai::AiEntityHandle>| -> Value {
            handle.map_or(Value::Null, |handle| resolve_ai_handle(handle.get()))
        };
        let ai_position = |position: crate::ai::Position| {
            json!({
                "map": point2(position.x, position.y),
                "sector": sector(position.sector),
                "layer": position.level,
            })
        };
        let seek_point = |point: &crate::ai::SeekPoint| {
            json!({
                "position": ai_position(point.position),
                "frame_when_full_interest": point.frame_when_full_interest,
                "directions": &point.directions,
                "last_calculated_interest": point.last_calculated_interest,
                "locked": point.locked,
            })
        };
        let known_strike_command = |strike: Option<crate::weapons::SwordStrike>| -> i32 {
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
        };
        let stimulus_state = |stimulus: &crate::ai::Stimulus| -> Value {
            use crate::ai::{StimulusInfo, StimulusType};
            assert_ne!(
                stimulus.stimulus_type,
                StimulusType::ForceBattleDecision,
                "parity local-AI stimulus contains Rust-only non-serializable type",
            );
            let (info_type, info) = match stimulus.info {
                StimulusInfo::None => (0, json!({ "kind": "none" })),
                StimulusInfo::Noise(noise) => (
                    1,
                    json!({
                        "kind": "noise",
                        "origin": {
                            "map": point2(noise.origin.x, noise.origin.y),
                            "sector": sector(noise.origin.sector),
                            "layer": noise.origin.layer.map(crate::position_interface::Layer::get),
                        },
                        "noise_type": noise.noise_type as u32,
                        "volume": noise.volume, "elevation": noise.elevation,
                    }),
                ),
                StimulusInfo::Position(position) => (
                    2,
                    json!({
                        "kind": "position", "position": ai_position(position),
                    }),
                ),
                StimulusInfo::Human(entity) => (
                    3,
                    json!({
                        "kind": "human", "entity": resolve_ai_handle(entity.get()),
                    }),
                ),
                StimulusInfo::Hint(hint) => (
                    4,
                    json!({
                        "kind": "hint", "position": ai_position(hint.seek_point),
                        "teller": resolve_ai_handle(hint.who_tells_me.get()), "seek_flags": hint.seek_flags,
                    }),
                ),
                StimulusInfo::Object(entity) => (
                    5,
                    json!({
                        "kind": "object", "entity": resolve_ai_handle(entity.get()),
                    }),
                ),
                StimulusInfo::Stolen(stolen) => (
                    6,
                    json!({
                        "kind": "stolen", "object": resolve_ai_handle(stolen.object.get()),
                        "thief": resolve_ai_handle(stolen.thief.get()),
                    }),
                ),
                StimulusInfo::Combat(combat) => (
                    7,
                    json!({
                        "kind": "combat", "actor": resolve_ai_handle(combat.actor_npc.get()),
                        "enemy_position": ai_position(combat.enemy_position),
                    }),
                ),
                StimulusInfo::DoorCombat(combat) => (
                    8,
                    json!({
                        "kind": "door_combat", "delay": combat.delay, "direction": combat.direction,
                        "goal": ai_position(combat.goal),
                        "adversary": resolve_optional_ai_handle(combat.adversary),
                    }),
                ),
                StimulusInfo::Index(value) => (9, json!({ "kind": "index", "value": value })),
                StimulusInfo::LegacyInvalidType(raw) => {
                    panic!("parity local-AI stimulus retains active invalid type word {raw}")
                }
            };
            json!({
                "stimulus_type": stimulus.stimulus_type as u32,
                "info_type": info_type,
                "owner": resolve_optional_ai_handle(stimulus.owner),
                "to_whole_patrol": stimulus.to_whole_patrol,
                "info": info,
            })
        };
        let patrol_stimulus = |stimulus: Option<&crate::ai::Stimulus>| -> Value {
            use crate::ai::{StimulusInfo, StimulusType};
            let Some(stimulus) = stimulus else {
                return Value::Null;
            };
            let is_default = stimulus.stimulus_type == StimulusType::NoEvent
                && matches!(
                    stimulus.info,
                    StimulusInfo::None | StimulusInfo::LegacyInvalidType(_)
                )
                && stimulus.owner.is_none()
                && !stimulus.to_whole_patrol;
            if is_default {
                Value::Null
            } else {
                stimulus_state(stimulus)
            }
        };
        let npc_ai = entity.npc_data().and_then(|npc| {
            let ai = npc.ai_brain.base()?;
            let ai_door = |index: Option<crate::gate::DoorIndex>| -> Value {
                let Some(index) = index else { return Value::Null };
                let door = self
                    .inner
                    .script_domains
                    .interactables
                    .doors
                    .get(usize::from(index))
                    .unwrap_or_else(|| panic!("parity AI references missing door {index}"));
                let kind = match door.gate_type {
                    crate::gate::GateType::Door => "door",
                    crate::gate::GateType::Jump => "jump",
                    crate::gate::GateType::None => "gate",
                };
                json!({
                    "kind": kind,
                    "sector_out": door.sector_out.get(),
                    "sector_in": door.sector_in.get(),
                    "layer_out": door.layer_out,
                    "layer_in": door.layer_in,
                    "point_out": point2(door.point_out.x, door.point_out.y),
                    "point_in": point2(door.point_in.x, door.point_in.y),
                })
            };
            let handles = |values: &[u32]| {
                values
                    .iter()
                    .copied()
                    .map(resolve_ai_handle)
                    .collect::<Vec<_>>()
            };
            let patrol_path_status = if let Some(path) = &ai.patrol_path {
                json!({
                    "current_waypoint_index": path.current_waypoint_index,
                    "last_waypoint_index": path.last_waypoint_index,
                    "forward": path.forward,
                    "hiking_path_index": path.hiking_path_index.get(),
                    "history": path.history.iter().map(|entry| json!({
                        "position": ai_position(entry.position), "direction": entry.direction,
                        "distance": entry.distance,
                    })).collect::<Vec<_>>(),
                })
            } else {
                let path = &ai.detached_patrol_path_status;
                json!({
                    "current_waypoint_index": path.current_waypoint_index,
                    "last_waypoint_index": path.last_waypoint_index,
                    "forward": path.forward,
                    "hiking_path_index": path.hiking_path_index.map_or(Value::Null, |id| json!(id.get())),
                    "history": path.history.iter().map(|entry| json!({
                        "position": ai_position(entry.position), "direction": entry.direction,
                        "distance": entry.distance,
                    })).collect::<Vec<_>>(),
                })
            };
            let mut state = json!({
                "last_goto": {
                    "destination": ai_position(ai.last_goto_destination),
                    "flags": ai.last_goto_flags.bits(), "stuck_counter": ai.stuck_counter,
                },
                "forbidden_remarks": ai.forbidden_remark_ids,
                "current_remark_flags": ai.current_remark_flags,
                "owner": ai.owner_entity_id.map_or(Value::Null, entity_ref),
                "state": ai.current_state as u32, "old_state": ai.old_state,
                "substate": ai.current_substate as u32,
                "music_alert": ai.current_music_alert_status as u32,
                "timer_launch_substate": ai.substate_at_last_timer_launch as u32,
                "attitude": ai.attitude as u32, "blood_alcohol": ai.blood_alcohol,
                "initial_action": ai.initial_action, "number_of_looks": ai.number_of_looks,
                "can_move": ai.can_move,
                "path_control": {
                    "stop_before_end": ai.stop_before_end_of_path,
                    "use_max_norm": ai.use_max_norm_to_stop_before_end_of_path,
                    "stop_distance": ai.stop_before_end_of_path_distance,
                    "status": patrol_path_status,
                    "has_patrol_path": ai.has_patrol_path,
                    "macro_cursor": ai.has_patrol_path.then_some(ai.macro_command_offset),
                },
                "macro": {
                    "remaining_bytes": ai.number_of_remaining_macro_bytes,
                    "in_progress": ai.macro_in_progress,
                    "started_this_frame": ai.macro_started_in_this_frame,
                    "next_rand": ai.next_macro_rand,
                    "next_rand_forecasted": ai.next_macro_rand_forecasted,
                },
                "targets": {
                    "primary": resolve_optional_ai_handle(ai.primary_target),
                    "friend_in_trouble": resolve_optional_ai_handle(ai.friend_in_trouble),
                    "detected_body": resolve_optional_ai_handle(ai.detected_body),
                    "interesting_object": resolve_optional_ai_handle(ai.interesting_object),
                    "antagonist": resolve_optional_ai_handle(ai.antagonist),
                    "last_stimulus_actor": resolve_optional_ai_handle(ai.last_stimulus_actor),
                },
                "timers": {
                    "running": ai.timer_is_running, "ring": ai.when_does_timer_ring,
                    "macro_running": ai.macro_timer_is_running,
                    "macro_ring": ai.when_does_macro_timer_ring,
                    "standing_around": ai.standing_around_timer,
                },
            });
            state
                .as_object_mut()
                .expect("parity NPC AI state must be an object")
                .extend(into_projection_fields(json!({
                "sorrow": ai.sorrow_level,
                "last_stimuli": ai.last_stimulus.map(|stimulus| stimulus as u32),
                "last_stimulus_multiplicities": ai.last_stimulus_multiplicity,
                "group": {
                    "is_master": ai.is_master, "master": resolve_optional_ai_handle(ai.master),
                    "us": handles(&ai.list_us), "alerted_us": handles(&ai.list_alerted_us),
                    "staying_us": handles(&ai.list_staying_us),
                },
                "seek_position": ai_position(ai.seek_position),
                "alert_soldiers_point": ai_position(ai.alert_soldiers_point),
                "first_try": ai.first_try,
                "panic": {
                    "center": point2(ai.panic_center_x, ai.panic_center_y),
                    "lasting_runs": ai.lasting_panic_runs, "directed": ai.directed_panic,
                },
                "movement_failures": {
                    "could_not_reach": ai.couldnt_reachpoint,
                    "already_on_point": ai.already_on_point, "already_turned": ai.already_turned,
                },
                "likes_to_sit": ai.likes_to_sit_around, "special_action": ai.special_action,
                "friends_alerted": ai.friends_are_alerted, "stay_at_home": ai.is_stay_at_home,
                "locks": ai.locks_flag_field.bits(), "was_busy": ai.was_busy,
                "stimulus_queue": ai.stimulus_queue.iter().map(stimulus_state).collect::<Vec<_>>(),
                "script_locked": ai.script_locked, "remember_events": ai.remember_events,
                "leave_house_number": ai.leave_house_number,
                "legacy_continuation": {
                    "remaining_tequila_gulps": ai.remaining_tequila_gulps,
                    "last_hint_actuality": ai.last_hint_actuality,
                    "last_hint_subject": ai.last_hint_subject as u32,
                    "current_door": ai_door(ai.my_door_index),
                    "looking_for_help_because_enemy_seen": ai.looking_for_help_because_enemy_seen,
                },
                "object_memory": {
                    "forgotten": handles(&ai.forgotten_objects),
                    "desire": resolve_optional_ai_handle(ai.object_of_desire),
                    "checkpoint_charly": resolve_optional_ai_handle(ai.checkpoint_charly),
                    "synchronize_charly": resolve_optional_ai_handle(ai.synchronize_charly),
                },
                "inside_halt": ai.inside_halt_method,
                "synchronizing_actors": handles(&ai.synchronizing_actors),
                "default_path_flags": ai.default_path_walking_flags.bits(),
                })));
            state
                .as_object_mut()
                .expect("parity NPC AI state must be an object")
                .extend(into_projection_fields(json!({
                "current_remark": ai.current_remark as u32,
                "emoticon": {
                    "type": ai.current_emoticon_type as u32,
                    "expiration": ai.emoticon_expiration_date,
                    "has_expiration": ai.emoticon_has_expiration_date,
                },
                "knocked_out_in_money_fight": ai.knocked_out_in_money_fight,
                "got_beggar_trick": ai.got_the_beggar_trick,
                "reconnaissance": {
                    "report_type": ai.my_reconnaissance_report.report_type as u32,
                    "seek_position": ai_position(ai.my_reconnaissance_report.seek_position),
                    "seen_bodies": handles(&ai.my_reconnaissance_report.seen_bodies),
                    "charly": resolve_optional_ai_handle(ai.my_reconnaissance_report.charly),
                    "charly_seen": ai.my_reconnaissance_report.charly_seen,
                },
                "patrol": {
                    "chief": ai.patrol_chief.map_or(Value::Null, entity_ref),
                    "active": ai.patrol.iter().copied().map(entity_ref).collect::<Vec<_>>(),
                    "missed": ai.missed_patrol_members.iter().copied().map(entity_ref).collect::<Vec<_>>(),
                    "theoretical": ai.theoretical_patrol.iter().copied().map(entity_ref).collect::<Vec<_>>(),
                    "stopped": ai.patrol_stopped, "direction": ai.patrol_direction,
                },
                })));
            let subclass = match &npc.ai_brain {
                crate::element::AiBrain::Friendly(friendly) => Some(json!({
                    "kind": "friendly",
                    "fleeing_seen_enemy_counter": friendly.fleeing_seen_enemy_counter,
                    "beggar_dont_talk_counter": friendly.beggar_dont_talk_counter,
                    "wants_to_talk": friendly.wants_to_talk,
                    "last_talk_partner": resolve_optional_ai_handle(friendly.last_talk_partner),
                    "can_go_away": friendly.can_go_away,
                })),
                crate::element::AiBrain::Enemy(enemy) => {
                    let mut subclass = json!({
                    "kind": "enemy",
                    "frame_when_missed_charly": enemy.frame_when_missed_charly,
                    "frame_when_enemy_detected": enemy.base.frame_when_enemy_detected,
                    "fleeing_seen_enemy_counter": enemy.fleeing_seen_enemy_counter,
                    "pc_gone_direction": enemy.pc_gone_away_in_this_direction,
                    "detected_something_there": ai_position(enemy.detected_something_there),
                    "missed_pc": resolve_optional_ai_handle(enemy.missed_pc),
                    "last_seek_direction_index": enemy.last_seek_direction_index,
                    "beggar_to_examine": resolve_optional_ai_handle(enemy.beggar_to_examine),
                    "pc_missed": enemy.pc_missed,
                    "task_priorities": {
                        "current": enemy.current_task_priority,
                        "minimal": enemy.minimal_task_priority,
                        "new": enemy.new_task_priority,
                    },
                    "different_checkpoints": enemy.number_of_different_checkpoints,
                    "delta_sorrow": enemy.base.delta_sorrow_level,
                    "thirsty": enemy.thirsty,
                    "old_life_points": enemy.old_life_points,
                    "initial_life_points": enemy.initial_life_points,
                    "old_odds": enemy.old_odds,
                    "position_change_locked_for_test": enemy.position_change_locked_for_test,
                    "heard_nets": handles(&enemy.heard_nets),
                    "other_seen_ale": handles(&enemy.other_seen_ale),
                    "search_charly_way": enemy.search_charly_way.iter().copied().map(ai_position).collect::<Vec<_>>(),
                    "missed_in_action": handles(&enemy.base.missed_in_action),
                    "other_bodies_to_examine": handles(&enemy.other_bodies_to_examine),
                    "beggars_to_control": handles(&enemy.beggars_to_control),
                    "them": handles(&enemy.list_them),
                    "ambush_point_array_reset": enemy.ambush_point_array_reset,
                    "ambush_point_status": enemy.ambush_point_status.iter().map(|status| *status as u32).collect::<Vec<_>>(),
                    "my_seek_points": &enemy.my_seek_points,
                    "personal_seek_point_1": enemy.personal_seek_point_1.as_ref().map(&seek_point),
                    "personal_seek_point_2": enemy.personal_seek_point_2.as_ref().map(seek_point),
                    "seek_center": ai_position(enemy.seek_center),
                    "actual_seek_point": enemy.actual_seek_point,
                    "seek_point_view_directions": &enemy.seek_point_view_directions,
                    "positions_of_beggars_to_control": enemy.positions_of_beggars_to_control.iter().copied().map(ai_position).collect::<Vec<_>>(),
                    "seek_flags": enemy.seek_flags.bits(),
                    "seen_dead_body": enemy.seen_dead_body,
                    "seeking_charly": enemy.seeking_charly,
                    });
                    subclass
                        .as_object_mut()
                        .expect("parity enemy AI state must be an object")
                        .extend(into_projection_fields(json!({
                    "forced_next_battle_decision": enemy.forced_next_battle_decision as u32,
                    "reset_battle_decision": enemy.reset_battle_decision,
                    "synchronize_index": enemy.base.synchronize_index,
                    "initial_view_cone": enemy.base.initial_view_cone as u32,
                    "company_number": enemy.company_number,
                    "left_combat_neighbour": resolve_optional_ai_handle(enemy.left_combat_neighbour),
                    "right_combat_neighbour": resolve_optional_ai_handle(enemy.right_combat_neighbour),
                    "attentive": enemy.attentive,
                    "will_be_attentive": enemy.will_be_attentive,
                    "forced_attentive": enemy.forced_attentive,
                    "guarded_pc": enemy.guarded_pc.map_or(Value::Null, |id| entity_ref(EntityId::Pc(id))),
                    "tower_guard": enemy.tower_guard,
                    "combat_trainer": enemy.combat_trainer,
                    "gather_position": ai_position(enemy.gather_position),
                    "gather_direction": enemy.gather_direction,
                    "gather_position_instructed": enemy.gather_position_instructed,
                    "officers_position": ai_position(enemy.officers_position),
                    "previous_state": enemy.previous_state,
                    "previous_substate": enemy.previous_substate,
                    "reported_to_officer": enemy.reported_to_officer,
                    "missed_soldier_timer": enemy.missed_soldier_timer,
                    "old_money": enemy.old_money,
                    "other_seen_money": handles(&enemy.other_seen_money),
                    "money_fight_enemies": handles(&enemy.money_fight_enemies),
                    "money_fight_victims": handles(&enemy.money_fight_victims),
                    "archer_behind_me": resolve_optional_ai_handle(enemy.archer_behind_me),
                    "shield_bearer_before_me": resolve_optional_ai_handle(enemy.shield_bearer_before_me),
                    "already_seen_bodies": handles(&enemy.already_seen_bodies),
                    "my_line_jump": jump_line(enemy.my_line_jump),
                    "shield_bearer_direction": enemy.shield_bearer_direction,
                    "phalanx_aborted": enemy.phalanx_aborted,
                    "changed_to_alert_path": enemy.changed_to_alert_path,
                    })));
                    subclass
                        .as_object_mut()
                        .expect("parity enemy AI state must be an object")
                        .extend(into_projection_fields(json!({
                    "shooting_point": enemy.my_shooting_point.map(|(sector_index, point_index)| json!({
                        "sector_index": sector_index, "point_index": point_index,
                    })),
                    "archery_sector": enemy.my_archery_sector,
                    "archery_sector_index": enemy.my_archery_sector_index,
                    "archery_point_index": enemy.my_archery_point_index.0,
                    "archery_point_increment": enemy.my_archery_point_increment,
                    "enemy_seen_below": enemy.enemy_seen_below,
                    "enemy_had_this_elevation": enemy.enemy_had_this_elevation,
                    "known_enemy_strike_commands": [
                        known_strike_command(enemy.known_enemy_strike_1),
                        known_strike_command(enemy.known_enemy_strike_2),
                        known_strike_command(enemy.known_enemy_strike_3),
                    ],
                    "last_stimulus_dispatched_to_patrol": patrol_stimulus(enemy.last_stimulus_dispatched_to_patrol.as_ref()),
                    })));
                    Some(subclass)
                },
                crate::element::AiBrain::None => None,
            };
            if let Some(subclass) = subclass {
                state
                    .as_object_mut()
                    .expect("parity NPC AI state must be an object")
                    .insert(
                        "subclass".to_owned(),
                        subclass,
                    );
            }
            Some(state)
        });
        let human_continuation = entity.human_data().map(|human| {
            json!({
                "already_detectable_body": human.already_detectable_body,
                "concussion_healing_timeout": human.concussion_healing_timeout,
                "tiredness": human.tiredness,
                "concussion": human.concussion_of_the_brain,
                "parry_counter": human.parry_counter,
                "detectable_list_index": human.detectable_list_index,
                "invulnerable": human.invulnerable,
                "last_motion_was_step_back": human.last_motion_was_step_back_in_combat,
                "smalltalk_initiative": human.smalltalk_initiative,
                "received_smalltalk_initiative": human.received_smalltalk_initiative,
                "smalltalk_hint": human.smalltalk_hint as u32,
                "smalltalk_hint_opponent": human.smalltalk_hint_opponent.map_or(Value::Null, entity_ref),
                "relative_fighting_ability": human.relative_fighting_ability,
                "hollow_man": human.hollow_man,
                "killed_by_accident": human.killed_by_accident,
                "stuck_under_nets_counter": human.stuck_under_nets_counter,
                "sword_strike_boredom": &human.sword_strike_boredom,
                "carrier": human.carrier.map_or(Value::Null, entity_ref),
                "small_repulsive_radius": human.small_repulsive_radius,
                "hulk": {
                    "running": human.running_hulk, "time": human.time_hulk,
                    "level": human.hulk_level, "direction": human.hulk_direction,
                    "speed": float(human.hulk_speed),
                },
            })
        });
        let human_structure = entity.human_data().map(|human| {
            let opponents = human
                .opponents
                .iter_with_jump_lines()
                .map(|(opponent, line)| json!({
                    "entity": entity_ref(opponent), "jump_line": jump_line(line.map(u32::from)),
                }))
                .collect::<Vec<_>>();
            let repulsive = &human.repulsive_point;
            let shield = &human.shield;
            let plane = |value: &crate::element::HumanPlaneState| json!({
                "a": point3(value.a.x, value.a.y, value.a.z),
                "b": point3(value.b.x, value.b.y, value.b.z),
                "normal": point3(value.normal.x, value.normal.y, value.normal.z),
                "origin": point3(value.origin.x, value.origin.y, value.origin.z),
                "u": point3(value.u.x, value.u.y, value.u.z),
                "v": point3(value.v.x, value.v.y, value.v.z),
                "az": float(value.az), "bz": float(value.bz),
                "dz": float(value.dz), "d": float(value.d),
            });
            let box2_state = |value: crate::element::HumanBoundingBox2State| json!({
                "top_left": point2(value.top_left.x, value.top_left.y),
                "bottom_right": point2(value.bottom_right.x, value.bottom_right.y),
                "bounds_are_set": value.bounds_are_set,
            });
            let sequence_ordinals: std::collections::BTreeMap<_, _> = self
                .inner
                .orders
                .sequence_manager
                .sequences_iter()
                .enumerate()
                .map(|(ordinal, sequence)| (sequence.id, ordinal))
                .collect();
            let sequence_ref = |value: crate::sequence::SequenceElementRef| {
                let sequence = sequence_ordinals.get(&value.sequence_id).copied().unwrap_or_else(|| {
                    panic!("parity human pending shoot points outside sequence manager: {value:?}")
                });
                json!({ "sequence": sequence, "element": value.element_index })
            };
            json!({
                "opponents": opponents,
                "repulsive_point": {
                    "position": point2(repulsive.position.x, repulsive.position.y),
                    "concave": repulsive.concave,
                    "limit_left": point2(repulsive.limit_left.x, repulsive.limit_left.y),
                    "limit_right": point2(repulsive.limit_right.x, repulsive.limit_right.y),
                    "action_radius": float(repulsive.action_radius),
                    "force_a": float(repulsive.force_a), "force_b": float(repulsive.force_b),
                    "radius": float(repulsive.radius), "id": repulsive.id,
                    "affects_pcs": repulsive.affects_pcs,
                    "affects_soldiers": repulsive.affects_soldiers,
                    "affects_civilians": repulsive.affects_civilians,
                    "affects_animals": repulsive.affects_animals,
                },
                "building": sector(human.building_sector),
                "shield": {
                    "points": shield.points.iter().map(|value| json!({
                        "obstacle": value.obstacle.map(float),
                        "polygon": point2(value.polygon.x, value.polygon.y),
                    })).collect::<Vec<_>>(),
                    "top_plane": plane(&shield.top_plane),
                    "bottom_plane": plane(&shield.bottom_plane),
                    "box_3d": shield.box_3d.map(float),
                    "ground_box": box2_state(shield.ground_box),
                    "screen_box": box2_state(shield.screen_box),
                    "on_ground": shield.on_ground,
                },
                "sword_sweep": {
                    "victims": human.sword_sweep.victims.iter().copied().map(entity_ref).collect::<Vec<_>>(),
                    "initial_angle": float(human.sword_sweep.initial_angle),
                    "current_angle": float(human.sword_sweep.current_angle),
                    "final_angle": float(human.sword_sweep.final_angle),
                },
                "pending_shoots": human.pending_shoots.iter().copied().map(sequence_ref).collect::<Vec<_>>(),
            })
        });
        let pc_core = entity.pc_data().map(|pc| {
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
            json!({
                "work_icon": pc.work_icon as u32,
                "campaign_description_index": campaign_description_index,
                "playable": pc.playable,
                "beam_me_index": pc.beam_me_index,
                "already_selected": pc.already_selected,
                "belt_seen": pc.belt_seen,
                "feet_seen": pc.feet_seen,
                "head_seen": pc.head_seen,
                "immortal": pc.immortal,
                "fried_psykokwack": pc.fried_psykokwack,
                "list_index": pc.list_index,
                "teleport_counter": pc.teleport_counter,
                "current_action": pc.current_action as u32,
                "saved_action": pc.saved_action as u32,
                "disabled_actions": pc.disabled_actions,
                "disabled_actions_temp": pc.disabled_actions_temp,
                "position_before_teleport": point2(
                    pc.position_before_teleport.x,
                    pc.position_before_teleport.y,
                ),
            })
        });
        let pc_qa = entity.pc_data().map(|pc| {
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
                .map(|slot| {
                    json!({
                        "special_count": pc.quick_action_special_counts[slot],
                        "quickito": pc.quick_action_types[slot] as u32,
                        "titbit": pc.titbits[slot].map(crate::titbit::TitbitId::get),
                        "button": pc.quick_action_buttons[slot],
                        "interactor": pc.quick_action_interactors[slot].map_or(Value::Null, entity_ref),
                        "action_size": pc.quick_action_sequences[slot].as_ref().map(|sequence| sequence.len()),
                        "seek_size": pc.quick_seek_sequences[slot].as_ref().map(|sequence| sequence.len()),
                    })
                })
                .collect::<Vec<_>>()
        });
        let pc_interface = entity.pc_data().map(|pc| {
            json!({
                "playable": pc.playable,
                "displayed": !pc.interface_hidden,
            })
        });
        let pc_portrait = entity.pc_data().map(|pc| {
            let profile = assets
                .profile_manager
                .get_character(pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "PC {id:?} portrait has missing profile {}",
                        pc.profile_index
                    )
                });
            let description = self
                .inner
                .pc_description_for_pc_data(pc)
                .unwrap_or_else(|| panic!("PC {id:?} portrait has no campaign description"));
            let quantities = profile
                .actions
                .map(|action| description.status.get_ammo(action));
            json!({
                "quantities": quantities,
                "two_buttons_mode": profile.actions[2] == crate::profiles::Action::NoAction,
                "displayed": !pc.interface_hidden,
                "burned": pc.portrait.burned,
                "open": pc.portrait.open,
                "life_level": float(f32::from(pc.life_points)),
                "trumpet_enabled": pc.trumpet_enabled,
                "quick_icons": pc.portrait.quick_icons.iter().map(|icon| json!({
                    "titbit": icon.titbit_id.map(crate::titbit::TitbitId::get),
                    "running": icon.running,
                })).collect::<Vec<_>>(),
            })
        });
        let pc_tail = entity.pc_data().map(|pc| {
            json!({
                "carried": pc.carried.map_or(Value::Null, entity_ref),
                "carried_posture": pc.carried_posture,
                "shield_danger_point": point3(
                    pc.shield_danger_point.x,
                    pc.shield_danger_point.y,
                    pc.shield_danger_point.z,
                ),
                "shield_protected": pc.shield_protected.map_or(Value::Null, entity_ref),
                "shield_protector": pc.shield_protector.map_or(Value::Null, entity_ref),
                "guard": pc.guard.map_or(Value::Null, entity_ref),
                "time_till_reinforcement": pc.time_till_reinforcement,
                "last_ammo_dropping_position": point2(
                    pc.last_ammo_dropping_position.x,
                    pc.last_ammo_dropping_position.y,
                ),
                "last_dropped_ammo": pc.last_dropped_ammo.map_or(Value::Null, entity_ref),
                "update_last_dropped_ammo": pc.update_last_dropped_ammo,
                "last_dropping_direction": pc.last_dropping_direction,
            })
        });
        let subtype = if entity.element_data().active {
            match entity {
                crate::element::Entity::Target(target) => Some(json!({
                    "kind": "target",
                    "animation": target.target.animation as u32,
                    "progression": target.target.progression,
                    "linked_fx": target.target.linked_fx.iter().copied().map(entity_ref).collect::<Vec<_>>(),
                    "force_display": target.fx.force_display,
                    "restore_background": target.fx.restore_background,
                })),
                crate::element::Entity::Scroll(scroll) => Some(json!({
                    "kind": "scroll",
                    "status": self.inner.scroll_status(id) as i32,
                    "script_hourglass_timeout": scroll.script_hourglass_timeout,
                })),
                crate::element::Entity::Net(net) => Some(json!({
                    "kind": "net",
                    "projectile": projectile_state(&net.projectile),
                    "victims": net.net.victims.iter().copied().map(entity_ref).collect::<Vec<_>>(),
                    "time_till_unfolding": net.net.time_till_unfolding,
                    "crumpled": net.net.crumpled,
                    "was_flying": net.net.was_flying,
                })),
                crate::element::Entity::Projectile(projectile) => {
                    use crate::element_kinds::ObjectType;
                    let common = projectile_state(&projectile.projectile);
                    Some(match projectile.object.object_type {
                        ObjectType::Arrow => json!({
                            "kind": "arrow", "projectile": common,
                            "bow_profile": projectile.projectile.arrow_bow_profile.flatten(),
                            "flat_shot": projectile.projectile.arrow_flat_shot,
                            "falling": projectile.projectile.falling,
                            "falling_direction": projectile.projectile.falling_direction,
                            "last_sector": projectile.projectile.last_orientation_sector,
                            "last_azimuth": projectile.projectile.last_orientation_azimuth,
                            "play_impact": projectile.projectile.arrow_play_impact,
                        }),
                        ObjectType::Purse => json!({
                            "kind": "purse", "projectile": common,
                            "number_of_coins": projectile.projectile.purse.number_of_coins,
                        }),
                        ObjectType::Coin => json!({
                            "kind": "coin", "projectile": common,
                            "source_purse": projectile.projectile.purse.source_purse.map_or(Value::Null, entity_ref),
                        }),
                        ObjectType::Wasp => json!({
                            "kind": "wasp",
                            "nest": projectile.projectile.wasp.source_nest.map_or(Value::Null, entity_ref),
                            "victim": projectile.projectile.wasp.victim.map_or(Value::Null, entity_ref),
                            "stinging": projectile.projectile.wasp.stinging,
                            "timeout": projectile.projectile.wasp.timeout,
                            "movement": point3(projectile.projectile.wasp.movement.x,
                                projectile.projectile.wasp.movement.y, projectile.projectile.wasp.movement.z),
                        }),
                        ObjectType::WaspNest | ObjectType::BonusWaspNest => json!({
                            "kind": "wasp_nest", "projectile": common,
                            "flying_wasp_count": projectile.projectile.wasp.flying_wasp_count,
                        }),
                        _ => json!({ "kind": "projectile", "projectile": common }),
                    })
                }
                _ => None,
            }
        } else {
            None
        };

        serde_json::to_value(projections::EntityRuntime {
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
        })
        .expect("typed entity parity envelope must serialize")
    }
    /// Read-only schema-13 parity view of gameplay-authoritative global state.
    #[doc(hidden)]
    pub fn parity_engine_state(&self) -> ParityEngineState {
        let mission = &self.inner.mission_domain.state;
        let seat = &self.inner.players.seats[0];
        ParityEngineState {
            cheat_used_flags: self.inner.mission_domain.cheat_used_flags,
            next_creation_order: self.inner.world.next_original_creation_order,
            chorus_timer: self.inner.control.chorus_timer,
            force_check: self.inner.script_domains.mission_ui.force_check,
            men_to_blazon_conversion: self
                .inner
                .script_domains
                .mission_ui
                .men_to_blazon_conversion_mode,
            lock_engine: self.inner.control.simulation_gates.engine_locked(),
            freeze_all: self.inner.control.simulation_gates.actors_frozen(),
            locker: seat.locker_active,
            speed: self.inner.control.speed,
            speed_int: self.inner.control.speed_int,
            mission_won: mission.mission_won,
            mission_won_first_time: mission.mission_won_first_time,
            quit_won: mission.quit_won,
            quit_lost: mission.quit_lost,
            quit_interrupted: mission.quit_interrupted,
            script_globals: self.inner.scripts.globals.clone(),
        }
    }

    /// Exact serialized game mission/controller latches. Host widgets
    /// mirror these values but do not own their authoritative state.
    #[doc(hidden)]
    pub fn parity_game_ui_state(&self) -> serde_json::Value {
        let ui = &self.inner.script_domains.mission_ui;
        serde_json::to_value(ParityGameUiState {
            campaign_map: ui.campaign_map,
            campaign_map_displayed: ui.campaign_map_displayed,
            post_initialized: ui.game_post_initialized,
            start_mission_disabled_temp: ui.start_mission_disabled_temp,
            quit_mission_disabled_temp: ui.quit_mission_disabled_temp,
            start_mission_enabled: ui.start_mission_enabled,
            quit_mission_enabled: ui.quit_mission_enabled,
        })
        .expect("typed parity UI state must serialize")
    }

    /// Serialized messenger controller state that remains gameplay-visible.
    #[doc(hidden)]
    pub fn parity_messenger_controller_state(&self) -> serde_json::Value {
        serde_json::to_value(ParityMessengerState {
            view_locked: self.inner.players.view_locked,
            selected_action: self.inner.players.seats[0].selected_action as u32,
        })
        .expect("typed parity messenger state must serialize")
    }

    /// Serialized engine-global two-click shield controller. This is separate
    /// from each PC's active shield links in `pc_tail`.
    #[doc(hidden)]
    pub fn parity_shield_controller_state(&self) -> serde_json::Value {
        let entity = |id: EntityId| {
            assert!(
                matches!(id.kind(), crate::element::EntityIdKind::Pc),
                "shield controller protects non-PC entity {:?}",
                id.kind()
            );
            typed_entity_reference(id)
        };
        let shield = &self.inner.world.shield;
        let bits = |value: f32| projections::FloatBits {
            bits: value.to_bits(),
        };
        serde_json::to_value(projections::ShieldController {
            is_protected: shield.is_protected,
            protected_pc: shield.protected_pc.map(entity),
            danger_point: projections::Point3Bits {
                x: bits(shield.danger_point.x),
                y: bits(shield.danger_point.y),
                z: bits(shield.danger_point.z),
            },
        })
        .expect("typed shield controller parity must serialize")
    }

    /// Canonical manager-insertion-ordered sequence state for schema-13
    /// Original parity. Runtime allocation IDs are deliberately replaced by
    /// `(sequence ordinal, element index)` references.
    #[doc(hidden)]
    pub fn parity_sequence_manager_state(&self) -> serde_json::Value {
        use crate::sequence::{Field, FieldValue, SequenceElementData};
        use serde_json::{Value, json};

        let float = |value: f32| json!({ "bits": value.to_bits() });
        let point = |x: f32, y: f32| json!({ "x": float(x), "y": float(y) });
        let point3 =
            |x: f32, y: f32, z: f32| json!({ "x": float(x), "y": float(y), "z": float(z) });
        let entity = parity_entity_reference;
        let doors = &self.inner.script_domains.interactables.doors;
        let gate = |id: Option<crate::gate::DoorIndex>| -> Value {
            let Some(id) = id else { return Value::Null };
            let door = doors
                .get(usize::from(id))
                .unwrap_or_else(|| panic!("parity sequence references missing door {id}"));
            let kind = match door.gate_type {
                crate::gate::GateType::Door => "door",
                crate::gate::GateType::Jump => "jump",
                crate::gate::GateType::None => "gate",
            };
            json!({
                "kind": kind,
                "sector_out": door.sector_out.get(),
                "sector_in": door.sector_in.get(),
                "layer_out": door.layer_out,
                "layer_in": door.layer_in,
                "point_out": point(door.point_out.x, door.point_out.y),
                "point_in": point(door.point_in.x, door.point_in.y),
            })
        };
        let lines = &self.inner.world.fast_grid.level.jump_lines;
        let line = |id: Option<crate::jump_line::JumpLineIndex>| -> Value {
            let Some(id) = id else { return Value::Null };
            let line = lines
                .get(usize::from(id))
                .unwrap_or_else(|| panic!("parity sequence references missing jump line {id}"));
            json!({
                "a": point(line.point_a.x, line.point_a.y),
                "b": point(line.point_b.x, line.point_b.y),
            })
        };

        let manager = &self.inner.orders.sequence_manager;
        let sequence_ordinals: std::collections::BTreeMap<_, _> = manager
            .sequences_iter()
            .enumerate()
            .map(|(ordinal, sequence)| (sequence.id, ordinal))
            .collect();
        let reference = |id: crate::sequence::SequenceId, element: usize| {
            let sequence = sequence_ordinals.get(&id).copied().unwrap_or_else(|| {
                panic!("parity sequence reference points outside manager: {id:?}/{element}")
            });
            json!({ "sequence": sequence, "element": element })
        };

        let mut sequences = Vec::new();
        for sequence in manager.sequences_iter() {
            let (cursor, current_level, running, in_progress, started) = sequence.parity_counters();
            let mut elements = Vec::new();
            for element_state in &sequence.elements {
                let orders: Vec<_> = element_state
                    .orders
                    .iter()
                    .map(|order| {
                        json!({
                            "action": order.order_type as u32,
                            "destination": point(order.target_x, order.target_y),
                            "destination_3d": point3(
                                order.destination_3d[0],
                                order.destination_3d[1],
                                order.destination_3d[2],
                            ),
                            "flight_vector": point(
                                order.flight_vector[0],
                                order.flight_vector[1],
                            ),
                            "tolerance": float(order.tolerance),
                            "apply_transition": order.apply_transition_at_this_point,
                            "reverse": order.reverse,
                            "compute_direction": order.compute_direction,
                            "can_fly": order.can_fly,
                            "lock_ai": order.lock_ai,
                            "transition": order.transition,
                            "done": order.done,
                            "id": order.order_id.get() - 1,
                            "antagonist": order.antagonist.map(&entity).unwrap_or(Value::Null),
                        })
                    })
                    .collect();

                let subtype = match &element_state.data {
                    SequenceElementData::Simple => json!({ "kind": "simple" }),
                    SequenceElementData::Interaction { antagonist } => json!({
                        "kind": "interaction",
                        "antagonist": antagonist.map(&entity).unwrap_or(Value::Null),
                    }),
                    SequenceElementData::Damage {
                        origin,
                        projectile,
                        damage,
                        concussion,
                        sword_strike,
                        is_harder_hit,
                        ..
                    } => json!({
                        "kind": "damage",
                        "origin": origin.map(&entity).unwrap_or(Value::Null),
                        "damage": damage,
                        "concussion": concussion,
                        "harder_hit": is_harder_hit,
                        "sword_strike": sword_strike.map(|strike| strike as i32).unwrap_or(11),
                        "arrow": projectile.map(&entity).unwrap_or(Value::Null),
                    }),
                    SequenceElementData::Movement {
                        destination,
                        layer,
                        sector,
                        gate_id,
                        line_id,
                        element,
                        flags,
                        tolerance,
                        direction,
                        action,
                        speed_factor,
                        ..
                    } => {
                        let linked_seek = element_state
                            .legacy_v48
                            .as_ref()
                            .and_then(|legacy| legacy.linked_seek)
                            .flatten()
                            .map(|linked| reference(linked.sequence_id, linked.element_index))
                            .unwrap_or(Value::Null);
                        json!({
                            "kind": "movement",
                            "destination": point(destination.x, destination.y),
                            "layer": layer,
                            "sector": sector.map(|value| value.get() as i32).unwrap_or(-1),
                            "gate": gate(*gate_id),
                            "line": line(*line_id),
                            "target": element.map(&entity).unwrap_or(Value::Null),
                            "flags": flags.bits(),
                            "tolerance": float(*tolerance),
                            "direction": direction,
                            "action": *action as u32,
                            "speed_factor": float(*speed_factor),
                            "linked_seek": linked_seek,
                        })
                    }
                    SequenceElementData::Generic { properties } => {
                        let mut ordered: Vec<_> = properties
                            .iter()
                            .filter_map(|(field, value)| {
                                field
                                    .original_ordinal()
                                    .map(|ordinal| (ordinal, *field, value))
                            })
                            .collect();
                        ordered.sort_by_key(|(ordinal, _, _)| *ordinal);
                        let properties: Vec<_> = ordered
                            .into_iter()
                            .map(|(ordinal, field, value)| {
                                let value = match value {
                                    FieldValue::Bool(value) => json!(value),
                                    FieldValue::Integer(value) => {
                                        if matches!(
                                            field,
                                            Field::JumplineSource | Field::JumplineDestination
                                        ) && *value == 0
                                        {
                                            Value::Null
                                        } else {
                                            json!(value)
                                        }
                                    }
                                    FieldValue::Float(value) => float(*value),
                                    FieldValue::GeoPoint2D { x, y } => point(*x, *y),
                                    FieldValue::Point3D { x, y, z } => point3(*x, *y, *z),
                                    FieldValue::Element(value) => entity(*value),
                                    FieldValue::OptionalElement(value) => {
                                        value.map(&entity).unwrap_or(Value::Null)
                                    }
                                    FieldValue::Animation(value) => json!(*value as u32),
                                    FieldValue::LineId(value) => line(Some(*value)),
                                    FieldValue::OptionalLineId(value) => line(*value),
                                    FieldValue::DoorId(value) => gate(Some(*value)),
                                    FieldValue::OptionalDoorId(value) => gate(*value),
                                };
                                json!({ "field": ordinal, "value": value })
                            })
                            .collect();
                        json!({ "kind": "generic", "properties": properties })
                    }
                };

                let postponed = match (
                    element_state.postponed_element_index,
                    element_state.cross_postponed,
                ) {
                    (Some(index), None) => reference(sequence.id, index),
                    (None, Some((id, index))) => reference(id, index),
                    (None, None) => Value::Null,
                    (Some(_), Some(_)) => panic!(
                        "parity sequence element carries both intra- and cross-sequence postponed refs"
                    ),
                };
                let transition_live =
                    element_state.state == crate::sequence::SequenceState::InProgress;
                elements.push(json!({
                    "command": element_state.command as u32,
                    "level": element_state.command_level,
                    "owner": element_state.owner.map(&entity).unwrap_or(Value::Null),
                    "state": element_state.state as u32,
                    "priority": element_state.priority as u32,
                    "posture_after_transition": if transition_live { json!(element_state.posture_after_transition as u32) } else { Value::Null },
                    "action_state_after_transition": if transition_live { json!(element_state.action_state_after_transition as u32) } else { Value::Null },
                    "transition_orders": if transition_live { json!(element_state.num_transition_orders) } else { Value::Null },
                    "script_driven": element_state.script_driven,
                    "postponed": postponed,
                    "orders": orders,
                    "subtype": subtype,
                }));
            }
            sequences.push(json!({
                "cursor": cursor,
                "current_level": current_level,
                "running_elements": running,
                "elements_in_progress": in_progress,
                "started": started,
                "elements": elements,
            }));
        }

        let (elements_to_go, actor_current) = manager.parity_runtime_refs();
        json!({
            "next_order_id": self.inner.orders.next_order_id - 1,
            "sequences": sequences,
            "elements_to_go": elements_to_go
                .into_iter()
                .map(|(id, element)| reference(id, element))
                .collect::<Vec<_>>(),
            "actor_current": actor_current
                .into_iter()
                .map(|(owner, selected)| json!({
                    "owner": entity(owner),
                    "element": reference(selected.sequence_id, selected.element_index),
                }))
                .collect::<Vec<_>>(),
        })
    }

    /// Sparse, serialized sound-source manager state. Host channels and
    /// backend playback queues are deliberately absent; these are the source
    /// fields that survive Original save/load and feed later simulation.
    #[doc(hidden)]
    pub fn parity_sound_sources_state(&self) -> serde_json::Value {
        let float = typed_float;
        let sources = &self.inner.feedback.sound_sim.sources;
        let mut result = Vec::with_capacity(sources.num_sources());
        for index in 0..sources.num_sources() {
            let Some(source) = sources.get(index) else {
                result.push(None);
                continue;
            };
            let kind = match source.source_kind {
                crate::sound_source::SoundSourceKind::Single => 0,
                crate::sound_source::SoundSourceKind::Looped => 1,
                crate::sound_source::SoundSourceKind::Delayed => 2,
                crate::sound_source::SoundSourceKind::Volatile => 3,
            };
            let altitude = match source.altitude {
                crate::sound_geometry::SoundSourceAltitude::Ground => 0,
                crate::sound_geometry::SoundSourceAltitude::Middle => 1,
                crate::sound_geometry::SoundSourceAltitude::Top => 2,
                crate::sound_geometry::SoundSourceAltitude::NoAltitude => 3,
            };
            result.push(Some(projections::SoundSource {
                kind: kind,
                id: source.id,
                global: source.is_global,
                inner_distance: source.inner_distance,
                outer_distance: source.outer_distance,
                noise_covering_distance: source.noise_covering_distance,
                inner_volume: source.inner_volume,
                outer_volume: source.outer_volume,
                shape: source
                    .shape
                    .iter()
                    .map(|point| projections::Point2 {
                        x: float(point.x),
                        y: float(point.y),
                    })
                    .collect::<Vec<_>>(),
                altitude: altitude,
                min_delay: source.min_delay,
                max_delay: source.max_delay,
                delay_stepping: source.delay_stepping,
                timer: source.timer,
                active: source.active,
                ambience_enabled: source.ambience_enabled,
            }));
        }
        serde_json::to_value(result).expect("typed sound source parity must serialize")
    }

    /// Ordered deterministic source-completion deadlines. Looped sources have
    /// no completion entry; Single, Volatile, and Delayed sources retain the
    /// order in which Original queued their pending playback records.
    #[doc(hidden)]
    pub fn parity_sound_completion_frontier_state(&self) -> serde_json::Value {
        serde_json::to_value(
            self.inner
                .feedback
                .sound_sim
                .playing_sources
                .iter()
                .map(|playing| {
                    if self
                        .inner
                        .feedback
                        .sound_sim
                        .sources
                        .get(playing.source_index as usize)
                        .is_none()
                    {
                        panic!(
                            "sound completion frontier references missing source {}",
                            playing.source_index
                        );
                    }
                    projections::SoundCompletion {
                        source_index: playing.source_index,
                        finish_frame: playing.finish_frame,
                    }
                })
                .collect::<Vec<_>>(),
        )
        .expect("typed sound completion parity must serialize")
    }

    /// Serialized global AI-manager state. Mission-static seek/archery
    /// geometry is intentionally absent; Original persists only these ordered
    /// mutable statuses, reservations, counters, alerts, and saved RNG seed.
    #[doc(hidden)]
    pub fn parity_ai_global_state(&self) -> serde_json::Value {
        let entity = typed_entity_reference;
        let global = &self.inner.ai.global;
        serde_json::to_value(projections::GlobalAi {
            stupid_soldiers_cheat: global.stupid_soldiers_cheat,
            seek_points: global
                .seek_points
                .iter()
                .map(|point| projections::SeekPointStatus {
                    frame_when_full_interest: point.frame_when_full_interest,
                    last_calculated_interest: point.last_calculated_interest,
                    locked: point.locked,
                })
                .collect::<Vec<_>>(),
            archery_sectors: global
                .archery_sectors
                .iter()
                .map(|sector| projections::ArcherySector {
                    num_owners: sector.num_owners,
                    point_owners: sector
                        .points
                        .iter()
                        .map(|point| point.owner.map(&entity))
                        .collect::<Vec<_>>(),
                })
                .collect::<Vec<_>>(),
            green_alert_soldiers: global.green_alert_soldiers,
            yellow_alert_soldiers: global.yellow_alert_soldiers,
            red_alert_soldiers: global.red_alert_soldiers,
            overall_alert_status: global.overall_alert_status as u32,
            overall_villain_alert_status: global.overall_villain_alert_status as u32,
            saved_random_seed: global.saved_random_seed,
            forbidden_remarks: global
                .forbidden_remarks
                .iter()
                .map(|entry| projections::ForbiddenRemark {
                    remark: entry.remark as u32,
                    flags: entry.flags,
                    speech_id: entry.speech_id,
                    // This is deliberately the stored scalar, not a normalized
                    // entity reference. The original game stores creation order here;
                    // parity must expose any slot-vs-creation-order divergence.
                    guy_index: entry.guy_index,
                    bad_guy: entry.bad_guy,
                    forbidden_till_frame: entry.forbidden_till_frame,
                })
                .collect::<Vec<_>>(),
            current_speech_variant: global.current_speech_variant,
        })
        .expect("typed global AI parity must serialize")
    }

    /// Exact engine player-character order. The portrait bar has a
    /// different priority-sorted owner and must not stand in for gameplay
    /// loops that walk the original game's player-character registry.
    #[doc(hidden)]
    pub fn parity_pc_registry_state(&self) -> serde_json::Value {
        use serde_json::json;

        serde_json::Value::Array(
            self.inner
                .world
                .original_pc_registry_ids
                .iter()
                .map(|id| {
                    let kind = match id.kind() {
                        crate::element::EntityIdKind::Pc => "pc",
                        other => panic!("Original PC registry contains non-PC entity {other:?}"),
                    };
                    json!({ "kind": kind, "index": id.index() })
                })
                .collect(),
        )
    }

    /// Engine-owned roots serialized outside the element/sequence managers.
    /// References use the same semantic entity and manager-ordinal sequence
    /// forms as the rest of the parity snapshot.
    #[doc(hidden)]
    pub fn parity_engine_runtime_roots_state(
        &self,
        menu_text: &dyn crate::sherwood_stat::MenuTextLookup,
    ) -> serde_json::Value {
        use serde_json::{Value, json};

        let entity = parity_entity_reference;
        let manager = &self.inner.orders.sequence_manager;
        let sequence_ordinals: std::collections::BTreeMap<_, _> = manager
            .sequences_iter()
            .enumerate()
            .map(|(ordinal, sequence)| (sequence.id, ordinal))
            .collect();
        let reference = |value: crate::sequence::SequenceElementRef| {
            let sequence = sequence_ordinals
                .get(&value.sequence_id)
                .copied()
                .unwrap_or_else(|| {
                    panic!(
                        "parity runtime root points outside sequence manager: {:?}/{}",
                        value.sequence_id, value.element_index
                    )
                });
            json!({ "sequence": sequence, "element": value.element_index })
        };
        let stat = &self.inner.mission_domain.mission_stat;
        let pc_names = stat
            .pc_names
            .iter()
            .map(|name| {
                if let Some(slot) = name.name_override {
                    let resolved = menu_text.get(slot.menu_text_id());
                    if !resolved.is_empty() {
                        return resolved;
                    }
                }
                name.fallback.clone()
            })
            .collect::<Vec<_>>();

        json!({
            "timer_elements": self.inner.orders.timer_elements.iter().map(|timer| json!({
                "element": reference(timer.element_ref), "remaining": timer.remaining,
            })).collect::<Vec<_>>(),
            "camera_sequence": self.inner.feedback.cutscene_camera.sequence_element
                .map(&reference).unwrap_or(Value::Null),
            "dead_pc": self.inner.mission_domain.dead_pc.map(&entity).unwrap_or(Value::Null),
            "mission_stat": {
                "collected_money": stat.collected_money,
                "bonus_money": stat.bonus_money,
                "soldier_money": stat.soldier_money,
                "living_soldier_count": stat.living_soldier_count,
                "total_soldier_count": stat.total_soldier_count,
                "new_peasant_count": stat.new_peasant_count,
                "killed_peasant_count": stat.killed_peasant_count,
                "killed_allied_count": stat.killed_allied_count,
                "added_score": stat.added_score,
                "pc_names": pc_names,
                "factions": stat.factions,
            },
            "user_locked": self.inner.players.user_locked,
            "selection_before_user_lock": self.inner.players.selection_before_user_lock
                .iter().copied().map(&entity).collect::<Vec<_>>(),
            "follow_element": self.inner.players.seats[0].follow_element
                .map(&entity).unwrap_or(Value::Null),
        })
    }

    /// Mutable patch, gate, and door-sector state in canonical mission-table
    /// order. Static geometry and patch configuration come from level data and
    /// are deliberately not duplicated.
    #[doc(hidden)]
    pub fn parity_world_interactables_state(&self, assets: &LevelAssets) -> serde_json::Value {
        use serde_json::{Value, json};

        let entity = parity_entity_reference;
        let interactables = &self.inner.script_domains.interactables;
        let patches = interactables
            .patches
            .iter()
            .map(|patch| {
                json!({
                    "active": patch.active,
                    "locked": patch.locked,
                    "occupants": patch.occupants.iter().map(|occupant| {
                        let id = self.inner.entity_id_for_index(occupant.0).unwrap_or_else(|| {
                            panic!("parity patch occupant references missing entity {}", occupant.0)
                        });
                        entity(id)
                    }).collect::<Vec<_>>(),
                    "applied": patch.applied,
                    "in_transition": patch.in_transition,
                })
            })
            .collect::<Vec<_>>();
        let doors = interactables
            .doors
            .iter()
            .map(|door| match door.gate_type {
                crate::gate::GateType::Door => json!({
                    "kind": "door",
                    "active": door.active,
                    "locked_pc": door.locked_pc,
                    "locked_npc_villain": door.locked_npc_villain,
                    "locked_npc_civilian": door.locked_npc_civilian,
                    "unlockable": door.unlockable,
                    "locked_pc_after_patch": door.locked_pc_after_patch,
                    "locked_npc_villain_after_patch": door.locked_npc_villain_after_patch,
                    "locked_npc_civilian_after_patch": door.locked_npc_civilian_after_patch,
                    "unlockable_after_patch": door.unlockable_after_patch,
                    "special_authorisation_pc": door.special_authorisation_pc,
                    "authorised_pc_direct": door.authorised_pc_direct,
                    "authorised_pc_indirect": door.authorised_pc_indirect,
                }),
                crate::gate::GateType::Jump => json!({
                    "kind": "jump", "active": door.active,
                }),
                crate::gate::GateType::None => json!({
                    "kind": "gate", "active": door.active,
                }),
            })
            .collect::<Vec<_>>();
        let grid = &self.inner.world.fast_grid;
        let sector_doors = grid
            .level
            .sectors
            .iter()
            .enumerate()
            .filter(|(_, sector)| sector.sector_type.is_door())
            .map(|(index, sector)| {
                let active = *grid.sector_active.get(index).unwrap_or_else(|| {
                    panic!("parity door sector {index} has no active-state slot")
                });
                json!({ "sector": sector.sector_number.get(), "active": active })
            })
            .collect::<Vec<_>>();

        let lifts = grid
            .level
            .sectors
            .iter()
            .enumerate()
            .filter(|(_, sector)| sector.sector_type.is_lift())
            .map(|(index, sector)| {
                let state = grid
                    .lift_state
                    .get(&(index as u32))
                    .copied()
                    .unwrap_or_default();
                json!({
                    "sector": sector.sector_number.get(),
                    "occupants_pc": state.occupants_pc,
                    "occupants": state.occupants,
                    "occupied_upwards": state.occupied_upwards,
                    "occupied_downwards": state.occupied_downwards,
                    "wait_time": state.wait_time,
                })
            })
            .collect::<Vec<_>>();

        let buildings = &self.inner.script_domains.buildings;
        if buildings.occupants.len() != buildings.arrow_reserves.len() {
            panic!(
                "building occupant table length {} differs from arrow-reserve table length {}",
                buildings.occupants.len(),
                buildings.arrow_reserves.len()
            );
        }
        let building_state = buildings
            .occupants
            .iter()
            .zip(&buildings.arrow_reserves)
            .map(|(occupants, &arrow_reserve)| {
                json!({
                    "occupants": occupants.iter().map(|&handle| {
                        let id = self.inner.entity_id_for_actor_handle(handle).unwrap_or_else(|| {
                            panic!("parity building occupant has invalid actor handle {handle}")
                        });
                        entity(id)
                    }).collect::<Vec<_>>(),
                    "arrow_reserve": arrow_reserve,
                })
            })
            .collect::<Vec<_>>();

        let zones = &self.inner.script_domains.zones.scripts;
        if zones.len() != assets.scripts.zone_grid_indices.len() {
            panic!(
                "script-zone runtime length {} differs from topology length {}",
                zones.len(),
                assets.scripts.zone_grid_indices.len()
            );
        }
        let script_zones = zones
            .iter()
            .zip(assets.scripts.zone_grid_indices.iter().copied())
            .map(|(zone, grid_index)| {
                let grid_apex = grid
                    .sector_type(grid_index)
                    .contains(crate::sector::SectorType::APEX);
                if grid_apex != zone.transformed_to_apex {
                    panic!(
                        "script-zone apex state disagrees with sector overlay at grid index {grid_index}"
                    );
                }
                json!({
                    "occupants": zone.occupant_indices.iter().copied().map(&entity)
                        .collect::<Vec<_>>(),
                    "transformed_to_apex": zone.transformed_to_apex,
                    "max_apex_height": if zone.transformed_to_apex { {
                        json!({
                            "bits": zone.max_throwing_apex_height.to_bits(),
                            "value": zone.max_throwing_apex_height,
                        })
                    } } else { Value::Null },
                })
            })
            .collect::<Vec<_>>();

        json!({
            "patches": patches,
            "doors": doors,
            "sector_doors": sector_doors,
            "lifts": lifts,
            "buildings": building_state,
            "script_zones": script_zones,
        })
    }

    /// Ordered script-created repulsive points plus Original's process-global
    /// next-ID counter. Mission-authored geometry is reconstructed from level
    /// data and is not duplicated here.
    #[doc(hidden)]
    pub fn parity_repulsive_points_state(&self) -> serde_json::Value {
        use serde_json::json;

        let float = parity_float;
        let points = self
            .inner
            .ai
            .global
            .repulsive_points
            .iter()
            .map(|point| {
                let id = u32::try_from(point.id).unwrap_or_else(|_| {
                    panic!("parity repulsive point has negative ID {}", point.id)
                });
                json!({
                    "position": {
                        "x": float(point.position.x),
                        "y": float(point.position.y),
                    },
                    "concave": point.concave,
                    "limit_left": {
                        "x": float(point.limit_left.x),
                        "y": float(point.limit_left.y),
                    },
                    "limit_right": {
                        "x": float(point.limit_right.x),
                        "y": float(point.limit_right.y),
                    },
                    "action_radius": float(point.action_radius),
                    "force_a": float(point.force_a),
                    "force_b": float(point.force_b),
                    "radius": float(point.radius),
                    "id": id,
                    "affects_pcs": point.flags & 1 != 0,
                    "affects_soldiers": point.flags & 2 != 0,
                    "affects_civilians": point.flags & 4 != 0,
                    "affects_animals": point.flags & 8 != 0,
                    "layer": point.position.level,
                })
            })
            .collect::<Vec<_>>();

        json!({
            "next_id": self.inner.world.original_repulsive_point_counter,
            "points": points,
        })
    }

    /// Original-serialized titbit-manager state. Render-only manager counters
    /// are excluded, but every live titbit field is retained because existence,
    /// lifetime, phase, and manager links participate in game logic.
    #[doc(hidden)]
    pub fn parity_titbit_manager_state(&self) -> serde_json::Value {
        use serde_json::{Value, json};

        let entity = |handle: Option<crate::titbit::ElementHandle>| {
            let Some(handle) = handle else {
                return Value::Null;
            };
            let id = self
                .inner
                .entity_id_for_index(handle.0)
                .unwrap_or_else(|| panic!("parity titbit references missing entity {}", handle.0));
            let kind = match id.kind() {
                crate::element::EntityIdKind::Pc => "pc",
                crate::element::EntityIdKind::Soldier => "soldier",
                crate::element::EntityIdKind::Civilian => "civilian",
                crate::element::EntityIdKind::Fx => "fx",
                crate::element::EntityIdKind::Target => "target",
                crate::element::EntityIdKind::Bonus => "bonus",
                crate::element::EntityIdKind::Scroll => "scroll",
                crate::element::EntityIdKind::Projectile => "projectile",
                crate::element::EntityIdKind::Net => "net",
            };
            json!({ "kind": kind, "index": id.index() })
        };
        let float = parity_float;
        let manager = &self.inner.feedback.titbit_manager;
        json!({
            "current_id": manager.parity_current_id(),
            "titbits": manager.titbits().iter().map(|titbit| json!({
                "kind": titbit.kind as u32,
                "frame_count": titbit.frame_count,
                "sprite_frame": titbit.sprite_frame,
                "sprite_row": titbit.sprite_row,
                "phase": titbit.phase,
                "display_order": float(titbit.display_order),
                "layer": titbit.layer,
                "blinking": titbit.blinking,
                "id": titbit.id.get(),
                "element_supplier": entity(titbit.element_supplier),
                "element_manager": entity(titbit.element_manager),
                "position": {
                    "x": float(titbit.position.x),
                    "y": float(titbit.position.y),
                    "z": float(titbit.position.z),
                },
            })).collect::<Vec<_>>(),
        })
    }
}
