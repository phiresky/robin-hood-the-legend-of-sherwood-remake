//! Stable manager projection schemas. These are diagnostic views, not save codecs.
use super::projections::{FloatBits, Point3Bits};
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Point2Bits {
    x: FloatBits,
    y: FloatBits,
}
#[derive(Serialize, Deserialize)]
struct GateBits {
    kind: String,
    sector_out: i16,
    sector_in: i16,
    layer_out: u16,
    layer_in: u16,
    point_out: Point2Bits,
    point_in: Point2Bits,
}
#[derive(Serialize, Deserialize)]
struct LineBits {
    a: Point2Bits,
    b: Point2Bits,
}
#[derive(Serialize, Deserialize)]
struct SequenceReference {
    sequence: usize,
    element: usize,
}
#[derive(Serialize, Deserialize)]
struct SequenceOrder {
    action: u32,
    destination: Point2Bits,
    destination_3d: Point3Bits,
    flight_vector: Point2Bits,
    tolerance: FloatBits,
    apply_transition: bool,
    reverse: bool,
    compute_direction: bool,
    can_fly: bool,
    lock_ai: bool,
    transition: bool,
    done: bool,
    id: u32,
    antagonist: Option<ParityEntityReference>,
}
/// Heterogeneous Original field-bag payload. Untagged, so every variant
/// serializes as exactly its inner JSON shape (`Null` as `null`).
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum PropertyValue {
    Null,
    Bool(bool),
    Integer(u32),
    Float(FloatBits),
    Point2(Point2Bits),
    Point3(Point3Bits),
    Element(ParityEntityReference),
    OptionalElement(Option<ParityEntityReference>),
    Animation(u32),
    Line(Option<LineBits>),
    Gate(Option<GateBits>),
}
#[derive(Serialize, Deserialize)]
struct Property {
    field: u32,
    value: PropertyValue,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SequenceSubtype {
    Simple,
    Interaction {
        antagonist: Option<ParityEntityReference>,
    },
    Damage {
        origin: Option<ParityEntityReference>,
        damage: u16,
        concussion: u16,
        harder_hit: bool,
        sword_strike: i32,
        arrow: Option<ParityEntityReference>,
    },
    Movement {
        destination: Point2Bits,
        layer: u16,
        sector: i32,
        gate: Option<GateBits>,
        line: Option<LineBits>,
        target: Option<ParityEntityReference>,
        flags: u32,
        tolerance: FloatBits,
        direction: i16,
        action: u32,
        speed_factor: FloatBits,
        linked_seek: Option<SequenceReference>,
    },
    Generic {
        properties: Vec<Property>,
    },
}
#[derive(Serialize, Deserialize)]
struct SequenceElement {
    command: u32,
    level: u16,
    owner: Option<ParityEntityReference>,
    state: u32,
    priority: u32,
    posture_after_transition: Option<u32>,
    action_state_after_transition: Option<u32>,
    transition_orders: Option<usize>,
    script_driven: bool,
    postponed: Option<SequenceReference>,
    orders: Vec<SequenceOrder>,
    subtype: SequenceSubtype,
}
#[derive(Serialize, Deserialize)]
struct Sequence {
    cursor: usize,
    current_level: u16,
    running_elements: u16,
    elements_in_progress: u16,
    started: bool,
    elements: Vec<SequenceElement>,
}
#[derive(Serialize, Deserialize)]
struct ActorCurrent {
    owner: ParityEntityReference,
    element: SequenceReference,
}
#[derive(Serialize, Deserialize)]
struct SequenceManager {
    next_order_id: u32,
    sequences: Vec<Sequence>,
    elements_to_go: Vec<SequenceReference>,
    actor_current: Vec<ActorCurrent>,
}

fn bits(value: f32) -> FloatBits {
    FloatBits {
        bits: value.to_bits(),
    }
}
fn point_bits(x: f32, y: f32) -> Point2Bits {
    Point2Bits {
        x: bits(x),
        y: bits(y),
    }
}
fn point3_bits(x: f32, y: f32, z: f32) -> Point3Bits {
    Point3Bits {
        x: bits(x),
        y: bits(y),
        z: bits(z),
    }
}

impl Engine {
    /// Canonical manager-insertion-ordered sequence state for schema-13
    /// Original parity. Runtime allocation IDs are deliberately replaced by
    /// `(sequence ordinal, element index)` references.
    #[doc(hidden)]
    pub fn parity_sequence_manager_state(&self) -> serde_json::Value {
        use crate::sequence::{Field, FieldValue, SequenceElementData};

        let float = bits;
        let point = point_bits;
        let point3 = point3_bits;
        let entity = typed_entity_reference;
        let doors = &self.inner.script_domains.interactables.doors;
        let gate = |id: Option<crate::gate::DoorIndex>| -> Option<GateBits> {
            let Some(id) = id else { return None };
            let door = doors
                .get(usize::from(id))
                .unwrap_or_else(|| panic!("parity sequence references missing door {id}"));
            let kind = match door.gate_type {
                crate::gate::GateType::Door => "door",
                crate::gate::GateType::Jump => "jump",
                crate::gate::GateType::None => "gate",
            };
            Some(GateBits {
                kind: kind.to_owned(),
                sector_out: door.sector_out.get(),
                sector_in: door.sector_in.get(),
                layer_out: door.layer_out,
                layer_in: door.layer_in,
                point_out: point(door.point_out.x, door.point_out.y),
                point_in: point(door.point_in.x, door.point_in.y),
            })
        };
        let lines = &self.inner.world.fast_grid.level.jump_lines;
        let line = |id: Option<crate::jump_line::JumpLineIndex>| -> Option<LineBits> {
            let Some(id) = id else { return None };
            let line = lines
                .get(usize::from(id))
                .unwrap_or_else(|| panic!("parity sequence references missing jump line {id}"));
            Some(LineBits {
                a: point(line.point_a.x, line.point_a.y),
                b: point(line.point_b.x, line.point_b.y),
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
            SequenceReference {
                sequence: sequence,
                element: element,
            }
        };

        let mut sequences = Vec::new();
        for sequence in manager.sequences_iter() {
            let (cursor, current_level, running, in_progress, started) = sequence.parity_counters();
            let mut elements = Vec::new();
            for element_state in &sequence.elements {
                let orders: Vec<_> = element_state
                    .orders
                    .iter()
                    .map(|order| SequenceOrder {
                        action: order.order_type as u32,
                        destination: point(order.target_x, order.target_y),
                        destination_3d: point3(
                            order.destination_3d[0],
                            order.destination_3d[1],
                            order.destination_3d[2],
                        ),
                        flight_vector: point(order.flight_vector[0], order.flight_vector[1]),
                        tolerance: float(order.tolerance),
                        apply_transition: order.apply_transition_at_this_point,
                        reverse: order.reverse,
                        compute_direction: order.compute_direction,
                        can_fly: order.can_fly,
                        lock_ai: order.lock_ai,
                        transition: order.transition,
                        done: order.done,
                        id: order.order_id.get() - 1,
                        antagonist: order.antagonist.map(&entity),
                    })
                    .collect();

                let subtype = match &element_state.data {
                    SequenceElementData::Simple => SequenceSubtype::Simple,
                    SequenceElementData::Interaction { antagonist } => {
                        SequenceSubtype::Interaction {
                            antagonist: antagonist.map(&entity),
                        }
                    }
                    SequenceElementData::Damage {
                        origin,
                        projectile,
                        damage,
                        concussion,
                        sword_strike,
                        is_harder_hit,
                        ..
                    } => SequenceSubtype::Damage {
                        origin: origin.map(&entity),
                        damage: *damage,
                        concussion: *concussion,
                        harder_hit: *is_harder_hit,
                        sword_strike: sword_strike.map(|strike| strike as i32).unwrap_or(11),
                        arrow: projectile.map(&entity),
                    },
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
                            .map(|linked| reference(linked.sequence_id, linked.element_index));
                        SequenceSubtype::Movement {
                            destination: point(destination.x, destination.y),
                            layer: *layer,
                            sector: sector.map(|value| value.get() as i32).unwrap_or(-1),
                            gate: gate(*gate_id),
                            line: line(*line_id),
                            target: element.map(&entity),
                            flags: flags.bits(),
                            tolerance: float(*tolerance),
                            direction: *direction,
                            action: *action as u32,
                            speed_factor: float(*speed_factor),
                            linked_seek: linked_seek,
                        }
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
                                    FieldValue::Bool(value) => PropertyValue::Bool(*value),
                                    FieldValue::Integer(value) => {
                                        if matches!(
                                            field,
                                            Field::JumplineSource | Field::JumplineDestination
                                        ) && *value == 0
                                        {
                                            PropertyValue::Null
                                        } else {
                                            PropertyValue::Integer(*value)
                                        }
                                    }
                                    FieldValue::Float(value) => PropertyValue::Float(float(*value)),
                                    FieldValue::GeoPoint2D { x, y } => {
                                        PropertyValue::Point2(point(*x, *y))
                                    }
                                    FieldValue::Point3D { x, y, z } => {
                                        PropertyValue::Point3(point3(*x, *y, *z))
                                    }
                                    FieldValue::Element(value) => {
                                        PropertyValue::Element(entity(*value))
                                    }
                                    FieldValue::OptionalElement(value) => {
                                        PropertyValue::OptionalElement(value.map(&entity))
                                    }
                                    FieldValue::Animation(value) => {
                                        PropertyValue::Animation(*value as u32)
                                    }
                                    FieldValue::LineId(value) => {
                                        PropertyValue::Line(line(Some(*value)))
                                    }
                                    FieldValue::OptionalLineId(value) => {
                                        PropertyValue::Line(line(*value))
                                    }
                                    FieldValue::DoorId(value) => {
                                        PropertyValue::Gate(gate(Some(*value)))
                                    }
                                    FieldValue::OptionalDoorId(value) => {
                                        PropertyValue::Gate(gate(*value))
                                    }
                                };
                                Property {
                                    field: ordinal,
                                    value,
                                }
                            })
                            .collect();
                        SequenceSubtype::Generic {
                            properties: properties,
                        }
                    }
                };

                let postponed = element_state
                    .postponed
                    .map(|link| reference(link.sequence_id, link.element_index));
                let transition_live =
                    element_state.state == crate::sequence::SequenceState::InProgress;
                elements.push(SequenceElement {
                    command: element_state.command as u32,
                    level: element_state.command_level,
                    owner: element_state.owner.map(&entity),
                    state: element_state.state as u32,
                    priority: element_state.priority as u32,
                    posture_after_transition: transition_live
                        .then_some(element_state.posture_after_transition as u32),
                    action_state_after_transition: transition_live
                        .then_some(element_state.action_state_after_transition as u32),
                    transition_orders: transition_live
                        .then_some(element_state.num_transition_orders),
                    script_driven: element_state.script_driven,
                    postponed: postponed,
                    orders: orders,
                    subtype: subtype,
                });
            }
            sequences.push(Sequence {
                cursor: cursor,
                current_level: current_level,
                running_elements: running,
                elements_in_progress: in_progress,
                started: started,
                elements: elements,
            });
        }

        let (elements_to_go, actor_current) =
            manager.parity_runtime_refs(&self.inner.world.entities);
        serde_json::to_value(SequenceManager {
            next_order_id: self.inner.orders.next_order_id - 1,
            sequences: sequences,
            elements_to_go: elements_to_go
                .into_iter()
                .map(|(id, element)| reference(id, element))
                .collect::<Vec<_>>(),
            actor_current: actor_current
                .into_iter()
                .map(|(owner, selected)| ActorCurrent {
                    owner: entity(owner),
                    element: reference(selected.sequence_id, selected.element_index),
                })
                .collect::<Vec<_>>(),
        })
        .expect("typed manager parity must serialize")
    }
}
