//! Frozen pre-refactor JSON subtype encoder.
use super::*;
impl Engine {
    fn original_subtype_frontier(&self, id: EntityId) -> serde_json::Value {
        use serde_json::{Value, json};
        let entity = self
            .inner
            .world
            .entities
            .get(id)
            .expect("subtype fixture entity");
        let entity_ref = parity_entity_reference;
        let point2 = |x: f32, y: f32| json!({ "x": parity_float(x), "y": parity_float(y) });
        let point3 = |x: f32, y: f32, z: f32| json!({ "x": parity_float(x), "y": parity_float(y), "z": parity_float(z) });
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

        subtype.unwrap_or(Value::Null)
    }
}

#[test]
fn projectile_subtypes_match_frozen_json_including_partial_runtime_metadata() {
    use crate::element::{
        ElementData, ElementKind, ElementProjectile, Entity, ObjectData, ProjectileData,
        TrajectoryPoint, TrajectoryPointRuntime,
    };
    use crate::element_kinds::ObjectType;
    for object_type in [
        ObjectType::Arrow,
        ObjectType::Purse,
        ObjectType::Coin,
        ObjectType::Wasp,
        ObjectType::WaspNest,
        ObjectType::BonusWaspNest,
        ObjectType::Stone,
    ] {
        for active in [false, true] {
            let mut element = ElementData::default();
            element.kind = ElementKind::ObjectProjectile;
            element.active = active;
            let mut projectile = ProjectileData::default();
            projectile.start_of_trajectory_x = -0.0;
            projectile.start_of_trajectory_y = f32::from_bits(0x7fc01234);
            projectile.frame_count = 17;
            projectile.arrow_bow_profile = Some(None);
            projectile.last_orientation_azimuth = -37;
            projectile.trajectory = vec![
                TrajectoryPoint {
                    position: crate::coordinates::WorldPoint3D::new(1.0, 2.0, 3.0),
                    time: 7,
                },
                TrajectoryPoint {
                    position: crate::coordinates::WorldPoint3D::new(-1.0, -2.0, -3.0),
                    time: 9,
                },
            ];
            projectile.trajectory_runtime = vec![TrajectoryPointRuntime {
                bounce: true,
                material: u32::MAX,
            }];
            let mut inner = EngineInner::new();
            let id = inner.add_test_entity(Entity::Projectile(ElementProjectile {
                element,
                object: ObjectData {
                    object_type,
                    ..Default::default()
                },
                projectile,
            }));
            let engine = Engine {
                inner,
                bootstrap_open: false,
            };
            let expected = engine.original_subtype_frontier(id);
            let actual = engine.parity_entity_runtime_state(id, &LevelAssets::new());
            if active {
                assert_eq!(actual["subtype"], expected);
            } else {
                assert!(actual.get("subtype").is_none());
            }
        }
    }
}
