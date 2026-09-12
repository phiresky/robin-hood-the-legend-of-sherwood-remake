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

    fn original_pc_registry_state(&self) -> serde_json::Value {
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

    fn original_engine_runtime_roots_state(
        &self,
        menu_text: &dyn crate::sherwood_stat::MenuTextLookup,
    ) -> serde_json::Value {
        use serde_json::{Value, json};

        let entity = original_entity_reference;
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

    fn original_world_interactables_state(&self, assets: &LevelAssets) -> serde_json::Value {
        use serde_json::{Value, json};

        let entity = original_entity_reference;
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

    fn original_repulsive_points_state(&self) -> serde_json::Value {
        use serde_json::json;

        let float = original_float;
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

    fn original_titbit_manager_state(&self) -> serde_json::Value {
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
        let float = original_float;
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

fn original_float(value: f32) -> serde_json::Value {
    serde_json::json!({ "bits": value.to_bits(), "value": value })
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

#[derive(serde::Serialize, serde::Deserialize)]
struct Menu;
impl crate::sherwood_stat::MenuTextLookup for Menu {
    fn get(&self, id: usize) -> String {
        if id == 250 {
            "Localized A".into()
        } else {
            String::new()
        }
    }
}
fn assert_manager_encoders(engine: &Engine, assets: &LevelAssets) {
    assert_eq!(
        engine.parity_sequence_manager_state(),
        engine.original_sequence_manager_state()
    );
    assert_eq!(
        engine.parity_pc_registry_state(),
        engine.original_pc_registry_state()
    );
    assert_eq!(
        engine.parity_engine_runtime_roots_state(&Menu),
        engine.original_engine_runtime_roots_state(&Menu)
    );
    assert_eq!(
        engine.parity_world_interactables_state(assets),
        engine.original_world_interactables_state(assets)
    );
    assert_eq!(
        engine.parity_repulsive_points_state(),
        engine.original_repulsive_points_state()
    );
    assert_eq!(
        engine.parity_titbit_manager_state(),
        engine.original_titbit_manager_state()
    );
}

#[test]
fn empty_managers_keep_null_references_and_empty_arrays() {
    let engine = Engine {
        inner: EngineInner::new(),
        bootstrap_open: false,
    };
    assert_manager_encoders(&engine, &LevelAssets::new());
    let roots = engine.parity_engine_runtime_roots_state(&Menu);
    assert!(roots["camera_sequence"].is_null());
    assert!(roots["dead_pc"].is_null());
    assert!(roots["follow_element"].is_null());
    assert_eq!(engine.parity_pc_registry_state(), serde_json::json!([]));
}

#[test]
fn sector_and_script_zone_schemas_keep_table_order_and_conditional_apex() {
    let mut inner = EngineInner::new();
    let mut assets = LevelAssets::new();
    for number in [31, 17, 49, 63] {
        crate::engine::test_support::ensure_ordinary_sector(&mut inner, number, 2);
    }
    let grid = &mut inner.world.fast_grid;
    grid.level_mut().sectors[0].sector_type |= crate::sector::SectorType::DOOR;
    grid.level_mut().sectors[1].sector_type |= crate::sector::SectorType::LIFT;
    grid.level_mut().sectors[2].sector_type |= crate::sector::SectorType::APEX;
    grid.set_sector_active(0, false);
    grid.lift_state.insert(
        1,
        crate::fast_find_grid::LiftRuntimeState {
            occupants_pc: 2,
            occupants: 3,
            occupied_upwards: true,
            occupied_downwards: false,
            wait_time: 77,
        },
    );
    let mut apex = crate::sector::ScriptSectorData::new();
    apex.transformed_to_apex = true;
    apex.max_throwing_apex_height = -0.0;
    let mut ordinary = crate::sector::ScriptSectorData::new();
    ordinary.max_throwing_apex_height = f32::NAN;
    inner.script_domains.zones.scripts = vec![apex, ordinary];
    assets.scripts.zone_grid_indices = std::sync::Arc::new(vec![2, 3]);
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };
    assert_manager_encoders(&engine, &assets);
    let value = engine.parity_world_interactables_state(&assets);
    assert_eq!(value["sector_doors"][0]["sector"], 31);
    assert_eq!(value["lifts"][0]["sector"], 17);
    assert_eq!(
        value["script_zones"][0]["max_apex_height"]["bits"],
        (-0.0f32).to_bits()
    );
    assert!(value["script_zones"][1]["max_apex_height"].is_null());
}

#[test]
fn populated_manager_schemas_match_original_encoders() {
    use crate::coordinates::{MapPoint, MapVec, WorldPoint3D};
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
    let stat = &mut inner.mission_domain.mission_stat;
    stat.collected_money = 11;
    stat.bonus_money = 23;
    stat.soldier_money = 37;
    stat.living_soldier_count = 5;
    stat.total_soldier_count = 9;
    stat.new_peasant_count = 3;
    stat.killed_peasant_count = 2;
    stat.killed_allied_count = 1;
    stat.added_score = 101;
    stat.pc_names = vec![
        crate::mission_stat::PcStatName::new(
            "fallback A".into(),
            Some(crate::pc_status::SpecialPeasantName::A),
        ),
        crate::mission_stat::PcStatName::new(
            "fallback B".into(),
            Some(crate::pc_status::SpecialPeasantName::B),
        ),
    ];
    stat.factions.insert(
        8,
        crate::mission_stat::FactionMissionStat {
            encountered_soldiers: 7,
            living_soldiers_at_end: 3,
            soldier_deaths: 4,
            player_caused_soldier_deaths: 2,
        },
    );

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
    let mut patch = crate::patch::Patch::new();
    patch.active = true;
    patch.locked = true;
    patch.applied = true;
    patch.in_transition = true;
    patch.occupants = vec![
        crate::patch::OccupantId(b.index()),
        crate::patch::OccupantId(a.index()),
    ];
    inner.script_domains.interactables.patches.push(patch);
    inner.script_domains.buildings.occupants = vec![vec![
        crate::natives::ScriptHandleCodec::actor_handle(b),
        crate::natives::ScriptHandleCodec::actor_handle(a),
    ]];
    inner.script_domains.buildings.arrow_reserves = vec![true];

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
    inner
        .world
        .fast_grid
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
    sequence.append_element(SequenceElement::new(1, Command::AssertPosition, None));
    sequence.append_element(SequenceElement::new_interaction(
        2,
        Command::AssertPosition,
        Some(b),
        Some(a),
    ));
    sequence.append_element(SequenceElement::new_damage(
        2,
        Command::AssertPosition,
        Some(a),
        Some(b),
        77,
        19,
    ));
    let mut generic = SequenceElement::new_generic(1, Command::AssertPosition, Some(a));
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
    let sequence_id = inner.orders.sequence_manager.launch_sequence(sequence);
    let sequence = inner
        .orders
        .sequence_manager
        .get_sequence_mut(sequence_id)
        .unwrap();
    sequence.elements[0].state = crate::sequence::SequenceState::InProgress;
    sequence.elements[0].num_transition_orders = 3;
    sequence.elements[0].postponed_element_index = Some(1);
    sequence.elements[1].cross_postponed = Some((sequence_id, 4));
    inner.orders.next_order_id = 12;
    let reference = SequenceElementRef::new(sequence_id, 2);
    inner.orders.timer_elements.push(TimerEntry {
        element_ref: reference,
        remaining: -3,
    });
    inner.feedback.cutscene_camera.sequence_element = Some(reference);

    let mut repulsive = crate::ai::RepulsivePoint::new(
        13,
        crate::ai::Position {
            x: -0.0,
            y: f32::NAN,
            sector: None,
            level: 4,
        },
        2.0,
        3.0,
        0xf,
    );
    repulsive.concave = true;
    repulsive.limit_left = MapVec::new(-1.0, 2.0);
    repulsive.limit_right = MapVec::new(3.0, -4.0);
    repulsive.force_a = f32::INFINITY;
    inner.ai.global.repulsive_points.push(repulsive);
    inner.world.original_repulsive_point_counter = 17;
    inner.feedback.titbit_manager.adopt_v48_serialized_state(
        99,
        vec![crate::titbit::TitbitInfo {
            kind: crate::titbit::TitbitKind::Plouf,
            phase: 3,
            sprite_row: 4,
            sprite_frame: 5,
            frame_count: 6,
            element_supplier: Some(crate::titbit::ElementHandle(a.index())),
            element_manager: None,
            layer: 2,
            position: WorldPoint3D::new(-0.0, 12.5, f32::NAN),
            display_order: f32::NEG_INFINITY,
            blinking: true,
            id: crate::titbit::TitbitId::new(97).unwrap(),
        }],
    );
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };
    assert_manager_encoders(&engine, &LevelAssets::new());
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
    let world = engine.parity_world_interactables_state(&LevelAssets::new());
    assert!(world["doors"][1].get("locked_pc").is_none());
    assert_eq!(
        engine.parity_engine_runtime_roots_state(&Menu)["mission_stat"]["pc_names"],
        serde_json::json!(["Localized A", "fallback B"])
    );
}
