use super::*;
use crate::coordinates::{MapPoint, MapVec};
use crate::element::{ElementData, ElementFx, ElementKind, FxData};
use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

fn inactive_civilian(position: MapPoint) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::default();
        initial_element.kind = ElementKind::ActorCivilian;
        initial_element.active = false;
        initial_element
    };
    element.set_position_map(position);
    Entity::Civilian(crate::element::ActorCivilian {
        element,
        actor: Default::default(),
        human: Default::default(),
        npc: crate::element::NpcData {
            ai: crate::element::AiActorData {
                ai_brain: crate::element::AiBrain::Friendly(Box::default()),
                ..Default::default()
            },
            ..Default::default()
        },
        civilian: Default::default(),
    })
}

fn mobile_fx(index: u16, position: MapPoint) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::default();
        initial_element.kind = ElementKind::Fx;
        initial_element.active = true;
        initial_element
    };
    element.set_position_map(position);
    Entity::Fx(ElementFx {
        element,
        fx: FxData {
            mobile_index: Some(index),
            animation_speed: 1.0,
            ..Default::default()
        },
    })
}

fn path() -> RawHikingPath {
    RawHikingPath {
        waypoints: vec![
            RawWaypoint {
                x: 0,
                y: 0,
                sector: 0,
                level: 0,
                command: WaypointCommand::None,
            },
            RawWaypoint {
                x: 100,
                y: 0,
                sector: 0,
                level: 0,
                command: WaypointCommand::None,
            },
        ],
    }
}

fn speed_macro(speed: f32) -> WaypointCommand {
    let mut data = Vec::new();
    data.extend_from_slice(&1u16.to_le_bytes());
    data.push(0);
    data.extend_from_slice(&5u16.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.push(100);
    data.extend_from_slice(&10u16.to_le_bytes());
    data.extend_from_slice(&5u16.to_le_bytes());
    data.push(129);
    data.extend_from_slice(&speed.to_le_bytes());
    WaypointCommand::Macro(data)
}

fn mobile(children: Vec<EntityId>) -> crate::mobile::MobileElement {
    crate::mobile::MobileElement {
        sprite_ids: children,
        motion_polygon: vec![
            MapPoint::new(0.0, 0.0),
            MapPoint::new(5.0, 0.0),
            MapPoint::new(0.0, 5.0),
        ],
        position: MapPoint::new(0.0, 0.0),
        old_position: MapPoint::new(0.0, 0.0),
        path_index: 0,
        current_waypoint: 1,
        forward: true,
        layer: 0,
        sector: 0,
        obstacle: None,
        active: true,
        stopped: false,
        speed: 2.0,
        speed_goal: 2.0,
        acceleration: 0.0,
        increment: MapVec::new(1.0, 0.0),
        goal: MapPoint::new(100.0, 0.0),
    }
}

#[test]
fn first_child_runs_master_once_and_freeze_all_only_suppresses_child_frames() {
    let sim_context = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.set_actors_frozen(true);
    let first = engine.add_test_entity(mobile_fx(0, MapPoint::new(10.0, 5.0)));
    let second = engine.add_test_entity(mobile_fx(0, MapPoint::new(20.0, 5.0)));
    engine
        .world
        .mobile_elements
        .push(mobile(vec![first, second]));
    let assets = LevelAssets {
        navigation: crate::engine::LevelNavigationAssets {
            hiking_paths: std::sync::Arc::new(vec![path()]),
            ..Default::default()
        },
        ..Default::default()
    };

    let sprite_before =
        serde_json::to_value(&engine.get_entity(first).unwrap().element_data().sprite).unwrap();
    let frame_before = (
        sprite_before["current_frame"].clone(),
        sprite_before["frame_count"].clone(),
    );

    engine.tick_actor_owner_envelopes(&sim_context, &assets);
    assert_eq!(engine.world.mobile_elements[0].position.x, 2.0);
    assert_eq!(
        engine
            .get_entity(first)
            .unwrap()
            .element_data()
            .position_map()
            .x,
        12.0
    );
    assert_eq!(
        engine
            .get_entity(second)
            .unwrap()
            .element_data()
            .position_map()
            .x,
        22.0
    );

    assert_eq!(
        engine.world.mobile_elements[0].position.x, 2.0,
        "later children must not retrigger the master"
    );
    let sprite_after =
        serde_json::to_value(&engine.get_entity(first).unwrap().element_data().sprite).unwrap();
    let frame_after = (
        sprite_after["current_frame"].clone(),
        sprite_after["frame_count"].clone(),
    );
    assert_eq!(
        frame_after, frame_before,
        "FrozenAll must freeze child frames"
    );
}

fn production_walk_mobile_observations(actor_before: bool) -> Vec<f32> {
    let sim_context = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.set_actors_frozen(true);
    if actor_before {
        engine.add_test_entity(inactive_civilian(MapPoint::new(-10.0, 0.0)));
    }
    let child = engine.add_test_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
    if !actor_before {
        engine.add_test_entity(inactive_civilian(MapPoint::new(10.0, 0.0)));
    }
    engine.world.mobile_elements.push(mobile(vec![child]));
    let assets = LevelAssets {
        navigation: crate::engine::LevelNavigationAssets {
            hiking_paths: std::sync::Arc::new(vec![path()]),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut observations = Vec::new();
    engine.tick_actor_owner_envelopes_with_test_owner_hook(
        &sim_context,
        &assets,
        |engine, owner| {
            if engine
                .get_entity(owner)
                .is_some_and(|entity| entity.actor_data().is_some())
            {
                observations.push(engine.world.mobile_elements[0].motion_polygon[0].x);
            }
        },
    );
    observations
}

#[test]
fn production_walk_actor_before_and_after_mobile_observe_old_and_new_geometry() {
    assert_eq!(production_walk_mobile_observations(true), vec![0.0]);
    assert_eq!(production_walk_mobile_observations(false), vec![2.0]);
}

#[test]
fn production_walk_runs_multiple_mobiles_once_across_a_hole_and_visits_spawned_tail() {
    let sim_context = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.set_actors_frozen(true);
    let hole = engine.add_test_entity(Entity::Fx(ElementFx {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Fx;
            initial_element
        },
        fx: FxData::default(),
    }));
    engine.remove_entity(hole);
    let first = engine.add_test_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
    let second = engine.add_test_entity(mobile_fx(1, MapPoint::new(20.0, 0.0)));
    engine.world.mobile_elements.push(mobile(vec![first]));
    engine.world.mobile_elements.push(mobile(vec![second]));
    let assets = LevelAssets {
        navigation: crate::engine::LevelNavigationAssets {
            hiking_paths: std::sync::Arc::new(vec![path()]),
            ..Default::default()
        },
        ..Default::default()
    };
    let visited = std::cell::RefCell::new(Vec::new());
    let spawned = std::cell::Cell::new(None);
    engine.tick_actor_owner_envelopes_with_test_owner_hook(
        &sim_context,
        &assets,
        |engine, owner| {
            visited.borrow_mut().push(owner);
            if owner == first {
                let tail = engine.add_test_entity(Entity::Fx(ElementFx {
                    element: {
                        let mut initial_element = ElementData::default();
                        initial_element.kind = ElementKind::Fx;
                        initial_element
                    },
                    fx: FxData::default(),
                }));
                spawned.set(Some(tail));
            }
        },
    );
    assert_eq!(engine.world.mobile_elements[0].position.x, 2.0);
    assert_eq!(engine.world.mobile_elements[1].position.x, 2.0);
    assert!(visited.borrow().contains(&spawned.get().unwrap()));
}

#[test]
fn mobile_boundary_precedes_static_dispatch_in_live_owner_walk() {
    let sim_context = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let child = engine.add_test_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
    let static_fx = engine.add_test_entity(Entity::Fx(ElementFx {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Fx;
            initial_element.active = false;
            initial_element
        },
        fx: FxData::default(),
    }));
    engine.world.mobile_elements.push(mobile(vec![child]));
    let assets = LevelAssets {
        navigation: crate::engine::LevelNavigationAssets {
            hiking_paths: std::sync::Arc::new(vec![path()]),
            ..Default::default()
        },
        ..Default::default()
    };

    let trace = std::cell::RefCell::new(Vec::new());
    engine.tick_actor_owner_envelopes_with_test_owner_hook(&sim_context, &assets, |_, owner| {
        if owner == child {
            trace.borrow_mut().push("mobile");
            return;
        }
        if owner == static_fx {
            trace.borrow_mut().push("static");
        }
    });
    assert_eq!(*trace.borrow(), vec!["mobile", "static"]);
}

#[test]
fn production_walk_uses_saved_original_creation_order_not_rust_slots() {
    let sim_context = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let first = engine.add_test_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
    let second = engine.add_test_entity(mobile_fx(1, MapPoint::new(10.0, 0.0)));
    let third = engine.add_test_entity(mobile_fx(2, MapPoint::new(20.0, 0.0)));
    for child in [first, second, third] {
        engine.world.mobile_elements.push(mobile(vec![child]));
    }
    engine.world.install_original_creation_orders(
        [(first, 80), (second, 42), (third, 61)]
            .into_iter()
            .collect(),
        81,
    );

    let visited = std::cell::RefCell::new(Vec::new());
    engine.tick_actor_owner_envelopes_with_test_owner_hook(
        &sim_context,
        &LevelAssets {
            navigation: crate::engine::LevelNavigationAssets {
                hiking_paths: std::sync::Arc::new(vec![path()]),
                ..Default::default()
            },
            ..Default::default()
        },
        |_, owner| visited.borrow_mut().push(owner),
    );

    assert_eq!(*visited.borrow(), vec![second, third, first]);
}

#[test]
fn reached_waypoint_uses_old_child_speed_then_new_speed_next_tick() {
    crate::sim_rng::with_seed(17, |sim| {
        let mut engine = EngineInner::new();
        engine.set_actors_frozen(true);
        let child = engine.add_test_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
        let mut owner = mobile(vec![child]);
        owner.speed = 2.0;
        owner.goal = MapPoint::new(2.0, 0.0);
        owner.current_waypoint = 1;
        engine.world.mobile_elements.push(owner);
        let assets = LevelAssets {
            navigation: crate::engine::LevelNavigationAssets {
                hiking_paths: std::sync::Arc::new(vec![RawHikingPath {
                    waypoints: vec![
                        RawWaypoint {
                            x: 0,
                            y: 0,
                            sector: 0,
                            level: 0,
                            command: WaypointCommand::None,
                        },
                        RawWaypoint {
                            x: 2,
                            y: 0,
                            sector: 0,
                            level: 0,
                            command: speed_macro(3.0),
                        },
                    ],
                }]),
                ..Default::default()
            },
            ..Default::default()
        };

        let ((_, trace), increments) =
            crate::engine::movement::capture_mobile_crossing_increments(|| {
                crate::sim_rng::with_draw_trace(|| {
                    engine.tick_mobile_child_owner_boundary(sim, &assets, child);
                })
            });
        assert_eq!(increments.last().copied(), Some(MapVec::new(1.0, 0.0)));
        assert_eq!(
            engine.world.mobile_elements[0].increment,
            MapVec::new(-1.0, 0.0)
        );
        assert_eq!(
            trace,
            vec![crate::sim_rng::RngSite::MobileWaypointProbability]
        );
        assert_eq!(engine.world.mobile_elements[0].speed, 3.0);
        let child_fx = engine
            .get_entity(child)
            .and_then(Entity::as_fx)
            .expect("mobile child remains FX");
        assert!(child_fx.element.active);
        assert_eq!(
            child_fx.fx.animation_speed, 0.5,
            "this child update must retain the movement-frame speed"
        );

        engine.tick_mobile_child_owner_boundary(sim, &assets, child);
        assert_eq!(engine.world.mobile_elements[0].speed, 3.0);
        let next_speed = engine
            .get_entity(child)
            .and_then(Entity::as_fx)
            .expect("mobile child remains FX")
            .fx
            .animation_speed;
        assert!(
            (next_speed - 1.0 / 3.0).abs() < f32::EPSILON,
            "the waypoint speed must become child modulation on the next Update"
        );
    });
}

#[test]
fn stopped_master_returns_before_crossing_and_never_replays_old_position_delta() {
    let sim_context = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.set_actors_frozen(true);
    let child = engine.add_test_entity(mobile_fx(0, MapPoint::new(10.0, 0.0)));
    let mut owner = mobile(vec![child]);
    owner.position = MapPoint::new(20.0, 0.0);
    owner.old_position = MapPoint::new(0.0, 0.0);
    owner.stopped = true;
    engine.world.mobile_elements.push(owner);
    let assets = LevelAssets {
        navigation: crate::engine::LevelNavigationAssets {
            hiking_paths: std::sync::Arc::new(vec![path()]),
            ..Default::default()
        },
        ..Default::default()
    };
    let (_, increments) = crate::engine::movement::capture_mobile_crossing_increments(|| {
        engine.tick_mobile_child_owner_boundary(&sim_context, &assets, child);
    });
    assert_eq!(
        engine
            .get_entity(child)
            .unwrap()
            .element_data()
            .position_map()
            .x,
        10.0
    );
    assert_eq!(increments, []);

    engine.world.mobile_elements[0].stopped = false;
    engine.world.mobile_elements[0].active = false;
    engine.world.mobile_elements[0].old_position = MapPoint::new(-30.0, 0.0);
    let (_, increments) = crate::engine::movement::capture_mobile_crossing_increments(|| {
        engine.tick_mobile_child_owner_boundary(&sim_context, &assets, child);
    });
    assert_eq!(
        engine
            .get_entity(child)
            .unwrap()
            .element_data()
            .position_map()
            .x,
        10.0
    );
    assert_eq!(increments, []);
}

#[test]
#[should_panic(expected = "wrong master index")]
fn first_child_boundary_rejects_a_later_child_with_the_wrong_mobile_index() {
    let sim_context = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let first = engine.add_test_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
    let second = engine.add_test_entity(mobile_fx(1, MapPoint::new(0.0, 0.0)));
    engine
        .world
        .mobile_elements
        .push(mobile(vec![first, second]));
    let assets = LevelAssets {
        navigation: crate::engine::LevelNavigationAssets {
            hiking_paths: std::sync::Arc::new(vec![path()]),
            ..Default::default()
        },
        ..Default::default()
    };

    engine.tick_mobile_child_owner_boundary(&sim_context, &assets, first);
}
