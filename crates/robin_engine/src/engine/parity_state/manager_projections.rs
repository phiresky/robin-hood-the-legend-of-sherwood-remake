//! Stable manager projection schemas. These are diagnostic views, not save codecs.
use super::projections::{FloatBits, Point2, Point3, Point3Bits};
use super::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

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
#[derive(Serialize, Deserialize)]
/// Only the heterogeneous Original field-bag payload remains dynamic.
/// Its ordinal and stable surrounding sequence schema remain typed.
struct Property {
    field: u32,
    value: serde_json::Value,
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

#[derive(Serialize, Deserialize)]
struct Timer {
    element: SequenceReference,
    remaining: i32,
}
#[derive(Serialize, Deserialize)]
struct MissionStat<'a> {
    collected_money: u32,
    bonus_money: u32,
    soldier_money: u32,
    living_soldier_count: u32,
    total_soldier_count: u32,
    new_peasant_count: u32,
    killed_peasant_count: u32,
    killed_allied_count: u32,
    added_score: u32,
    pc_names: Vec<String>,
    factions: Cow<'a, std::collections::BTreeMap<u16, crate::mission_stat::FactionMissionStat>>,
}
#[derive(Serialize, Deserialize)]
struct RuntimeRoots<'a> {
    timer_elements: Vec<Timer>,
    camera_sequence: Option<SequenceReference>,
    dead_pc: Option<ParityEntityReference>,
    mission_stat: MissionStat<'a>,
    user_locked: bool,
    selection_before_user_lock: Vec<ParityEntityReference>,
    follow_element: Option<ParityEntityReference>,
}

#[derive(Serialize, Deserialize)]
struct Patch {
    active: bool,
    locked: bool,
    occupants: Vec<ParityEntityReference>,
    applied: bool,
    in_transition: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Gate {
    Door {
        active: bool,
        locked_pc: bool,
        locked_npc_villain: bool,
        locked_npc_civilian: bool,
        unlockable: bool,
        locked_pc_after_patch: bool,
        locked_npc_villain_after_patch: bool,
        locked_npc_civilian_after_patch: bool,
        unlockable_after_patch: bool,
        special_authorisation_pc: bool,
        authorised_pc_direct: u16,
        authorised_pc_indirect: u16,
    },
    Jump {
        active: bool,
    },
    Gate {
        active: bool,
    },
}
#[derive(Serialize, Deserialize)]
struct SectorDoor {
    sector: i16,
    active: bool,
}
#[derive(Serialize, Deserialize)]
struct Lift {
    sector: i16,
    occupants_pc: u16,
    occupants: u16,
    occupied_upwards: bool,
    occupied_downwards: bool,
    wait_time: u32,
}
#[derive(Serialize, Deserialize)]
struct Building {
    occupants: Vec<ParityEntityReference>,
    arrow_reserve: bool,
}
#[derive(Serialize, Deserialize)]
struct ScriptZone {
    occupants: Vec<ParityEntityReference>,
    transformed_to_apex: bool,
    max_apex_height: Option<ParityFloat>,
}
#[derive(Serialize, Deserialize)]
struct WorldInteractables {
    patches: Vec<Patch>,
    doors: Vec<Gate>,
    sector_doors: Vec<SectorDoor>,
    lifts: Vec<Lift>,
    buildings: Vec<Building>,
    script_zones: Vec<ScriptZone>,
}
#[derive(Serialize, Deserialize)]
struct RepulsivePoint {
    position: Point2,
    concave: bool,
    limit_left: Point2,
    limit_right: Point2,
    action_radius: ParityFloat,
    force_a: ParityFloat,
    force_b: ParityFloat,
    radius: ParityFloat,
    id: u32,
    affects_pcs: bool,
    affects_soldiers: bool,
    affects_civilians: bool,
    affects_animals: bool,
    layer: u16,
}
#[derive(Serialize, Deserialize)]
struct RepulsivePoints {
    next_id: u32,
    points: Vec<RepulsivePoint>,
}
#[derive(Serialize, Deserialize)]
struct Titbit {
    kind: u32,
    frame_count: u16,
    sprite_frame: u16,
    sprite_row: u16,
    phase: u16,
    display_order: ParityFloat,
    layer: u16,
    blinking: bool,
    id: u32,
    element_supplier: Option<ParityEntityReference>,
    element_manager: Option<ParityEntityReference>,
    position: Point3,
}
#[derive(Serialize, Deserialize)]
struct TitbitManager {
    current_id: u32,
    titbits: Vec<Titbit>,
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
        use serde_json::{Value, json};

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
                        ..
                    } => {
                        let linked_seek = element_state
                            .legacy_v48
                            .as_ref()
                            .and_then(|legacy| legacy.linked_seek)
                            .flatten()
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
                                    FieldValue::Float(value) => json!(float(*value)),
                                    FieldValue::GeoPoint2D { x, y } => json!(point(*x, *y)),
                                    FieldValue::Point3D { x, y, z } => json!(point3(*x, *y, *z)),
                                    FieldValue::Element(value) => json!(entity(*value)),
                                    FieldValue::OptionalElement(value) => {
                                        json!(value.map(&entity))
                                    }
                                    FieldValue::Animation(value) => json!(*value as u32),
                                    FieldValue::LineId(value) => json!(line(Some(*value))),
                                    FieldValue::OptionalLineId(value) => json!(line(*value)),
                                    FieldValue::DoorId(value) => json!(gate(Some(*value))),
                                    FieldValue::OptionalDoorId(value) => json!(gate(*value)),
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

                let postponed = match (
                    element_state.postponed_element_index,
                    element_state.cross_postponed,
                ) {
                    (Some(index), None) => Some(reference(sequence.id, index)),
                    (None, Some((id, index))) => Some(reference(id, index)),
                    (None, None) => None,
                    (Some(_), Some(_)) => panic!(
                        "parity sequence element carries both intra- and cross-sequence postponed refs"
                    ),
                };
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

        let (elements_to_go, actor_current) = manager.parity_runtime_refs();
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

    /// Exact engine player-character order. The portrait bar has a
    /// different priority-sorted owner and must not stand in for gameplay
    /// loops that walk the original game's player-character registry.
    #[doc(hidden)]
    pub fn parity_pc_registry_state(&self) -> serde_json::Value {
        serde_json::to_value(
            self.inner
                .world
                .original_pc_registry_ids
                .iter()
                .map(|id| match id.kind() {
                    crate::element::EntityIdKind::Pc => typed_entity_reference(*id),
                    other => panic!("Original PC registry contains non-PC entity {other:?}"),
                })
                .collect::<Vec<_>>(),
        )
        .expect("typed PC registry must serialize")
    }

    /// Engine-owned roots serialized outside the element/sequence managers.
    /// References use the same semantic entity and manager-ordinal sequence
    /// forms as the rest of the parity snapshot.
    #[doc(hidden)]
    pub fn parity_engine_runtime_roots_state(
        &self,
        menu_text: &dyn crate::sherwood_stat::MenuTextLookup,
    ) -> serde_json::Value {
        let entity = typed_entity_reference;
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
            SequenceReference {
                sequence: sequence,
                element: value.element_index,
            }
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

        serde_json::to_value(RuntimeRoots {
            timer_elements: self
                .inner
                .orders
                .timer_elements
                .iter()
                .map(|timer| Timer {
                    element: reference(timer.element_ref),
                    remaining: timer.remaining,
                })
                .collect::<Vec<_>>(),
            camera_sequence: self
                .inner
                .feedback
                .cutscene_camera
                .sequence_element
                .map(&reference),
            dead_pc: self.inner.mission_domain.dead_pc.map(&entity),
            mission_stat: MissionStat {
                collected_money: stat.collected_money,
                bonus_money: stat.bonus_money,
                soldier_money: stat.soldier_money,
                living_soldier_count: stat.living_soldier_count,
                total_soldier_count: stat.total_soldier_count,
                new_peasant_count: stat.new_peasant_count,
                killed_peasant_count: stat.killed_peasant_count,
                killed_allied_count: stat.killed_allied_count,
                added_score: stat.added_score,
                pc_names: pc_names,
                factions: Cow::Borrowed(&stat.factions),
            },
            user_locked: self.inner.players.user_locked,
            selection_before_user_lock: self
                .inner
                .players
                .selection_before_user_lock
                .iter()
                .copied()
                .map(&entity)
                .collect::<Vec<_>>(),
            follow_element: self.inner.players.seats[0].follow_element.map(&entity),
        })
        .expect("typed manager parity must serialize")
    }

    /// Mutable patch, gate, and door-sector state in canonical mission-table
    /// order. Static geometry and patch configuration come from level data and
    /// are deliberately not duplicated.
    #[doc(hidden)]
    pub fn parity_world_interactables_state(&self, assets: &LevelAssets) -> serde_json::Value {
        let entity = typed_entity_reference;
        let interactables = &self.inner.script_domains.interactables;
        let patches = interactables
            .patches
            .iter()
            .map(|patch| Patch {
                active: patch.active,
                locked: patch.locked,
                occupants: patch
                    .occupants
                    .iter()
                    .map(|occupant| {
                        let id = self
                            .inner
                            .entity_id_for_index(occupant.0)
                            .unwrap_or_else(|| {
                                panic!(
                                    "parity patch occupant references missing entity {}",
                                    occupant.0
                                )
                            });
                        entity(id)
                    })
                    .collect::<Vec<_>>(),
                applied: patch.applied,
                in_transition: patch.in_transition,
            })
            .collect::<Vec<_>>();
        let doors = interactables
            .doors
            .iter()
            .map(|door| match door.gate_type {
                crate::gate::GateType::Door => Gate::Door {
                    active: door.active,
                    locked_pc: door.locked_pc,
                    locked_npc_villain: door.locked_npc_villain,
                    locked_npc_civilian: door.locked_npc_civilian,
                    unlockable: door.unlockable,
                    locked_pc_after_patch: door.locked_pc_after_patch,
                    locked_npc_villain_after_patch: door.locked_npc_villain_after_patch,
                    locked_npc_civilian_after_patch: door.locked_npc_civilian_after_patch,
                    unlockable_after_patch: door.unlockable_after_patch,
                    special_authorisation_pc: door.special_authorisation_pc,
                    authorised_pc_direct: door.authorised_pc_direct,
                    authorised_pc_indirect: door.authorised_pc_indirect,
                },
                crate::gate::GateType::Jump => Gate::Jump {
                    active: door.active,
                },
                crate::gate::GateType::None => Gate::Gate {
                    active: door.active,
                },
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
                SectorDoor {
                    sector: sector.sector_number.get(),
                    active: active,
                }
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
                Lift {
                    sector: sector.sector_number.get(),
                    occupants_pc: state.occupants_pc,
                    occupants: state.occupants,
                    occupied_upwards: state.occupied_upwards,
                    occupied_downwards: state.occupied_downwards,
                    wait_time: state.wait_time,
                }
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
            .map(|(occupants, &arrow_reserve)| Building {
                occupants: occupants
                    .iter()
                    .map(|&handle| {
                        let id = self
                            .inner
                            .entity_id_for_actor_handle(handle)
                            .unwrap_or_else(|| {
                                panic!("parity building occupant has invalid actor handle {handle}")
                            });
                        entity(id)
                    })
                    .collect::<Vec<_>>(),
                arrow_reserve: arrow_reserve,
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
                ScriptZone {
                    occupants: zone.occupant_indices.iter().copied().map(&entity)
                        .collect::<Vec<_>>(),
                    transformed_to_apex: zone.transformed_to_apex,
                    max_apex_height: zone.transformed_to_apex.then(|| typed_float(zone.max_throwing_apex_height)),
                }
            })
            .collect::<Vec<_>>();

        serde_json::to_value(WorldInteractables {
            patches: patches,
            doors: doors,
            sector_doors: sector_doors,
            lifts: lifts,
            buildings: building_state,
            script_zones: script_zones,
        })
        .expect("typed manager parity must serialize")
    }

    /// Ordered script-created repulsive points plus Original's process-global
    /// next-ID counter. Mission-authored geometry is reconstructed from level
    /// data and is not duplicated here.
    #[doc(hidden)]
    pub fn parity_repulsive_points_state(&self) -> serde_json::Value {
        let float = typed_float;
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
                RepulsivePoint {
                    position: Point2 {
                        x: float(point.position.x),
                        y: float(point.position.y),
                    },
                    concave: point.concave,
                    limit_left: Point2 {
                        x: float(point.limit_left.x),
                        y: float(point.limit_left.y),
                    },
                    limit_right: Point2 {
                        x: float(point.limit_right.x),
                        y: float(point.limit_right.y),
                    },
                    action_radius: float(point.action_radius),
                    force_a: float(point.force_a),
                    force_b: float(point.force_b),
                    radius: float(point.radius),
                    id: id,
                    affects_pcs: point.flags & 1 != 0,
                    affects_soldiers: point.flags & 2 != 0,
                    affects_civilians: point.flags & 4 != 0,
                    affects_animals: point.flags & 8 != 0,
                    layer: point.position.level,
                }
            })
            .collect::<Vec<_>>();

        serde_json::to_value(RepulsivePoints {
            next_id: self.inner.world.original_repulsive_point_counter,
            points: points,
        })
        .expect("typed manager parity must serialize")
    }

    /// Original-serialized titbit-manager state. Render-only manager counters
    /// are excluded, but every live titbit field is retained because existence,
    /// lifetime, phase, and manager links participate in game logic.
    #[doc(hidden)]
    pub fn parity_titbit_manager_state(&self) -> serde_json::Value {
        let entity = |handle: Option<crate::titbit::ElementHandle>| {
            let Some(handle) = handle else {
                return None;
            };
            let id = self
                .inner
                .entity_id_for_index(handle.0)
                .unwrap_or_else(|| panic!("parity titbit references missing entity {}", handle.0));
            Some(typed_entity_reference(id))
        };
        let float = typed_float;
        let manager = &self.inner.feedback.titbit_manager;
        serde_json::to_value(TitbitManager {
            current_id: manager.parity_current_id(),
            titbits: manager
                .titbits()
                .iter()
                .map(|titbit| Titbit {
                    kind: titbit.kind as u32,
                    frame_count: titbit.frame_count,
                    sprite_frame: titbit.sprite_frame,
                    sprite_row: titbit.sprite_row,
                    phase: titbit.phase,
                    display_order: float(titbit.display_order),
                    layer: titbit.layer,
                    blinking: titbit.blinking,
                    id: titbit.id.get(),
                    element_supplier: entity(titbit.element_supplier),
                    element_manager: entity(titbit.element_manager),
                    position: Point3 {
                        x: float(titbit.position.x),
                        y: float(titbit.position.y),
                        z: float(titbit.position.z),
                    },
                })
                .collect::<Vec<_>>(),
        })
        .expect("typed manager parity must serialize")
    }
}
