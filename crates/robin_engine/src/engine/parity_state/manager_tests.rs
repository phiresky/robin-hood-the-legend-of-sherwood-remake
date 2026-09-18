//! Frozen pre-refactor encoders: intentionally independent of the typed manager schemas.
use super::*;

impl Engine {
    fn original_sequence_manager_state(&self) -> serde_json::Value {
        use crate::sequence::{Field, FieldValue, SequenceElementData};
        use serde_json::{Value, json};

        let float = |value: f32| json!({ "bits": value.to_bits() });
        let point = |x: f32, y: f32| json!({ "x": float(x), "y": float(y) });
        let point3 =
            |x: f32, y: f32, z: f32| json!({ "x": float(x), "y": float(y), "z": float(z) });
        let entity = original_entity_reference;
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
                        linked_seek,
                        ..
                    } => {
                        let linked_seek = linked_seek
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

                let postponed = element_state
                    .postponed
                    .map(|link| reference(link.sequence_id, link.element_index))
                    .unwrap_or(Value::Null);
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

        let (elements_to_go, actor_current) =
            manager.parity_runtime_refs(&self.inner.world.entities);
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
}

fn original_entity_reference(id: EntityId) -> serde_json::Value {
    use crate::element::EntityIdKind::*;
    let kind = match id.kind() {
        Pc => "pc",
        Soldier => "soldier",
        Civilian => "civilian",
        Fx => "fx",
        Target => "target",
        Bonus => "bonus",
        Scroll => "scroll",
        Projectile => "projectile",
        Net => "net",
    };
    serde_json::json!({ "kind": kind, "index": id.index() })
}

fn assert_manager_encoders(engine: &Engine, label: &str) {
    golden::assert_golden(
        &format!("manager_{label}_sequence_manager"),
        &engine.parity_sequence_manager_state(),
    );
    assert_eq!(
        engine.parity_sequence_manager_state(),
        engine.original_sequence_manager_state()
    );
}

#[test]
fn empty_managers_keep_null_references_and_empty_arrays() {
    let engine = Engine {
        inner: EngineInner::new(),
        bootstrap_open: false,
    };
    assert_manager_encoders(&engine, "empty");
}

#[test]
fn populated_manager_schemas_match_original_encoders() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::engine::test_support::actors::make_test_pc;
    use crate::sequence::{
        Field, FieldValue, Sequence, SequenceElement, SequenceElementData, SequenceElementRef,
    };
    let mut inner = EngineInner::new();
    let a = inner.add_test_entity(make_test_pc(Posture::Upright));
    let b = inner.add_test_entity(make_test_pc(Posture::Upright));
    inner.world.original_pc_registry_ids = vec![b, a];
    inner.players.selection_before_user_lock = vec![b, a];
    inner.players.user_locked = true;
    inner.players.seats[0].follow_element = Some(a);
    inner.mission_domain.dead_pc = Some(b);
    for gate_type in [
        crate::gate::GateType::Door,
        crate::gate::GateType::Jump,
        crate::gate::GateType::None,
    ] {
        let mut door = crate::gate::Door::default();
        door.gate_type = gate_type;
        door.active = true;
        door.locked_pc = true;
        door.locked_npc_villain_after_patch = true;
        door.unlockable_after_patch = true;
        door.special_authorisation_pc = true;
        door.authorised_pc_direct = 0x21;
        door.authorised_pc_indirect = 0x42;
        door.point_out = MapPoint::new(-0.0, 2.5);
        door.point_in = MapPoint::new(13.0, -9.0);
        inner.script_domains.interactables.doors.push(door);
    }
    let mut sequence = Sequence::new();
    let mut movement = SequenceElement::new_movement(
        3,
        Command::AssertPosition,
        Some(a),
        crate::order::OrderType::WalkingUpright,
    );
    if let SequenceElementData::Movement {
        destination,
        gate_id,
        line_id,
        direction,
        tolerance,
        speed_factor,
        ..
    } = &mut movement.data
    {
        *destination = MapPoint::new(-0.0, f32::from_bits(0x7fc01234));
        *gate_id = crate::gate::DoorIndex::new(0);
        *line_id = crate::jump_line::JumpLineIndex::new(0);
        *direction = -7;
        *tolerance = 1.25;
        *speed_factor = 2.0;
    }
    std::sync::Arc::make_mut(&mut inner.world.fast_grid)
        .level_mut()
        .jump_lines
        .push(crate::jump_line::JumpLine::new(
            MapPoint::new(1.0, 2.0),
            MapPoint::new(3.0, 4.0),
            0.0,
            0.0,
        ));
    let mut order = crate::order::Order::new(
        crate::order::OrderType::WalkingUpright,
        -0.0,
        3.5,
        std::num::NonZeroU32::new(11).unwrap(),
    );
    order.antagonist = Some(b);
    order.reverse = true;
    order.can_fly = true;
    order.destination_3d = [1.0, -0.0, f32::INFINITY];
    order.flight_vector = [3.0, 4.0];
    movement.orders.push_back(order);
    sequence.append_element(movement);
    sequence.append_element(SequenceElement::new(3, Command::AssertPosition, None));
    sequence.append_element(SequenceElement::new_interaction(
        4,
        Command::AssertPosition,
        Some(b),
        Some(a),
    ));
    sequence.append_element(SequenceElement::new_damage(
        4,
        Command::AssertPosition,
        Some(a),
        Some(b),
        77,
        19,
    ));
    let mut generic = SequenceElement::new_generic(5, Command::AssertPosition, Some(a));
    for (field, value) in [
        (Field::Timer, FieldValue::Integer(17)),
        (Field::ActionAvailable, FieldValue::Bool(true)),
        (Field::CameraZoomLevel, FieldValue::Float(-0.0)),
        (
            Field::CameraPoint,
            FieldValue::GeoPoint2D { x: 1.5, y: -0.0 },
        ),
        (
            Field::ShieldDangerPoint,
            FieldValue::Point3D {
                x: 2.0,
                y: 3.0,
                z: f32::NAN,
            },
        ),
        (Field::Opponent, FieldValue::Element(b)),
        (Field::ScrollOwner, FieldValue::OptionalElement(None)),
        (Field::JumplineSource, FieldValue::Integer(0)),
        (
            Field::JumplineDestination,
            FieldValue::OptionalLineId(crate::jump_line::JumpLineIndex::new(0)),
        ),
        (
            Field::Gate,
            FieldValue::OptionalDoorId(crate::gate::DoorIndex::new(0)),
        ),
        (Field::NoiseDistractionTarget, FieldValue::Integer(999)),
    ] {
        generic.set_property(field, value);
    }
    sequence.append_element(generic);
    let sequence_id = inner.orders.sequence_manager.insert_sequence(sequence);
    inner
        .start_sequence_inline(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            &mut Vec::new(),
            sequence_id,
        )
        .expect("projection fixture registers its initial FIFO entries");
    let sequence = inner
        .orders
        .sequence_manager
        .get_sequence_mut(sequence_id)
        .unwrap();
    sequence.elements[0].state = crate::sequence::SequenceState::InProgress;
    sequence.elements[0].num_transition_orders = 3;
    sequence.elements[0].postponed = Some(SequenceElementRef::new(sequence_id, 1));
    sequence.elements[1].postponed = Some(crate::sequence::SequenceElementRef::new(sequence_id, 4));
    inner.orders.next_order_id = 12;
    let reference = SequenceElementRef::new(sequence_id, 2);
    inner.orders.timer_elements.push(crate::engine::TimerEntry {
        element_ref: reference,
        remaining: -3,
    });
    inner.feedback.cutscene_camera.sequence_element = Some(reference);

    let engine = Engine {
        inner,
        bootstrap_open: false,
    };
    assert_manager_encoders(&engine, "populated");
    let sequences = engine.parity_sequence_manager_state();
    assert!(sequences["sequences"][0]["elements"][1]["transition_orders"].is_null());
    assert_eq!(
        sequences["sequences"][0]["elements"][0]["transition_orders"],
        3
    );
    assert_eq!(
        sequences["sequences"][0]["elements"][0]["subtype"]["destination"]["x"],
        serde_json::json!({"bits": (-0.0f32).to_bits()})
    );
}
