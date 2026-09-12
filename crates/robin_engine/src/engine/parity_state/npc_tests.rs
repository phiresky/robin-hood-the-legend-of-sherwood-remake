//! Independent pre-refactor NPC JSON oracle, frozen before typed records.
use super::*;
impl Engine {
    fn original_npc_frontier(&self, id: EntityId, assets: &LevelAssets) -> serde_json::Value {
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

        drop((position_state, sprite_state));
        npc_ai.unwrap_or(Value::Null)
    }
}

#[test]
fn npc_base_and_subclasses_match_frozen_json_encoder() {
    use crate::element::{ActorSoldier, AiBrain, ElementData, ElementKind, Entity, NpcData};
    for mut brain in [
        AiBrain::None,
        AiBrain::Friendly(Box::default()),
        AiBrain::Enemy(Box::default()),
    ] {
        if let Some(base) = brain.base_mut() {
            base.old_state = i32::MIN;
            base.forbidden_remark_ids = vec![u32::MAX, 1, 0];
            base.last_stimulus_multiplicity = [5, 4, 3, 2, 1];
            base.panic_center_x = -0.0;
            base.panic_center_y = f32::from_bits(0x7fc01234);
            base.detached_patrol_path_status.current_waypoint_index = 11;
            base.stimulus_queue.push(crate::ai::Stimulus::with_position(
                crate::ai::StimulusType::EventEnemyNear,
                crate::ai::Position::default(),
            ));
            let mut index = crate::ai::Stimulus::new(crate::ai::StimulusType::EventEnemyNear);
            index.info = crate::ai::StimulusInfo::Index(u16::MAX);
            base.stimulus_queue.push(index);
        }
        if let AiBrain::Enemy(enemy) = &mut brain {
            enemy.previous_state = i32::MIN;
            enemy.previous_substate = -27;
            enemy.my_seek_points = vec![7, 2, 7];
            enemy.seek_point_view_directions = vec![9, 1];
            enemy.personal_seek_point_1 = Some(crate::ai::SeekPoint {
                position: Default::default(),
                frame_when_full_interest: 23,
                directions: vec![5, 1, 5],
                last_calculated_interest: 77,
                locked: true,
                id: 1111,
            });
            enemy.my_shooting_point = Some((3, 7));
            enemy.last_stimulus_dispatched_to_patrol = Some(crate::ai::Stimulus::with_door_combat(
                crate::ai::StimulusType::EventDoorCombat,
                crate::ai::DoorCombatInfo {
                    delay: 29,
                    goal: Default::default(),
                    direction: 15,
                    adversary: None,
                },
            ));
        }
        let mut element = ElementData::default();
        element.kind = ElementKind::ActorSoldier;
        let mut inner = EngineInner::new();
        let id = inner.add_test_entity(Entity::Soldier(ActorSoldier {
            element,
            actor: Default::default(),
            human: Default::default(),
            npc: NpcData {
                ai_brain: brain,
                ..Default::default()
            },
            soldier: Default::default(),
        }));
        let handle = crate::ai::AiEntityHandle::new(id.index());
        if let Some(base) = inner
            .get_entity_mut(id)
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .ai_brain
            .base_mut()
        {
            use crate::ai::{
                CombatInfo, DoorCombatInfo, Hint, Noise, NoiseOrigin, NoiseType, Stimulus,
                StimulusInfo, StimulusType, StolenObject,
            };
            let infos = [
                StimulusInfo::None,
                StimulusInfo::Noise(Noise {
                    origin: NoiseOrigin {
                        x: -0.0,
                        y: 9.5,
                        sector: None,
                        layer: None,
                    },
                    noise_type: NoiseType::Distraction,
                    volume: 3,
                    elevation: 7,
                    element_id: 0,
                }),
                StimulusInfo::Position(Default::default()),
                StimulusInfo::Human(handle),
                StimulusInfo::Hint(Hint {
                    seek_point: Default::default(),
                    seek_flags: 5,
                    who_tells_me: handle,
                }),
                StimulusInfo::Object(handle),
                StimulusInfo::Stolen(StolenObject {
                    object: handle,
                    thief: handle,
                }),
                StimulusInfo::Combat(CombatInfo {
                    actor_npc: handle,
                    enemy_position: Default::default(),
                }),
                StimulusInfo::DoorCombat(DoorCombatInfo {
                    delay: 13,
                    goal: Default::default(),
                    direction: 9,
                    adversary: Some(handle),
                }),
                StimulusInfo::Index(31),
            ];
            base.stimulus_queue.extend(infos.into_iter().map(|info| {
                let mut stimulus = Stimulus::new(StimulusType::EventEnemyNear);
                stimulus.info = info;
                stimulus.owner = Some(handle);
                stimulus.to_whole_patrol = true;
                stimulus
            }));
        }
        let engine = Engine {
            inner,
            bootstrap_open: false,
        };
        let assets = LevelAssets::new();
        let expected = engine.original_npc_frontier(id, &assets);
        let actual = engine.parity_entity_runtime_state(id, &assets);
        if expected.is_null() {
            assert!(actual.get("npc_ai").is_none());
        } else {
            assert_eq!(actual["npc_ai"], expected);
        }
    }
}
