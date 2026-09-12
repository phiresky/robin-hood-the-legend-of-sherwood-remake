//! Frozen pre-refactor JSON encoder: intentionally independent of typed schemas.
use super::*;

impl Engine {
    fn original_position_sprite_frontier(
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
        let float = parity_float;
        let point2 = |x: f32, y: f32| json!({ "x": float(x), "y": float(y) });
        let point3 =
            |x: f32, y: f32, z: f32| json!({ "x": float(x), "y": float(y), "z": float(z) });
        let bbox = |bbox: crate::coordinates::MapBBox| match bbox.0 {
            Some(rect) => json!({
                "min": point2(rect.min().x, rect.min().y),
                "max": point2(rect.max().x, rect.max().y),
            }),
            None => Value::Null,
        };
        let sector = |handle: Option<crate::position_interface::SectorHandle>| {
            handle.map_or(Value::Null, |handle| {
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
                json!(sector.sector_number.get())
            })
        };
        let target = position.target_element.map_or(Value::Null, entity_ref);
        let door = position.door.map_or(Value::Null, |door_handle| {
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
            json!({
                "kind": kind,
                "sector_out": door.sector_out.get(),
                "sector_in": door.sector_in.get(),
                "layer_out": door.layer_out,
                "layer_in": door.layer_in,
                "point_out": point2(door.point_out.x, door.point_out.y),
                "point_in": point2(door.point_in.x, door.point_in.y),
            })
        });
        let obstacle = position.obstacle.map_or(Value::Null, |handle| {
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
                json!({ "kind": "projection", "index": index })
            } else {
                json!({ "kind": "sight", "index": obstacle.id })
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
            .map(|(&from, &to)| json!({ "from": from as u32, "to": to as u32 }))
            .collect::<Vec<_>>();

        // Keep these in bounded chunks: a single `json!` object containing
        // the entire serialized position frontier exceeds the macro's normal
        // recursion limit.
        let mut position_state = json!({
                "computed_position": position.computed_position.bits(),
                "computed_increment": position.computed_increment.bits(),
                "material": position.material,
                "posture": position.posture as u32,
                "old_posture": position.old_posture as u32,
                "direction": i16::from(position.direction),
                "direction_goal": i16::from(position.direction_goal),
                "slow_turn_count": position.slow_turn_count,
                "direction_count": position.direction_count,
                "layer": position.layer.map(crate::position_interface::Layer::get),
                "layer_goal": position.layer_goal.map(crate::position_interface::Layer::get),
                "tolerance": float(position.tolerance),
                "directional_tolerance": position.directional_tolerance,
                "accumulate_movement_map": position.accumulate_movement_map,
                "anti_collision_on": position.anti_collision_on,
                "goal_next_valid": position.goal_next_valid,
                "deviated": position.deviated,
                "door_direction": position.door_direction,
                "reversed_movement": position.reversed_movement,
                "blocked_count": position.blocked_count,
                "radius": float(position.radius),
                "emergency_lying_box": position.use_emergency_lying_box,
                "sector": sector(position.sector), "sector_goal": sector(position.sector_goal),
                "door": door, "obstacle": obstacle, "target": target,
        });
        position_state
            .as_object_mut()
            .expect("parity position chunk must be an object")
            .extend(into_projection_fields(json!({
                "world": point3(position.position.x, position.position.y, position.position.z),
                "map": point2(position.map.x, position.map.y),
                "sprite": point2(current_sprite.x, current_sprite.y),
                "old_world": point3(position.old_position.x, position.old_position.y, position.old_position.z),
                "old_map": point2(position.old_map.x, position.old_map.y),
                "old_sprite": point2(position.old_sprite.x, position.old_sprite.y),
                "goal_map": point2(position.goal_map.x, position.goal_map.y),
                "goal_next_map": point2(position.goal_next_map.x, position.goal_next_map.y),
                "goal_world": point3(position.goal.x, position.goal.y, position.goal.z),
                "increment": point3(position.increment.x, position.increment.y, position.increment.z),
                "increment_map": point2(position.increment_map.x, position.increment_map.y),
                "accumulated_movement_map": point2(position.accumulated_movement_map.x, position.accumulated_movement_map.y),
                "forecasted_movement": point3(position.forecasted_movement.x, position.forecasted_movement.y, position.forecasted_movement.z),
                "move_box": bbox(position.move_box_map), "blocked_box": bbox(position.blocked_box),
                })));
        let sprite_state = json!({
                "row": sprite.current_row, "frame": sprite.current_frame,
                "frame_count": sprite.frame_count,
                "flight_countdown": sprite.flight_frame_countdown,
                "width": sprite.current_width, "height": sprite.current_height,
                "last_action": sprite.last_action as u32,
                "last_processed_order_id": sprite.last_processed_order_id,
                "masked": sprite.masked, "alternate_profile": sprite.use_alternate_profile,
                "action_done_frame": sprite.action_done_frame,
                "action_done_counter": sprite.action_done_counter,
                "last_sound_id": sprite.last_sound_id,
                "behind_display_order_reference": sprite.behind_display_order_ref,
                "display_order_reference": sprite.display_order_ref.map_or(Value::Null, entity_ref),
                "replacements": replacements,
        });

        json!({ "position": position_state, "sprite": sprite_state })
    }
}

#[test]
fn position_and_sprite_match_frozen_json_encoder() {
    for (x, y) in [(0.0, -0.0), (123.25, -7.75), (f32::MIN_POSITIVE, 99.0)] {
        let mut inner = EngineInner::new();
        let mut element = crate::element::ElementData::default();
        element.kind = crate::element::ElementKind::Fx;
        element.set_position_map(crate::coordinates::MapPoint::new(x, y));
        element.sprite.current_row = 17;
        element.sprite.current_frame = 9;
        element.sprite.masked = true;
        element.sprite.last_processed_order_id = u32::MAX;
        let id = inner.add_test_entity(crate::element::Entity::Fx(crate::element::ElementFx {
            element,
            fx: Default::default(),
        }));
        let engine = Engine {
            inner,
            bootstrap_open: false,
        };
        let assets = LevelAssets::new();
        let actual = engine.parity_entity_runtime_state(id, &assets);
        let expected = engine.original_position_sprite_frontier(id, &assets);
        assert_eq!(actual["position"], expected["position"]);
        assert_eq!(actual["sprite"], expected["sprite"]);
        assert_eq!(
            actual.get("npc_ai"),
            None,
            "absent subtype keys must stay omitted"
        );
        assert!(actual["position"]["target"].is_null());
    }
}

#[test]
fn float_projection_preserves_bits_nonfinite_null_and_negative_zero() {
    for value in [
        0.0,
        -0.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::from_bits(0x7fc01234),
    ] {
        let expected = serde_json::json!({ "bits": value.to_bits(), "value": value });
        assert_eq!(serde_json::to_value(typed_float(value)).unwrap(), expected);
    }
}

#[test]
fn script_globals_projection_remains_an_ordered_signed_scalar_array() {
    let mut inner = EngineInner::new();
    inner.scripts.globals = vec![i32::MIN, -1, 0, i32::MAX, 0];
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };
    let state = engine.parity_engine_state();
    assert_eq!(state.script_globals, vec![i32::MIN, -1, 0, i32::MAX, 0]);
    let value = serde_json::to_value(state).unwrap();
    assert_eq!(
        value["script_globals"],
        serde_json::json!([-2147483648i64, -1, 0, 2147483647i64, 0])
    );
}

impl Engine {
    fn original_parity_sound_sources_state(&self) -> serde_json::Value {
        use serde_json::{Value, json};

        let float = parity_float;
        let sources = &self.inner.feedback.sound_sim.sources;
        let mut result = Vec::with_capacity(sources.num_sources());
        for index in 0..sources.num_sources() {
            let Some(source) = sources.get(index) else {
                result.push(Value::Null);
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
            result.push(json!({
                "kind": kind,
                "id": source.id,
                "global": source.is_global,
                "inner_distance": source.inner_distance,
                "outer_distance": source.outer_distance,
                "noise_covering_distance": source.noise_covering_distance,
                "inner_volume": source.inner_volume,
                "outer_volume": source.outer_volume,
                "shape": source.shape.iter().map(|point| json!({
                    "x": float(point.x), "y": float(point.y)
                })).collect::<Vec<_>>(),
                "altitude": altitude,
                "min_delay": source.min_delay,
                "max_delay": source.max_delay,
                "delay_stepping": source.delay_stepping,
                "timer": source.timer,
                "active": source.active,
                "ambience_enabled": source.ambience_enabled,
            }));
        }
        Value::Array(result)
    }
    fn original_parity_sound_completion_frontier_state(&self) -> serde_json::Value {
        use serde_json::json;

        serde_json::Value::Array(
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
                    json!({
                        "source_index": playing.source_index,
                        "finish_frame": playing.finish_frame,
                    })
                })
                .collect(),
        )
    }
    fn original_parity_ai_global_state(&self) -> serde_json::Value {
        use serde_json::{Value, json};

        let entity = parity_entity_reference;
        let global = &self.inner.ai.global;
        json!({
            "stupid_soldiers_cheat": global.stupid_soldiers_cheat,
            "seek_points": global.seek_points.iter().map(|point| json!({
                "frame_when_full_interest": point.frame_when_full_interest,
                "last_calculated_interest": point.last_calculated_interest,
                "locked": point.locked,
            })).collect::<Vec<_>>(),
            "archery_sectors": global.archery_sectors.iter().map(|sector| json!({
                "num_owners": sector.num_owners,
                "point_owners": sector.points.iter().map(|point| point.owner
                    .map(&entity).unwrap_or(Value::Null)).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "green_alert_soldiers": global.green_alert_soldiers,
            "yellow_alert_soldiers": global.yellow_alert_soldiers,
            "red_alert_soldiers": global.red_alert_soldiers,
            "overall_alert_status": global.overall_alert_status as u32,
            "overall_villain_alert_status": global.overall_villain_alert_status as u32,
            "saved_random_seed": global.saved_random_seed,
            "forbidden_remarks": global.forbidden_remarks.iter().map(|entry| json!({
                "remark": entry.remark as u32,
                "flags": entry.flags,
                "speech_id": entry.speech_id,
                // This is deliberately the stored scalar, not a normalized
                // entity reference. The original game stores creation order here;
                // parity must expose any slot-vs-creation-order divergence.
                "guy_index": entry.guy_index,
                "bad_guy": entry.bad_guy,
                "forbidden_till_frame": entry.forbidden_till_frame,
            })).collect::<Vec<_>>(),
            "current_speech_variant": global.current_speech_variant,
        })
    }
    fn original_parity_shield_controller_state(&self) -> serde_json::Value {
        use serde_json::{Value, json};

        let entity = |id: EntityId| {
            let kind = match id.kind() {
                crate::element::EntityIdKind::Pc => "pc",
                other => panic!("shield controller protects non-PC entity {other:?}"),
            };
            json!({ "kind": kind, "index": id.index() })
        };
        let shield = &self.inner.world.shield;
        json!({
            "is_protected": shield.is_protected,
            "protected_pc": shield.protected_pc.map(&entity).unwrap_or(Value::Null),
            "danger_point": {
                "x": { "bits": shield.danger_point.x.to_bits() },
                "y": { "bits": shield.danger_point.y.to_bits() },
                "z": { "bits": shield.danger_point.z.to_bits() },
            },
        })
    }
}

#[test]
fn manager_snapshots_match_frozen_json_encoders() {
    let mut inner = EngineInner::new();
    inner.ai.global.saved_random_seed = i64::MIN;
    inner.ai.global.green_alert_soldiers = 7;
    inner.ai.global.current_speech_variant = u16::MAX;
    inner.ai.global.seek_points.push(crate::ai::SeekPoint {
        position: Default::default(),
        frame_when_full_interest: u32::MAX,
        directions: vec![1, 3],
        last_calculated_interest: 19,
        locked: true,
        id: 0,
    });
    let mut source = crate::sound_source::SoundSource::default();
    source.id = 17;
    source.shape = vec![crate::coordinates::MapPoint::new(-0.0, 3.5)];
    source.timer = 41;
    inner.feedback.sound_sim.sources.add(source);
    inner
        .feedback
        .sound_sim
        .playing_sources
        .push(crate::sound::PlayingSource {
            source_index: 0,
            finish_frame: u32::MAX,
        });
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };
    assert_eq!(
        engine.parity_sound_sources_state(),
        engine.original_parity_sound_sources_state()
    );
    assert_eq!(
        engine.parity_sound_completion_frontier_state(),
        engine.original_parity_sound_completion_frontier_state()
    );
    assert_eq!(
        engine.parity_ai_global_state(),
        engine.original_parity_ai_global_state()
    );
    assert_eq!(
        engine.parity_shield_controller_state(),
        engine.original_parity_shield_controller_state()
    );
}

#[test]
fn entity_envelope_omits_absent_components_but_keeps_explicit_component_null() {
    let mut inner = EngineInner::new();
    let mut element = crate::element::ElementData::default();
    element.kind = crate::element::ElementKind::Fx;
    let id = inner.add_test_entity(crate::element::Entity::Fx(crate::element::ElementFx {
        element,
        fx: Default::default(),
    }));
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };
    let original = engine.original_position_sprite_frontier(id, &LevelAssets::new());
    let position = serde_json::from_value(original["position"].clone()).unwrap();
    let sprite = serde_json::from_value(original["sprite"].clone()).unwrap();
    let envelope = projections::EntityRuntime {
        position,
        sprite,
        subtype: Some(serde_json::Value::Null),
        npc_ai: None,
        human_continuation: None,
        human_structure: None,
        pc_tail: None,
        pc_core: None,
        pc_qa: Some(vec![]),
        pc_interface: None,
        pc_portrait: None,
    };
    let value = serde_json::to_value(envelope).unwrap();
    assert_eq!(value["subtype"], serde_json::Value::Null);
    assert!(value.as_object().unwrap().contains_key("subtype"));
    assert!(!value.as_object().unwrap().contains_key("npc_ai"));
    assert_eq!(value["pc_qa"], serde_json::json!([]));
}
