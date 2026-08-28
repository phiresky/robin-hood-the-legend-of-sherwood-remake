//! Main per-frame update tick (`perform_hourglass`).

mod deferred_outcomes;

use super::movement::{CompletedPathWork, PathScheduleContext};
#[cfg(test)]
use super::sequence_runtime::{
    DirectAbilityCommandContext, LiftWaitCommandContext, NpcAttentionCommandContext,
    NpcStateCommandContext, OwnerActionBarrier, PositionAssertionContext, StealthCommandContext,
    TurnCommandContext, WaitCommandContext,
};
use super::sequence_runtime::{required_canonical_door, required_canonical_door_mut};
use super::*;
use crate::abilities;
use crate::element::{Command, Entity, EntityId};
use crate::entities::EntitySlots;
use crate::game_operation::GameCode;
use crate::messenger::{MessageType, SimpleMessage};
use crate::profiles::MissionType;

/// Strict opt-in gate for the Drop Execute-boundary diagnostic.
fn drop_owner_boundary_matches(frame: u32, owner: EntityId) -> bool {
    if std::env::var_os("PARITY_DEBUG_DROP_BOUNDARY").is_none() {
        return false;
    }
    let owner_filter = std::env::var("PARITY_DEBUG_DROP_OWNER").unwrap_or_else(|_| {
        panic!("PARITY_DEBUG_DROP_BOUNDARY requires PARITY_DEBUG_DROP_OWNER=pc:INDEX")
    });
    let (kind, index) = owner_filter
        .split_once(':')
        .unwrap_or_else(|| panic!("PARITY_DEBUG_DROP_OWNER must look like pc:INDEX"));
    assert_eq!(kind, "pc", "PARITY_DEBUG_DROP_OWNER only accepts PC owners");
    let index = index.parse::<u32>().unwrap_or_else(|error| {
        panic!("invalid PARITY_DEBUG_DROP_OWNER={owner_filter:?}: {error}")
    });
    if !matches!(owner, EntityId::Pc(_)) || owner.index() != index {
        return false;
    }
    let parse_frame = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for Drop boundary diagnostic: {error}")
            })
        })
    };
    if let Some(exact) = parse_frame("PARITY_DEBUG_DROP_FRAME") {
        return frame == exact;
    }
    let from = parse_frame("PARITY_DEBUG_DROP_FROM").unwrap_or_else(|| {
        panic!(
            "PARITY_DEBUG_DROP_BOUNDARY requires PARITY_DEBUG_DROP_FRAME or PARITY_DEBUG_DROP_FROM"
        )
    });
    let until = parse_frame("PARITY_DEBUG_DROP_UNTIL").unwrap_or(from);
    assert!(
        from <= until,
        "PARITY_DEBUG_DROP_FROM must not exceed PARITY_DEBUG_DROP_UNTIL"
    );
    (from..=until).contains(&frame)
}

#[cfg(test)]
mod restored_pass_door_completion_tests {
    use super::*;
    use crate::element::{
        ActionState, ActorData, ActorPc, ElementData, ElementKind, Entity, HumanData, PcData,
        Posture,
    };
    use crate::order::OrderType;

    fn airborne_pc() -> Entity {
        Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Flying,
                ..ElementData::default()
            },
            actor: ActorData {
                action_state: ActionState::Moving,
                active_door_pass: None,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    #[test]
    fn restored_crenel_exit_completes_without_active_door_pass() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(airborne_pc());
        engine.apply_door_pass_transition_completion_side_effects(
            &LevelAssets::new(),
            owner,
            OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
        );

        let pc = engine.get_entity(owner).unwrap();
        assert_eq!(pc.element_data().posture, Posture::Crouched);
        assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Waiting);
        assert!(pc.actor_data().unwrap().active_door_pass.is_none());
    }

    #[test]
    fn unrelated_transition_still_requires_active_door_pass() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(airborne_pc());
        engine.apply_door_pass_transition_completion_side_effects(
            &LevelAssets::new(),
            owner,
            OrderType::TransitionClimbingWallDownWaitingUpright,
        );

        let pc = engine.get_entity(owner).unwrap();
        assert_eq!(pc.element_data().posture, Posture::Flying);
        assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Moving);
    }
}

#[cfg(test)]
thread_local! {
    static PROJECTILE_DERIVED_TAIL_TRACE: std::cell::RefCell<Option<Vec<(EntityId, crate::element::ObjectType)>>> =
        const { std::cell::RefCell::new(None) };
}

pub(super) fn observe_projectile_derived_tail(
    id: EntityId,
    object_type: crate::element::ObjectType,
) {
    tracing::trace!(
        target: "robin_engine::engine::tick::projectile_tail",
        ?id,
        ?object_type,
        "projectile derived tail"
    );
    #[cfg(test)]
    PROJECTILE_DERIVED_TAIL_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push((id, object_type));
        }
    });
}

#[cfg(test)]
pub(super) fn capture_projectile_derived_tails<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<(EntityId, crate::element::ObjectType)>) {
    PROJECTILE_DERIVED_TAIL_TRACE.with(|trace| {
        assert!(trace.borrow().is_none(), "tail capture is not re-entrant");
        *trace.borrow_mut() = Some(Vec::new());
    });
    let result = f();
    let tails = PROJECTILE_DERIVED_TAIL_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("tail capture must remain active")
    });
    (result, tails)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NpcHourglassPhase {
    SoldierPrelude,
    Patrol,
    BaseHuman,
    Broadcasts,
    View,
    Detection,
    Ambush,
    Busy,
    Ladder,
    LockGate,
    SixteenthFrame,
    NormalTimer,
    MacroTimer,
    QueuedStimuli,
}

#[cfg(test)]
mod mobile_owner_boundary_tests {
    use super::*;
    use crate::coordinates::{MapPoint, MapVec};
    use crate::element::{ElementData, ElementFx, ElementKind, FxData};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    fn inactive_civilian(position: MapPoint) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorCivilian,
            active: false,
            ..Default::default()
        };
        element.set_position_map(position);
        Entity::Civilian(crate::element::ActorCivilian {
            element,
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            civilian: Default::default(),
        })
    }

    fn mobile_fx(index: u16, position: MapPoint) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::Fx,
            active: true,
            ..Default::default()
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
        let first = engine.add_entity(mobile_fx(0, MapPoint::new(10.0, 5.0)));
        let second = engine.add_entity(mobile_fx(0, MapPoint::new(20.0, 5.0)));
        engine
            .world
            .mobile_elements
            .push(mobile(vec![first, second]));
        let mut assets = LevelAssets::default();
        assets.hiking_paths = std::sync::Arc::new(vec![path()]);

        let sprite_before =
            serde_json::to_value(&engine.get_entity(first).unwrap().element_data().sprite).unwrap();
        let frame_before = (
            sprite_before["current_frame"].clone(),
            sprite_before["frame_count"].clone(),
        );
        let positions = EntitySlots::filled(engine.world.entities.len(), None);
        engine.tick_actor_owner_envelopes(&sim_context, &assets, &positions);
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
            engine.add_entity(inactive_civilian(MapPoint::new(-10.0, 0.0)));
        }
        let child = engine.add_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
        if !actor_before {
            engine.add_entity(inactive_civilian(MapPoint::new(10.0, 0.0)));
        }
        engine.world.mobile_elements.push(mobile(vec![child]));
        let mut assets = LevelAssets::default();
        assets.hiking_paths = std::sync::Arc::new(vec![path()]);
        let mut observations = Vec::new();
        engine.tick_actor_animation_action_change_slots_with_hooks(
            &sim_context,
            &assets,
            |engine, owner| {
                engine.tick_mobile_child_owner_boundary(&sim_context, &assets, owner);
            },
            |engine, _| {
                observations.push(engine.first_live_mobile_polygon_point(0).x);
            },
            |_, _, _, _, _, _, _| {},
            |_, _, _| {},
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
        let hole = engine.add_entity(Entity::Fx(ElementFx {
            element: ElementData {
                kind: ElementKind::Fx,
                ..Default::default()
            },
            fx: FxData::default(),
        }));
        engine.remove_entity(hole);
        let first = engine.add_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
        let second = engine.add_entity(mobile_fx(1, MapPoint::new(20.0, 0.0)));
        engine.world.mobile_elements.push(mobile(vec![first]));
        engine.world.mobile_elements.push(mobile(vec![second]));
        let mut assets = LevelAssets::default();
        assets.hiking_paths = std::sync::Arc::new(vec![path()]);
        let visited = std::cell::RefCell::new(Vec::new());
        let spawned = std::cell::Cell::new(None);
        engine.tick_actor_animation_action_change_slots_with_hooks(
            &sim_context,
            &assets,
            |engine, owner| {
                visited.borrow_mut().push(owner);
                engine.tick_mobile_child_owner_boundary(&sim_context, &assets, owner);
                if owner == first {
                    let tail = engine.add_entity(Entity::Fx(ElementFx {
                        element: ElementData {
                            kind: ElementKind::Fx,
                            ..Default::default()
                        },
                        fx: FxData::default(),
                    }));
                    spawned.set(Some(tail));
                }
            },
            |_, _| {},
            |_, _, _, _, _, _, _| {},
            |_, _, _| {},
        );
        assert_eq!(engine.world.mobile_elements[0].position.x, 2.0);
        assert_eq!(engine.world.mobile_elements[1].position.x, 2.0);
        assert!(visited.borrow().contains(&spawned.get().unwrap()));
    }

    #[test]
    fn mobile_boundary_precedes_static_dispatch_in_live_owner_walk() {
        let sim_context = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let child = engine.add_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
        let static_fx = engine.add_entity(Entity::Fx(ElementFx {
            element: ElementData {
                kind: ElementKind::Fx,
                active: false,
                ..Default::default()
            },
            fx: FxData::default(),
        }));
        engine.world.mobile_elements.push(mobile(vec![child]));
        let mut assets = LevelAssets::default();
        assets.hiking_paths = std::sync::Arc::new(vec![path()]);

        let trace = std::cell::RefCell::new(Vec::new());
        engine.tick_actor_animation_action_change_slots_with_hooks(
            &sim_context,
            &assets,
            |engine, owner| {
                if engine.tick_mobile_child_owner_boundary(&sim_context, &assets, owner) {
                    trace.borrow_mut().push("mobile");
                    return;
                }
                if owner == static_fx {
                    trace.borrow_mut().push("static");
                }
            },
            |_, _| {},
            |_, _, _, _, _, _, _| {},
            |_, _, _| {},
        );
        assert_eq!(*trace.borrow(), vec!["mobile", "static"]);
    }

    #[test]
    fn production_walk_uses_saved_original_creation_order_not_rust_slots() {
        let sim_context = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let first = engine.add_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
        let second = engine.add_entity(mobile_fx(1, MapPoint::new(10.0, 0.0)));
        let third = engine.add_entity(mobile_fx(2, MapPoint::new(20.0, 0.0)));
        engine.world.install_original_creation_orders(
            [(first, 80), (second, 42), (third, 61)]
                .into_iter()
                .collect(),
            81,
        );

        let visited = std::cell::RefCell::new(Vec::new());
        engine.tick_actor_animation_action_change_slots_with_hooks(
            &sim_context,
            &LevelAssets::default(),
            |_, owner| visited.borrow_mut().push(owner),
            |_, _| {},
            |_, _, _, _, _, _, _| {},
            |_, _, _| {},
        );

        assert_eq!(*visited.borrow(), vec![second, third, first]);
    }

    #[test]
    fn reached_waypoint_uses_old_child_speed_then_new_speed_next_tick() {
        crate::sim_rng::with_seed(17, |sim| {
            let mut engine = EngineInner::new();
            engine.set_actors_frozen(true);
            let child = engine.add_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
            let mut owner = mobile(vec![child]);
            owner.speed = 2.0;
            owner.goal = MapPoint::new(2.0, 0.0);
            owner.current_waypoint = 1;
            engine.world.mobile_elements.push(owner);
            let mut assets = LevelAssets::default();
            assets.hiking_paths = std::sync::Arc::new(vec![RawHikingPath {
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
            }]);

            let (_, trace) = crate::sim_rng::with_draw_trace(|| {
                engine.tick_mobile_child_owner_boundary(sim, &assets, child);
            });
            assert_eq!(
                super::super::movement::take_last_mobile_crossing_increment(),
                Some(MapVec::new(1.0, 0.0))
            );
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
                "this child Hourglass must retain the movement-frame speed"
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
        let child = engine.add_entity(mobile_fx(0, MapPoint::new(10.0, 0.0)));
        let mut owner = mobile(vec![child]);
        owner.position = MapPoint::new(20.0, 0.0);
        owner.old_position = MapPoint::new(0.0, 0.0);
        owner.stopped = true;
        engine.world.mobile_elements.push(owner);
        let mut assets = LevelAssets::default();
        assets.hiking_paths = std::sync::Arc::new(vec![path()]);
        let _ = super::super::movement::take_last_mobile_crossing_increment();

        engine.tick_mobile_child_owner_boundary(&sim_context, &assets, child);
        assert_eq!(
            engine
                .get_entity(child)
                .unwrap()
                .element_data()
                .position_map()
                .x,
            10.0
        );
        assert_eq!(
            super::super::movement::take_last_mobile_crossing_increment(),
            None
        );

        engine.world.mobile_elements[0].stopped = false;
        engine.world.mobile_elements[0].active = false;
        engine.world.mobile_elements[0].old_position = MapPoint::new(-30.0, 0.0);
        engine.tick_mobile_child_owner_boundary(&sim_context, &assets, child);
        assert_eq!(
            engine
                .get_entity(child)
                .unwrap()
                .element_data()
                .position_map()
                .x,
            10.0
        );
        assert_eq!(
            super::super::movement::take_last_mobile_crossing_increment(),
            None
        );
    }

    #[test]
    #[should_panic(expected = "wrong master index")]
    fn first_child_boundary_rejects_a_later_child_with_the_wrong_mobile_index() {
        let sim_context = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let first = engine.add_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
        let second = engine.add_entity(mobile_fx(1, MapPoint::new(0.0, 0.0)));
        engine
            .world
            .mobile_elements
            .push(mobile(vec![first, second]));
        let mut assets = LevelAssets::default();
        assets.hiking_paths = std::sync::Arc::new(vec![path()]);

        engine.tick_mobile_child_owner_boundary(&sim_context, &assets, first);
    }
}

#[cfg(test)]
mod generic_actor_line_crossing_tests {
    use super::*;
    use crate::coordinates::{MapPoint, MapVec};
    use crate::element::{
        ActionState, ActorData, ActorSoldier, ElementData, ElementKind, HumanData, NpcData,
        Posture, SoldierData,
    };
    use crate::fast_find_grid::GridLine;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    fn dying_sprite() -> crate::sprite::Sprite {
        let action = OrderType::DyingSword;
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;
        crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(conversion),
        )
    }

    fn dying_find_place_increment_after_crossing(
        patch_line_count: usize,
        precompute_increment: bool,
    ) -> (MapVec, MapVec, MapPoint) {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(4, 4);
        engine.world.fast_grid_mut().allocate_layers(1);

        // The lying box centered at (130,130) straddles this solid edge.
        // FindPlaceToDie pushes it toward the click side (+Y), producing a
        // real generic-Execute movement segment.
        engine.world.fast_grid_mut().add_line(
            GridLine::new(
                MapPoint::new(100.0, 128.0),
                MapPoint::new(160.0, 128.0),
                true,
            ),
            0,
        );
        for offset in 0..patch_line_count {
            engine.world.fast_grid_mut().add_line(
                GridLine::new_patch(
                    MapPoint::new(100.0, 131.0 + offset as f32),
                    MapPoint::new(160.0, 131.0 + offset as f32),
                    crate::patch::PatchIndex::new(offset as u32)
                        .expect("test patch index is representable"),
                ),
                0,
            );
        }
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            sprite: dying_sprite(),
            ..ElementData::default()
        };
        element.set_position_map(MapPoint::new(130.0, 130.0));
        // Aim horizontally before corpse placement so the relocation's +Y
        // displacement makes a successful post-cross recompute observably
        // different from the cached pre-Execute increment.
        element
            .sprite
            .position_iface
            .set_map_goal(MapPoint::new(200.0, 130.0));
        if precompute_increment {
            element.sprite.position_iface.compute_increment_all(false);
        }
        element.set_direction_instantly(13);
        let stale_increment = element.sprite.position_iface.raw_increment_map();
        let owner = engine.add_entity(Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData {
                action_state: ActionState::WaitingSword,
                ..ActorData::default()
            },
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        }));

        let mut dying = SequenceElement::new(1, Command::ReceiveSwordDamage, Some(owner));
        let mut order = Order::test_new(OrderType::DyingSword, 0.0, 0.0);
        order.compute_direction = false;
        dying.orders.push_back(order);
        let sequence_id = engine.orders.sequence_manager.launch_element(dying);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);

        engine.tick_actor_animation_action_change_slots(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
        );

        let entity = engine.get_entity(owner).expect("dying owner remains live");
        let old_position = entity.position_iface().old_map_position();
        let new_position = entity.element_data().position_map();
        let non_elevation_crossing_count = engine
            .world
            .fast_grid
            .get_actor_non_elevation_crossing_line_indices(0, old_position, new_position)
            .len();
        assert_eq!(old_position, MapPoint::new(130.0, 130.0));
        assert_eq!(non_elevation_crossing_count, patch_line_count);
        assert!(
            new_position.y > 132.0,
            "FindPlaceToDie must cross both synthetic boundaries"
        );
        (
            stale_increment,
            entity.position_iface().raw_increment_map(),
            new_position,
        )
    }

    #[test]
    fn find_place_to_die_multi_line_crossing_computes_uncached_increment() {
        let (stale, recomputed, position) = dying_find_place_increment_after_crossing(2, false);
        assert_ne!(recomputed, stale, "corpse relocation ended at {position:?}");
        let dx = 200.0 - position.x;
        let dy = 130.0 - position.y;
        let norm = (dx * dx + dy * dy).sqrt();
        let expected = MapVec::new(dx / norm, dy / norm);
        assert!((recomputed.x - expected.x).abs() < 1.0e-6);
        assert!((recomputed.y - expected.y).abs() < 1.0e-6);
    }

    #[test]
    fn find_place_to_die_single_non_elevation_crossing_retains_increment() {
        let (stale, retained, _) = dying_find_place_increment_after_crossing(1, true);
        assert_eq!(retained, stale);
    }

    #[test]
    fn find_place_to_die_multi_non_elevation_crossing_retains_cached_increment() {
        let (stale, retained, _) = dying_find_place_increment_after_crossing(2, true);
        assert_eq!(retained, stale);
    }

    #[test]
    fn delayed_position_multi_non_elevation_crossing_recomputes_invalid_increment() {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(4, 4);
        engine.world.fast_grid_mut().allocate_layers(1);
        for (offset, patch_index) in [(131.0, 0), (132.0, 1)] {
            engine.world.fast_grid_mut().add_line(
                GridLine::new_patch(
                    MapPoint::new(100.0, offset),
                    MapPoint::new(160.0, offset),
                    crate::patch::PatchIndex::new(patch_index)
                        .expect("test patch index is representable"),
                ),
                0,
            );
        }

        let stale = MapVec::new(1.0, 0.0);
        let destination = MapPoint::new(130.0, 134.0);
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Tied,
            ..ElementData::default()
        };
        element.set_position_map(MapPoint::new(130.0, 130.0));
        element.sprite.position_iface.set_map_increment(stale);
        // The outgoing movement condolence writes the zero idle goal and
        // invalidates the cached increment before corpse placement commits.
        element.sprite.position_iface.set_map_goal(MapPoint::ZERO);
        element.set_position_map_delayed(destination);
        let owner = engine.add_entity(Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData::default(),
            human: HumanData {
                unconscious: true,
                ..HumanData::default()
            },
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        }));

        let mut wait = SequenceElement::new(1, Command::Wait, Some(owner));
        let mut order = Order::test_new(OrderType::BeingTied, 0.0, 0.0);
        order.compute_direction = false;
        wait.orders.push_back(order);
        let sequence_id = engine.orders.sequence_manager.launch_element(wait);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);

        let crossing_count = engine
            .world
            .fast_grid
            .get_actor_crossing_line_indices(0, MapPoint::new(130.0, 130.0), destination)
            .len();
        let elevation_count = engine
            .world
            .fast_grid
            .get_crossing_elevation_line_indices(0, MapPoint::new(130.0, 130.0), destination)
            .len();
        assert_eq!(crossing_count, 2);
        assert_eq!(elevation_count, 0);

        engine.apply_delayed_actor_position(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
        );

        let position = engine
            .get_entity(owner)
            .expect("delayed-position owner remains live")
            .position_iface();
        let recomputed = position.get_increment_map();
        let dx = -destination.x;
        let dy = -destination.y;
        let norm = (dx * dx + dy * dy).sqrt();
        let expected = MapVec::new(dx / norm, dy / norm);
        assert_ne!(recomputed, stale);
        assert!((recomputed.x - expected.x).abs() < 1.0e-6);
        assert!((recomputed.y - expected.y).abs() < 1.0e-6);
    }
}

#[cfg(test)]
thread_local! {
    static NPC_HOURGLASS_PHASE_TRACE: std::cell::RefCell<Option<Vec<NpcHourglassPhase>>> =
        const { std::cell::RefCell::new(None) };
}

fn observe_npc_hourglass_phase(phase: NpcHourglassPhase) {
    tracing::trace!(
        target: "robin_engine::engine::tick::npc_phases",
        ?phase,
        "npc hourglass phase"
    );
    #[cfg(test)]
    NPC_HOURGLASS_PHASE_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push(phase);
        }
    });
}

#[cfg(test)]
pub(super) fn capture_npc_hourglass_phases<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<NpcHourglassPhase>) {
    NPC_HOURGLASS_PHASE_TRACE.with(|trace| {
        assert!(trace.borrow().is_none(), "phase capture is not re-entrant");
        *trace.borrow_mut() = Some(Vec::new());
    });
    let result = f();
    let phases = NPC_HOURGLASS_PHASE_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("phase capture must remain active")
    });
    (result, phases)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ActorAnimationBoundaryPhase {
    WaitReady(EntityId),
    GenericExecute(EntityId),
    CompletionEffects(EntityId),
    CombatInjuryThink(EntityId),
    ActionChange(EntityId),
}

#[cfg(test)]
thread_local! {
    static ACTOR_ANIMATION_BOUNDARY_TRACE: std::cell::RefCell<Option<Vec<ActorAnimationBoundaryPhase>>> =
        const { std::cell::RefCell::new(None) };
}

fn observe_actor_animation_boundary(phase: ActorAnimationBoundaryPhase) {
    tracing::trace!(
        target: "robin_engine::engine::tick::actor_animation_boundary",
        ?phase,
        "actor animation boundary"
    );
    #[cfg(test)]
    ACTOR_ANIMATION_BOUNDARY_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push(phase);
        }
    });
}

#[cfg(test)]
pub(super) fn capture_actor_animation_boundary<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<ActorAnimationBoundaryPhase>) {
    ACTOR_ANIMATION_BOUNDARY_TRACE.with(|trace| {
        assert!(
            trace.borrow_mut().replace(Vec::new()).is_none(),
            "actor animation boundary capture is not re-entrant"
        );
    });
    let result = f();
    let phases = ACTOR_ANIMATION_BOUNDARY_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("actor animation boundary capture must remain active")
    });
    (result, phases)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ActorOwnerEnvelopePhase {
    SoldierPrelude(EntityId),
    Patrol(EntityId),
    HumanPrelude(EntityId),
    BaseActor(EntityId),
    MovementExecute(EntityId),
    HumanNoise(EntityId),
    HumanTiredness(EntityId),
    PcTail(EntityId),
    NpcTail(EntityId),
}

#[cfg(test)]
thread_local! {
    static ACTOR_OWNER_ENVELOPE_TRACE: std::cell::RefCell<Option<Vec<ActorOwnerEnvelopePhase>>> =
        const { std::cell::RefCell::new(None) };
}

fn observe_actor_owner_envelope(phase: ActorOwnerEnvelopePhase) {
    tracing::trace!(
        target: "robin_engine::engine::tick::actor_owner_envelope",
        ?phase,
        "actor owner envelope"
    );
    #[cfg(test)]
    ACTOR_OWNER_ENVELOPE_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push(phase);
        }
    });
}

#[cfg(test)]
pub(super) fn capture_actor_owner_envelope<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<ActorOwnerEnvelopePhase>) {
    ACTOR_OWNER_ENVELOPE_TRACE.with(|trace| {
        assert!(
            trace.borrow_mut().replace(Vec::new()).is_none(),
            "actor-owner envelope capture is not re-entrant"
        );
    });
    let result = f();
    let phases = ACTOR_OWNER_ENVELOPE_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("actor-owner envelope capture must remain active")
    });
    (result, phases)
}

/// Exact base-Actor Execute identity selected at entry to one legacy slot.
///
/// The coordinator carries the selected Original sequence/element/order
/// identity and revalidates it immediately before dispatch because an earlier
/// synchronous callback in the same actor slot may replace that order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) struct MeleeOwnerSelection {
    pub(in crate::engine) seq_id: crate::sequence::SequenceId,
    pub(in crate::engine) elem_idx: usize,
    pub(in crate::engine) order_id: std::num::NonZeroU32,
}

pub(super) const MELEE_ORDERS: &[crate::order::OrderType] = &[
    crate::order::OrderType::StrikingStraightSword,
    crate::order::OrderType::StrikingStraightStrongSword,
    crate::order::OrderType::ExecutingSword,
    crate::order::OrderType::StrikingLeftSword,
    crate::order::OrderType::StrikingRightSword,
    crate::order::OrderType::StrikingSemiroundLeftSword,
    crate::order::OrderType::StrikingSemiroundRightSword,
    crate::order::OrderType::StrikingRoundLeftSword,
    crate::order::OrderType::StrikingRoundRightSword,
];

/// Concrete Original override whose switch contributes an Execute arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ExecuteOverride {
    Actor,
    Human,
    Pc,
    Npc,
    Soldier,
    Civilian,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ExecuteOwnerFamily {
    GenericAnimation,
    Movement,
    Melee,
    Bow,
    Ability,
    Beggar,
    WaitingSword,
}

/// Whether the entry-latched Human::Execute arm reaches the synchronous
/// `WAITING_SWORD` swordfight work.
///
/// Original keys this work to the selected order after the common execution-
/// freeze and validity exits; it is not conditional on a later sprite helper
/// returning a completion record (`RHelementactorhuman.cpp:3729-3741`).
fn waiting_sword_execute_reaches_evaluation(
    selected_order_type: Option<crate::order::OrderType>,
    validity_short_circuited: bool,
    execution_frozen: bool,
) -> bool {
    selected_order_type == Some(crate::order::OrderType::WaitingSword)
        && !validity_short_circuited
        && !execution_frozen
}

#[cfg(test)]
#[test]
fn waiting_sword_evaluation_follows_entry_latched_execute_arm() {
    use crate::order::OrderType;

    assert!(waiting_sword_execute_reaches_evaluation(
        Some(OrderType::WaitingSword),
        false,
        false,
    ));
    assert!(!waiting_sword_execute_reaches_evaluation(
        Some(OrderType::WaitingSword),
        true,
        false,
    ));
    assert!(!waiting_sword_execute_reaches_evaluation(
        Some(OrderType::WaitingSword),
        false,
        true,
    ));
    assert!(!waiting_sword_execute_reaches_evaluation(
        Some(OrderType::WaitingUpright),
        false,
        false,
    ));
}

/// Return the canonical order installed by `TranslateCommand` for the active
/// ability.  Execute-owner selection must match that order, even when an
/// ability later asks the sprite to perform a different animation.  In
/// particular, Original installs `RHANIMATION_HEALING` for every Heal command
/// and substitutes `RHANIMATION_EATING` only inside PC::Execute for self-heal.
fn active_ability_order_type(actor: &crate::element::ActorData) -> Option<crate::order::OrderType> {
    use crate::element::{ListenPhase, ReceivePursePhase};
    use crate::movement::AbilityKind;
    use crate::order::OrderType;

    match actor.active_ability.kind? {
        AbilityKind::Listen => match actor.listen_phase {
            ListenPhase::EnterTransition => Some(OrderType::TransitionWaitingUprightListening),
            ListenPhase::CountingDown => Some(OrderType::Listening),
            ListenPhase::ExitTransition => Some(OrderType::TransitionListeningWaitingUpright),
            ListenPhase::Inactive => None,
        },
        AbilityKind::ReceivePurse => match actor.receive_purse_phase {
            ReceivePursePhase::Receiving => Some(OrderType::ReceivingPurse),
            ReceivePursePhase::Waiting => Some(OrderType::WaitingWithPurse),
            ReceivePursePhase::Transition => {
                Some(OrderType::TransitionWaitingWithPurseWaitingUpright)
            }
            ReceivePursePhase::Inactive => None,
        },
        kind => Some(crate::abilities::ability_order_type(kind)),
    }
}

#[cfg(test)]
mod active_ability_owner_selection_tests {
    use super::active_ability_order_type;
    use crate::element::{ActorData, EntityId, EntityIdKind};
    use crate::movement::AbilityKind;
    use crate::order::OrderType;

    #[test]
    fn self_heal_keeps_the_canonical_healing_order_for_owner_selection() {
        let healer = EntityId::new(172, EntityIdKind::Pc);
        let mut actor = ActorData::default();
        actor.active_ability.kind = Some(AbilityKind::Heal);
        actor.active_ability.target = Some(healer);

        assert_eq!(active_ability_order_type(&actor), Some(OrderType::Healing));
    }
}

macro_rules! actor_execute_arm_catalog {
    ($emit:ident) => {
        $emit! {
            (Actor, WaitingUpright, GenericAnimation),
            (Actor, WaitingUprightBored, GenericAnimation),
            (Actor, WaitingUprightBoredRandom, GenericAnimation),
            (Actor, TransitionWaitingUprightBoredWaitingUpright, GenericAnimation),
            (Actor, TransitionWaitingUprightWaitingUprightBored, GenericAnimation),
            (Actor, TransitionWalkingUprightWaitingUpright, Movement),
            (Actor, TransitionRunningUprightWaitingUpright, Movement),
            (Actor, TransitionWaitingUprightWalkingUpright, Movement),
            (Actor, TransitionWaitingUprightRunningUpright, Movement),
            (Actor, TransitionWalkingUprightRunningUpright, Movement),
            (Actor, TransitionRunningUprightWalkingUpright, Movement),
            (Actor, TransitionWaitingCrouchedWalkingCrouched, Movement),
            (Actor, TransitionWalkingCrouchedWaitingCrouched, Movement),
            (Actor, TransitionCrouchingDown, GenericAnimation),
            (Actor, TransitionCrouchingUp, GenericAnimation),
            (Actor, TransitionWalkingUprightWalkingCrouched, Movement),
            (Actor, TransitionWalkingCrouchedWalkingUpright, Movement),
            (Actor, TransitionRunningUprightWalkingCrouched, Movement),
            (Actor, TransitionWalkingCrouchedRunningUpright, Movement),
            (Actor, Turning, GenericAnimation),
            (Actor, Freezing, GenericAnimation),
            (Actor, ClimbingLadderUp, Movement),
            (Actor, ClimbingLadderUpAlerted, Movement),
            (Actor, ClimbingLadderDown, Movement),
            (Actor, ClimbingLadderDownAlerted, Movement),
            (Actor, ClimbingLadderDownFast, Movement),
            (Actor, ClimbingLadderUpFast, Movement),
            (Actor, TransitionClimbingLadderUpWaitingCrouched, Movement),
            (Actor, TransitionClimbingLadderUpWaitingUprightAlerted, Movement),
            (Actor, TransitionWaitingCrouchedClimbingLadderDown, Movement),
            (Actor, TransitionWaitingUprightClimbingLadderDownAlerted, Movement),
            (Actor, TransitionWaitingUprightClimbingLadderUp, Movement),
            (Actor, TransitionWaitingUprightClimbingLadderUpAlerted, Movement),
            (Actor, TransitionClimbingLadderDownWaitingUpright, Movement),
            (Actor, TransitionClimbingLadderDownWaitingUprightAlerted, Movement),
            (Actor, ClimbingWallUp, Movement),
            (Actor, ClimbingWallDown, Movement),
            (Actor, ClimbingWallDownFast, Movement),
            (Actor, ClimbingWallUpFast, Movement),
            (Actor, TransitionClimbingWallUpWaitingCrouched, Movement),
            (Actor, TransitionClimbingWallUpWaitingCrouchedCrenel, Movement),
            (Actor, TransitionWaitingCrouchedClimbingWallDown, Movement),
            (Actor, TransitionWaitingCrouchedClimbingWallDownCrenel, Movement),
            (Actor, TransitionWaitingUprightClimbingWallUp, Movement),
            (Actor, TransitionClimbingWallDownWaitingUpright, Movement),
            (Actor, WalkingUpright, Movement),
            (Actor, RunningUpright, Movement),
            (Actor, WalkingStairs, Movement),
            (Actor, RunningStairs, Movement),
            (Actor, PassingDoor, Movement),
            (Actor, WaitingFreeLift, GenericAnimation),
            (Actor, PlayCustom, GenericAnimation),
            (Actor, PlayCustomFreeze, GenericAnimation),
            (Actor, PlayCustomFrozen, GenericAnimation),
            (Actor, PlayCustomLooped, GenericAnimation),
            (Actor, RefreshingSeek, Movement),
            (Human, Select, GenericAnimation),
            (Human, TransitionEquipBow, Bow),
            (Human, TransitionEquipBowAnonymous, Bow),
            (Human, TransitionUnequipBow, Bow),
            (Human, TransitionUnequipBowAnonymous, Bow),
            (Human, AimingWithBow, GenericAnimation),
            (Human, AimingWithBowAnonymous, GenericAnimation),
            (Human, AimingWithBowUp, GenericAnimation),
            (Human, AimingWithBowUpAnonymous, GenericAnimation),
            (Human, TransitionLoadingBow, Bow),
            (Human, TransitionLoadingBowAnonymous, Bow),
            (Human, TransitionUnloadBow, Bow),
            (Human, TransitionUnloadBowAnonymous, Bow),
            (Human, TransitionLoweringBow, Bow),
            (Human, TransitionLoweringBowAnonymous, Bow),
            (Human, TransitionRaisingBow, Bow),
            (Human, TransitionRaisingBowAnonymous, Bow),
            (Human, ShootingWithBow, Bow),
            (Human, ShootingWithBowAnonymous, Bow),
            (Human, ShootingWithBowUp, Bow),
            (Human, ShootingWithBowUpAnonymous, Bow),
            (Human, TransitionRaisingSword, GenericAnimation),
            (Human, TransitionLoweringSword, GenericAnimation),
            (Human, WaitingSword, WaitingSword),
            (Human, WalkingWithSword, Movement),
            (Human, RunningWithSword, Movement),
            (Human, TransitionWaitingSwordParryingSword, GenericAnimation),
            (Human, TransitionWaitingSwordParryingSwordLow, GenericAnimation),
            (Human, TransitionParryingSwordWaitingSword, GenericAnimation),
            (Human, ParryingLowSword, GenericAnimation),
            (Human, ParryingSword, GenericAnimation),
            (Human, DyingSword, GenericAnimation),
            (Human, DyingBow, GenericAnimation),
            (Human, BeingDeadSword, GenericAnimation),
            (Human, BeingDeadBow, GenericAnimation),
            (Human, BeingDead, GenericAnimation),
            (Human, FallingBackSword, GenericAnimation),
            (Human, FallingBackBow, GenericAnimation),
            (Human, BeingUnconsciousSword, GenericAnimation),
            (Human, BeingUnconsciousBow, GenericAnimation),
            (Human, BeingDeadFallenBackSword, GenericAnimation),
            (Human, BeingDeadFallenBackBow, GenericAnimation),
            (Human, BeingDeadFallenBack, GenericAnimation),
            // Smalltalk strikes have bespoke Human::Execute semantics. The
            // generic animation arm owns their back-facing/sword-state hit
            // test; the ordinary melee sweep intentionally does not.
            (Human, StrikingLeftSmalltalk, GenericAnimation),
            (Human, StrikingRightSmalltalk, GenericAnimation),
            (Human, StrikingLowRightSmalltalk, GenericAnimation),
            (Human, StrikingLowLeftSmalltalk, GenericAnimation),
            (Human, ParryingLeftSmalltalk, GenericAnimation),
            (Human, ParryingRightSmalltalk, GenericAnimation),
            (Human, ParryingLowRightSmalltalk, GenericAnimation),
            (Human, ParryingLowLeftSmalltalk, GenericAnimation),
            (Human, StrikingStraightSword, Melee),
            (Human, StrikingStraightStrongSword, Melee),
            (Human, ExecutingSword, Melee),
            (Human, StrikingLeftSword, Melee),
            (Human, StrikingRightSword, Melee),
            (Human, StrikingSemiroundRightSword, Melee),
            (Human, StrikingSemiroundLeftSword, Melee),
            (Human, StrikingRoundRightSword, Melee),
            (Human, StrikingRoundLeftSword, Melee),
            (Human, StrikingDownSword, Melee),
            (Human, DyingUpright, GenericAnimation),
            (Human, StandingUpSword, GenericAnimation),
            (Human, StandingUp, GenericAnimation),
            (Human, StandingUpBow, GenericAnimation),
            (Human, FallingLadderWall, GenericAnimation),
            (Human, FallingBackUpright, GenericAnimation),
            (Human, FallingBackCrouched, GenericAnimation),
            (Human, BeingUnconscious, GenericAnimation),
            (Human, BeingHitSword, GenericAnimation),
            (Human, BeingWeakSword, GenericAnimation),
            (Human, ExtractingArrowSword, GenericAnimation),
            (Human, ExtractingArrowUpright, GenericAnimation),
            (Human, ExtractingArrowCrouched, GenericAnimation),
            (Human, ExtractingArrowBow, GenericAnimation),
            (Human, DyingCrouched, GenericAnimation),
            (Human, BeingStunnedSword, GenericAnimation),
            (Human, WakingUp, GenericAnimation),
            (Human, Provoking, GenericAnimation),
            (Human, Hitting, Ability),
            (Human, FallingHitHarderUpright, GenericAnimation),
            (Human, FallingHitHarderWithBow, GenericAnimation),
            (Human, FallingHitHarderWithSword, GenericAnimation),
            (Human, FallingHitHarderCrouched, GenericAnimation),
            (Human, FallingHitUpright, GenericAnimation),
            (Human, FallingHitWithBow, GenericAnimation),
            (Human, FallingHitWithSword, GenericAnimation),
            (Human, FallingHitCrouched, GenericAnimation),
            (Human, FallingPushedUpright, GenericAnimation),
            (Human, FallingPushedWithBow, GenericAnimation),
            (Human, FallingPushedWithSword, GenericAnimation),
            (Human, FallingPushedCrouched, GenericAnimation),
            (Human, BeingCarriedLittleJohn, GenericAnimation),
            (Human, BeingCarriedPeasantC, GenericAnimation),
            (Human, RaisingShield, GenericAnimation),
            (Human, LoweringShield, GenericAnimation),
            (Human, ParryingShield, GenericAnimation),
            (Human, WaitingShield, GenericAnimation),
            (Human, Rolling, GenericAnimation),
            (Human, LyingStuckUnderNet, GenericAnimation),
            (Human, WriggleUnderNet, GenericAnimation),
            (Human, BeingTied, GenericAnimation),
            (Human, TakingNet, GenericAnimation),
            (Human, GettingWounded, GenericAnimation),
            (Human, PassingDoor, Movement),
            (Human, TransitionWaitingUprightSpecial, GenericAnimation),
            (Human, TransitionSpecialWaitingUpright, GenericAnimation),
            (Human, Special, GenericAnimation),
            (Pc, WalkingWithSword, Movement),
            (Pc, RunningWithSword, Movement),
            (Pc, Select, GenericAnimation),
            (Pc, WalkingCrouched, Movement),
            (Pc, WaitingCrouched, GenericAnimation),
            (Pc, WalkingCarryingOnShoulders, Movement),
            (Pc, ShootingWithBow, Bow),
            (Pc, ShootingWithBowUp, Bow),
            (Pc, JumpingUp, Movement),
            (Pc, JumpingDown, Movement),
            (Pc, JumpingLong, Movement),
            (Pc, JumpingLongSword, Movement),
            (Pc, TransitionWaitingOnShouldersJumpingUp, Movement),
            (Pc, TransitionWaitingOnShouldersJumpingLong, Movement),
            (Pc, TransitionWaitingUprightJumpingUp, Movement),
            (Pc, TransitionJumpingUpWaitingCrouched, Movement),
            (Pc, WaitingCape, GenericAnimation),
            (Pc, WaitingCapeAnonymousArcher, GenericAnimation),
            (Pc, TransitionWaitingCapeWaitingUpright, GenericAnimation),
            (Pc, WaitingHidden, GenericAnimation),
            (Pc, TransitionWaitingHiddenWaitingUpright, GenericAnimation),
            (Pc, TransitionWaitingCrouchedJumpingDown, Movement),
            (Pc, TransitionJumpingDownWaitingCrouched, Movement),
            (Pc, TransitionWaitingUprightJumpingLong, Movement),
            (Pc, TransitionWaitingSwordJumpingLongSword, Movement),
            (Pc, TransitionJumpingLongWaitingUpright, Movement),
            (Pc, TransitionJumpingLongSwordWaitingSword, Movement),
            (Pc, Taking, GenericAnimation),
            (Pc, TakingCrouched, GenericAnimation),
            (Pc, Eating, Ability),
            (Pc, Whistling, Ability),
            (Pc, Searching, GenericAnimation),
            (Pc, SearchingCrouched, GenericAnimation),
            (Pc, Healing, Ability),
            (Pc, TransitionWaitingUprightHelpingClimbing, GenericAnimation),
            (Pc, TransitionHelpingClimbingWaitingUpright, GenericAnimation),
            (Pc, WaitingHelpingClimbing, GenericAnimation),
            (Pc, WaitingCarryingOnShoulders, GenericAnimation),
            (Pc, WaitingOnShoulders, GenericAnimation),
            (Pc, ClimbingUpOnShoulders, Ability),
            (Pc, ClimbingDownFromShoulders, Ability),
            (Pc, TransitionHelpingClimbingDown, Movement),
            (Pc, TransitionWaitingUprightCarryingCorpse, Ability),
            (Pc, TransitionCarryingCorpseWaitingUpright, Ability),
            (Pc, WaitingWithCorpse, GenericAnimation),
            (Pc, WalkingWithCorpse, Movement),
            (Pc, FallingShoulders, GenericAnimation),
            (Pc, TransitionWaitingCarryingOnShouldersWaitingUpright, GenericAnimation),
            (Pc, DroppingAmmo, GenericAnimation),
            (Pc, DroppingAmmoCrouched, GenericAnimation),
            (Pc, ThrowingApple, Ability),
            (Pc, ThrowingStone, Ability),
            (Pc, ThrowingPurse, Ability),
            (Pc, ThrowingWaspNest, Ability),
            (Pc, ThrowingNet, Ability),
            (Pc, RaisingShield, GenericAnimation),
            (Pc, LoweringShield, GenericAnimation),
            (Pc, WalkingWithShield, Movement),
            (Pc, WaitingShield, GenericAnimation),
            (Pc, HidingBehindShield, GenericAnimation),
            (Pc, UsingLever, GenericAnimation),
            (Pc, DroppingAle, GenericAnimation),
            (Pc, DroppingAleCrouched, GenericAnimation),
            (Pc, UnlockingDoor, GenericAnimation),
            (Pc, UnlockingTrap, GenericAnimation),
            (Pc, HandlingTarget, GenericAnimation),
            (Pc, HittingTarget, GenericAnimation),
            (Pc, TakingTarget, GenericAnimation),
            (Pc, Paying, Ability),
            (Pc, Tying, Ability),
            (Pc, Strangling, Ability),
            (Pc, TransitionWaitingUprightSimulatingBeggar, Ability),
            (Pc, TransitionSimulatingBeggarWaitingUpright, Ability),
            (Pc, SimulatingBeggar, Beggar),
            (Pc, TransitionWaitingUprightListening, Ability),
            (Pc, TransitionListeningWaitingUpright, Ability),
            (Pc, Listening, Ability),
            (Pc, TransitionRaisingSword, GenericAnimation),
            (Pc, Provoking, GenericAnimation),
            (Pc, StrikingLeftSmalltalk, GenericAnimation),
            (Pc, StrikingRightSmalltalk, GenericAnimation),
            (Pc, StrikingLowRightSmalltalk, GenericAnimation),
            (Pc, StrikingLowLeftSmalltalk, GenericAnimation),
            (Pc, StrikingRoundLeftSword, Melee),
            (Pc, StrikingRoundRightSword, Melee),
            (Pc, ExecutingSword, Melee),
            (Pc, ExtractingArrowUpright, GenericAnimation),
            (Pc, ExtractingArrowBow, GenericAnimation),
            (Pc, ExtractingArrowSword, GenericAnimation),
            (Npc, Sitting, GenericAnimation),
            (Npc, TransitionSittingWaitingUpright, GenericAnimation),
            (Npc, TransitionWaitingUprightSitting, GenericAnimation),
            (Npc, BeggarShowingFace, GenericAnimation),
            (Npc, Pointing, GenericAnimation),
            (Npc, Searching, GenericAnimation),
            (Soldier, WaitingAlerted, GenericAnimation),
            (Soldier, WaitingUpright, GenericAnimation),
            (Soldier, TransitionWaitingUprightWaitingAlerted, GenericAnimation),
            (Soldier, LookingLeft, GenericAnimation),
            (Soldier, LookingLeftAlerted, GenericAnimation),
            (Soldier, LookingRight, GenericAnimation),
            (Soldier, LookingRightAlerted, GenericAnimation),
            (Soldier, TransitionWaitingAlertedWaitingUpright, GenericAnimation),
            (Soldier, TransitionWaitingAlertedWaitingUprightOfficer, GenericAnimation),
            (Soldier, TransitionWalkingUprightWaitingUpright, Movement),
            (Soldier, TransitionRunningUprightWaitingUpright, Movement),
            (Soldier, TransitionWaitingUprightWalkingUpright, Movement),
            (Soldier, TransitionWaitingUprightRunningUpright, Movement),
            (Soldier, TransitionWalkingUprightRunningUpright, Movement),
            (Soldier, TransitionRunningUprightWalkingUpright, Movement),
            (Soldier, WalkingUpright, Movement),
            (Soldier, WalkingStairs, Movement),
            (Soldier, RunningStairs, Movement),
            (Soldier, Turning, GenericAnimation),
            (Soldier, StandingUpSword, GenericAnimation),
            (Soldier, TransitionRaisingSword, GenericAnimation),
            (Soldier, TransitionCharging, Melee),
            (Soldier, GettingFreeFromWasp, GenericAnimation),
            (Soldier, Taking, GenericAnimation),
            (Soldier, TransitionWaitingSwordMenacing, GenericAnimation),
            (Soldier, Menacing, GenericAnimation),
            (Soldier, SleepingUpright, GenericAnimation),
            (Soldier, TransitionSleepingWaitingUpright, GenericAnimation),
            (Soldier, GatheringSoldiers, GenericAnimation),
            (Soldier, TransitionMenacingWaitingSword, GenericAnimation),
            (Soldier, LeaningOut, GenericAnimation),
            (Soldier, TransitionWaitingAlertedLeaningOut, GenericAnimation),
            (Soldier, TransitionLeaningOutWaitingAlerted, GenericAnimation),
            (Soldier, DrinkingAle, GenericAnimation),
            (Soldier, TransitionLoweringBowLeaningOut, Bow),
            (Soldier, TransitionRaisingBowLeaningOut, Bow),
            (Soldier, AimingWithBowLeaningOut, GenericAnimation),
            (Soldier, ShootingWithBowLeaningOut, Bow),
            (Soldier, RunningUpright, Movement),
            (Soldier, RiderCharging, Movement),
            (Soldier, Special, GenericAnimation),
            (Civilian, WaitingUpright, GenericAnimation),
            (Civilian, WaitingUprightBored, GenericAnimation),
            (Civilian, WaitingUprightBoredRandom, GenericAnimation),
            (Civilian, TransitionWaitingUprightBoredWaitingUpright, GenericAnimation),
            (Civilian, TransitionWaitingUprightWaitingUprightBored, GenericAnimation),
            (Civilian, ReceivingPurse, Ability),
            (Civilian, WaitingWithPurse, Ability),
            (Civilian, TransitionWaitingWithPurseWaitingUpright, Ability),
        }
    };
}

macro_rules! define_actor_execute_catalog {
    ($(($override:ident, $order:ident, $owner:ident),)*) => {
        #[cfg(test)]
        pub(super) const ORIGINAL_ACTOR_EXECUTE_CATALOG: &[(ExecuteOverride, crate::order::OrderType, ExecuteOwnerFamily)] = &[
            $((ExecuteOverride::$override, crate::order::OrderType::$order, ExecuteOwnerFamily::$owner),)*
        ];

        pub(super) fn classify_actor_execute_arm(
            override_kind: ExecuteOverride,
            order: crate::order::OrderType,
        ) -> Option<ExecuteOwnerFamily> {
            match (override_kind, order) {
                $((ExecuteOverride::$override, crate::order::OrderType::$order) => Some(ExecuteOwnerFamily::$owner),)*
                _ => None,
            }
        }
    };
}
actor_execute_arm_catalog!(define_actor_execute_catalog);

pub(super) fn classify_live_actor_execute_arm(
    entity_id: EntityId,
    order: crate::order::OrderType,
) -> Option<ExecuteOwnerFamily> {
    let chain: &[ExecuteOverride] = match entity_id {
        EntityId::Pc(_) => &[
            ExecuteOverride::Pc,
            ExecuteOverride::Human,
            ExecuteOverride::Actor,
        ],
        EntityId::Soldier(_) => &[
            ExecuteOverride::Soldier,
            ExecuteOverride::Npc,
            ExecuteOverride::Human,
            ExecuteOverride::Actor,
        ],
        EntityId::Civilian(_) => &[
            ExecuteOverride::Civilian,
            ExecuteOverride::Npc,
            ExecuteOverride::Human,
            ExecuteOverride::Actor,
        ],
        _ => return None,
    };
    chain
        .iter()
        .find_map(|override_kind| classify_actor_execute_arm(*override_kind, order))
}

/// Motion state returned by a specialized derived Execute arm.
///
/// Most specialized owners forward the sprite result. The PC beggar idle is
/// an explicit exception: `RHElementActorPC::Execute` performs the sprite
/// action and side effects, then always returns `RHMOTION_IN_PROGRESS`.
fn specialized_execute_motion(
    sprite_motion: Option<crate::sprite::MotionState>,
    selected_beggar: bool,
    movement_entity_target_seek: bool,
) -> Option<crate::sprite::MotionState> {
    if selected_beggar {
        Some(crate::sprite::MotionState::InProgress)
    } else if movement_entity_target_seek
        && sprite_motion
            .is_some_and(|motion| !matches!(motion, crate::sprite::MotionState::Terminated))
    {
        // Actor::PerformSeek consumes non-terminal sprite results while an
        // entity target remains live. The surrounding movement Execute arm
        // observes IN_PROGRESS even though Sprite::PerformMotion recorded a
        // raw START or DONE edge.
        Some(crate::sprite::MotionState::InProgress)
    } else {
        sprite_motion
    }
}

pub(super) trait IntoExplicitExecuteMotion {
    fn into_explicit_execute_motion(self) -> ExplicitExecuteMotion;
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ExplicitExecuteMotion {
    pub initial: Option<crate::sprite::MotionState>,
    pub post_completion_override: Option<crate::sprite::MotionState>,
}

impl IntoExplicitExecuteMotion for () {
    fn into_explicit_execute_motion(self) -> ExplicitExecuteMotion {
        ExplicitExecuteMotion::default()
    }
}

impl IntoExplicitExecuteMotion for Option<crate::sprite::MotionState> {
    fn into_explicit_execute_motion(self) -> ExplicitExecuteMotion {
        ExplicitExecuteMotion {
            initial: self,
            post_completion_override: None,
        }
    }
}

impl IntoExplicitExecuteMotion for ExplicitExecuteMotion {
    fn into_explicit_execute_motion(self) -> ExplicitExecuteMotion {
        self
    }
}

fn apply_post_completion_execute_override(
    projected: crate::sprite::MotionState,
    post_completion_override: Option<crate::sprite::MotionState>,
    selected_element_interrupted: bool,
    installed_successor_exists: bool,
) -> crate::sprite::MotionState {
    if !selected_element_interrupted || installed_successor_exists {
        projected
    } else {
        post_completion_override.unwrap_or(projected)
    }
}

fn project_post_completion_motion(
    current: crate::sprite::MotionState,
    selected_element_impossible: bool,
    installed_successor_exists: bool,
    selected_specialized_order_advanced: bool,
) -> crate::sprite::MotionState {
    use crate::sprite::MotionState;
    if selected_element_impossible {
        MotionState::Aborted
    } else if installed_successor_exists
        && (current == MotionState::Terminated || selected_specialized_order_advanced)
    {
        MotionState::InProgress
    } else if selected_specialized_order_advanced {
        MotionState::Terminated
    } else {
        current
    }
}

#[derive(Debug)]
struct MotionLatchDebugConfig {
    frame: u32,
    creation_order: u32,
}

fn motion_latch_debug_config() -> Option<&'static MotionLatchDebugConfig> {
    static CONFIG: std::sync::OnceLock<Option<MotionLatchDebugConfig>> = std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            std::env::var_os("PARITY_DEBUG_MOTION_LATCH")?;
            let parse = |name: &str| {
                let raw = std::env::var(name).unwrap_or_else(|_| {
                    panic!("{name} is required when motion-latch debugging is enabled")
                });
                raw.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid {name}={raw:?} for motion-latch diagnostic: {error}")
                })
            };
            Some(MotionLatchDebugConfig {
                frame: parse("PARITY_DEBUG_MOTION_LATCH_FRAME"),
                creation_order: parse("PARITY_DEBUG_MOTION_LATCH_CREATION_ORDER"),
            })
        })
        .as_ref()
}

fn specialized_order_advanced_after_execute(
    execute_motion: Option<crate::sprite::MotionState>,
    selected_order_rewritten_by_stop: bool,
    selected_element_retired: bool,
    selected_element_interrupted: bool,
    selected_entry_order_still_current: bool,
) -> bool {
    execute_motion.is_some_and(|motion| motion != crate::sprite::MotionState::Aborted)
        && !selected_order_rewritten_by_stop
        // Original Actor::Hourglass latches Execute's return before
        // CheckForLineCrossing. A synchronous line callback may interrupt and
        // replace the selected sequence, but the later motion-state switch
        // still reads that already-held nonterminal result; interruption is
        // not Execute-owned DoNextOrder advancement.
        && !selected_element_interrupted
        && (selected_element_retired || !selected_entry_order_still_current)
}

/// `RHSequenceElementMovement::StopMovement` rewrites its first `RHOrder` in
/// place and calls `NewID()`.  That identity change is not `DoNextOrder`: the
/// motion result already returned by `Execute` remains authoritative.
fn is_start_stop_movement_rewrite(
    entry_order_id: std::num::NonZeroU32,
    entry_order: crate::order::OrderType,
    live_order_id: std::num::NonZeroU32,
    live_order: crate::order::OrderType,
    execute_motion: crate::sprite::MotionState,
) -> bool {
    use crate::order::OrderType;

    matches!(
        execute_motion,
        crate::sprite::MotionState::Start
            | crate::sprite::MotionState::InProgress
            | crate::sprite::MotionState::Done
    )
        // StopMovement calls NewID on the existing order. Runtime order IDs
        // are monotonic, whereas a translated stop-transition successor was
        // allocated before path waypoints that may later be inserted ahead of
        // it. This separates an in-place reseed from DoNextOrder exposing an
        // already queued transition after a fresh waypoint reaches its goal.
        && live_order_id > entry_order_id
        && matches!(
            (entry_order, live_order),
            (
                OrderType::WalkingUpright,
                OrderType::TransitionWalkingUprightWaitingUpright
            ) | (
                OrderType::RunningUpright,
                OrderType::TransitionRunningUprightWaitingUpright
            ) | (
                OrderType::WalkingCrouched,
                OrderType::TransitionWalkingCrouchedWaitingCrouched
            )
        )
}

#[cfg(test)]
mod specialized_execute_motion_tests {
    use super::{
        apply_post_completion_execute_override, is_start_stop_movement_rewrite,
        project_post_completion_motion, specialized_execute_motion,
        specialized_order_advanced_after_execute,
    };
    use crate::order::OrderType;
    use crate::sprite::MotionState;

    #[test]
    fn beggar_idle_returns_in_progress_while_retaining_the_sprite_start() {
        assert_eq!(
            specialized_execute_motion(Some(MotionState::Start), true, false),
            Some(MotionState::InProgress)
        );
        assert_eq!(
            specialized_execute_motion(Some(MotionState::Done), false, false),
            Some(MotionState::Done)
        );
        assert_eq!(
            specialized_execute_motion(Some(MotionState::Start), false, true),
            Some(MotionState::InProgress)
        );
        assert_eq!(
            specialized_execute_motion(Some(MotionState::Terminated), false, true),
            Some(MotionState::Terminated)
        );
    }

    #[test]
    fn impossible_entry_element_preserves_aborted_across_condolence_sprite_edges() {
        assert_eq!(
            project_post_completion_motion(MotionState::Terminated, true, false, true),
            MotionState::Aborted
        );
        assert_eq!(
            project_post_completion_motion(MotionState::Done, true, true, true),
            MotionState::Aborted
        );
    }

    #[test]
    fn manager_resident_wait_without_an_installed_order_does_not_mask_completion() {
        assert_eq!(
            project_post_completion_motion(MotionState::Done, false, false, true),
            MotionState::Terminated
        );
    }

    #[test]
    fn exhausted_jump_landing_retains_terminated_without_a_successor() {
        assert_eq!(
            project_post_completion_motion(MotionState::Terminated, false, false, true),
            MotionState::Terminated
        );
    }

    #[test]
    fn jump_landing_with_an_installed_successor_resumes_in_progress() {
        assert_eq!(
            project_post_completion_motion(MotionState::Terminated, false, true, true),
            MotionState::InProgress
        );
    }

    #[test]
    fn synchronous_line_crossing_interruption_preserves_nonterminal_execute_result() {
        assert!(!specialized_order_advanced_after_execute(
            Some(MotionState::InProgress),
            false,
            true,
            true,
            false,
        ));
        assert_eq!(
            project_post_completion_motion(MotionState::InProgress, false, false, false),
            MotionState::InProgress,
            "the line callback cannot turn the preceding Execute result into Terminated"
        );

        assert!(specialized_order_advanced_after_execute(
            Some(MotionState::Terminated),
            false,
            true,
            false,
            false,
        ));
        assert!(specialized_order_advanced_after_execute(
            Some(MotionState::Done),
            false,
            false,
            false,
            false,
        ));
    }

    #[test]
    fn committed_arrival_termination_survives_line_callback_interruption() {
        assert_eq!(
            apply_post_completion_execute_override(
                MotionState::InProgress,
                Some(MotionState::Terminated),
                true,
                false,
            ),
            MotionState::Terminated,
            "Original latches PerformMotion's terminal arrival before the line callback interrupts the movement"
        );
        assert_eq!(
            apply_post_completion_execute_override(
                MotionState::InProgress,
                Some(MotionState::Terminated),
                true,
                true,
            ),
            MotionState::InProgress,
            "DoNextOrder's installed successor must still project the actor motion back to InProgress"
        );
        assert_eq!(
            apply_post_completion_execute_override(
                MotionState::InProgress,
                Some(MotionState::Terminated),
                false,
                false,
            ),
            MotionState::InProgress,
            "an ordinary committed arrival must retain the normal post-completion projection"
        );
    }

    #[test]
    fn stop_movement_new_id_is_not_a_successor_order_advance() {
        assert!(is_start_stop_movement_rewrite(
            std::num::NonZeroU32::new(10).unwrap(),
            OrderType::WalkingUpright,
            std::num::NonZeroU32::new(11).unwrap(),
            OrderType::TransitionWalkingUprightWaitingUpright,
            MotionState::Start,
        ));
        assert!(is_start_stop_movement_rewrite(
            std::num::NonZeroU32::new(10).unwrap(),
            OrderType::RunningUpright,
            std::num::NonZeroU32::new(11).unwrap(),
            OrderType::TransitionRunningUprightWaitingUpright,
            MotionState::Done,
        ));
        assert!(is_start_stop_movement_rewrite(
            std::num::NonZeroU32::new(10).unwrap(),
            OrderType::WalkingCrouched,
            std::num::NonZeroU32::new(11).unwrap(),
            OrderType::TransitionWalkingCrouchedWaitingCrouched,
            MotionState::InProgress,
        ));
        assert!(!is_start_stop_movement_rewrite(
            std::num::NonZeroU32::new(10).unwrap(),
            OrderType::WalkingUpright,
            std::num::NonZeroU32::new(11).unwrap(),
            OrderType::TransitionWalkingUprightWaitingUpright,
            MotionState::Terminated,
        ));
        assert!(!is_start_stop_movement_rewrite(
            std::num::NonZeroU32::new(10).unwrap(),
            OrderType::WalkingUpright,
            std::num::NonZeroU32::new(11).unwrap(),
            OrderType::WalkingUpright,
            MotionState::Start,
        ));
        assert!(!is_start_stop_movement_rewrite(
            std::num::NonZeroU32::new(11).unwrap(),
            OrderType::RunningUpright,
            std::num::NonZeroU32::new(10).unwrap(),
            OrderType::TransitionRunningUprightWaitingUpright,
            MotionState::Start,
        ));
    }

    #[test]
    fn stop_movement_reseed_preserves_outgoing_done_latch() {
        let rewritten_by_stop = is_start_stop_movement_rewrite(
            std::num::NonZeroU32::new(10).unwrap(),
            OrderType::RunningUpright,
            std::num::NonZeroU32::new(11).unwrap(),
            OrderType::TransitionRunningUprightWaitingUpright,
            MotionState::Done,
        );
        assert!(rewritten_by_stop);

        let advanced = specialized_order_advanced_after_execute(
            Some(MotionState::Done),
            rewritten_by_stop,
            false,
            false,
            false,
        );
        assert!(!advanced);
        assert_eq!(
            project_post_completion_motion(MotionState::Done, false, true, advanced),
            MotionState::Done
        );
    }
}

#[cfg(test)]
pub(super) fn assert_execute_owner_handler_is_linked(family: ExecuteOwnerFamily) {
    match family {
        ExecuteOwnerFamily::GenericAnimation => {
            let _ = EngineInner::tick_actor_animation_for;
        }
        ExecuteOwnerFamily::Movement => {
            let _ = EngineInner::tick_entity_movement_owner;
        }
        ExecuteOwnerFamily::Melee => {
            let _ = EngineInner::tick_selected_melee_owner;
        }
        ExecuteOwnerFamily::Bow => {
            let _ = EngineInner::tick_bow_shot_for;
        }
        ExecuteOwnerFamily::Ability => {
            let _ = EngineInner::tick_ability_for;
        }
        ExecuteOwnerFamily::Beggar => {
            let _ = EngineInner::tick_beggar_bid_for;
        }
        ExecuteOwnerFamily::WaitingSword => {
            let _ = EngineInner::tick_waiting_sword_execute_for;
        }
    }
}
// ─── Per-tick timing instrumentation ─────────────────────────────────
//
// Records the wall-clock duration of every `perform_hourglass` call
// and emits a periodic summary so we can see where the rollback
// checker's 25-replays-per-frame cost actually goes. Lives in a
// thread-local so the live tick and the rollback-replay ticks each get
// their own histogram (rollback runs on the same thread but typically
// happens in bursts of 25, so they'll dominate any window they hit).
thread_local! {
    static HOURGLASS_STATS: std::cell::RefCell<HourglassStats> =
        std::cell::RefCell::new(HourglassStats::default());
    static HOURGLASS_PHASE_STATS: std::cell::RefCell<HourglassPhaseStats> =
        std::cell::RefCell::new(HourglassPhaseStats::default());
}

/// Number of `perform_hourglass` calls between log lines.
const HOURGLASS_LOG_INTERVAL: u32 = 100;

/// Coarse, ordered phases of [`EngineInner::perform_hourglass_inner`].
///
/// Keep these deliberately broader than individual systems: the phase trace is
/// an ordering contract for the tick spine, not a second scheduler.  In
/// particular, `Paths` names the fixed completion/start barrier and failed-path
/// deadlines; movement dispatch only queues the requests resolved there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HourglassPhase {
    DeferredEffectsStart,
    MissionAndMessages,
    NpcOrders,
    Paths,
    Entities,
    EntitySystems,
    Npcs,
    GameplaySystems,
    Sequences,
    DeferredEffectsEnd,
}

#[derive(Default)]
struct HourglassPhaseStats {
    count: u32,
    total_us: [u128; 10],
}

/// Opt-in detail inside the otherwise broad `EntitySystems` phase.
#[derive(Clone, Copy)]
pub(super) enum EntitySystemDetail {
    BoundarySnapshot = 0,
    PrepareNpc = 1,
    StaticOwners = 2,
    OwnerPrelude = 3,
    OwnerExecute = 4,
    NpcTail = 5,
    FinishNpc = 6,
    CorpseUpdates = 7,
    FrameSounds = 8,
    BuildEntityViews = 9,
    BuildWorldView = 10,
    RefreshDetection = 11,
}

const ENTITY_SYSTEM_DETAIL_COUNT: usize = 12;

#[derive(Default)]
struct EntitySystemDetailStats {
    frames: u32,
    calls: [u64; ENTITY_SYSTEM_DETAIL_COUNT],
    total_us: [u128; ENTITY_SYSTEM_DETAIL_COUNT],
}

thread_local! {
    static ENTITY_SYSTEM_DETAIL_STATS: std::cell::RefCell<EntitySystemDetailStats> =
        std::cell::RefCell::new(EntitySystemDetailStats::default());
}

pub(super) struct EntitySystemDetailGuard {
    phase: EntitySystemDetail,
    start: Option<web_time::Instant>,
}

impl Drop for EntitySystemDetailGuard {
    fn drop(&mut self) {
        let Some(start) = self.start else { return };
        let elapsed_us = start.elapsed().as_micros();
        ENTITY_SYSTEM_DETAIL_STATS.with(|cell| {
            let mut stats = cell.borrow_mut();
            let index = self.phase as usize;
            stats.calls[index] += 1;
            stats.total_us[index] += elapsed_us;
        });
    }
}

pub(super) fn entity_system_detail_guard(phase: EntitySystemDetail) -> EntitySystemDetailGuard {
    EntitySystemDetailGuard {
        phase,
        start: tracing::enabled!(
            target: "robin_engine::engine::tick::entity_system_perf",
            tracing::Level::INFO
        )
        .then(web_time::Instant::now),
    }
}

fn finish_entity_system_detail_frame() {
    if !tracing::enabled!(
        target: "robin_engine::engine::tick::entity_system_perf",
        tracing::Level::INFO
    ) {
        return;
    }
    ENTITY_SYSTEM_DETAIL_STATS.with(|cell| {
        let mut stats = cell.borrow_mut();
        stats.frames += 1;
        if stats.frames < HOURGLASS_LOG_INTERVAL {
            return;
        }
        let frames = u128::from(stats.frames);
        let per_frame = |phase: EntitySystemDetail| stats.total_us[phase as usize] / frames;
        let calls = |phase: EntitySystemDetail| stats.calls[phase as usize];
        tracing::info!(
            target: "robin_engine::engine::tick::entity_system_perf",
            frames = stats.frames,
            boundary_us = per_frame(EntitySystemDetail::BoundarySnapshot),
            prepare_npc_us = per_frame(EntitySystemDetail::PrepareNpc),
            static_owners_us = per_frame(EntitySystemDetail::StaticOwners),
            owner_prelude_us = per_frame(EntitySystemDetail::OwnerPrelude),
            owner_execute_us = per_frame(EntitySystemDetail::OwnerExecute),
            npc_tail_us = per_frame(EntitySystemDetail::NpcTail),
            finish_npc_us = per_frame(EntitySystemDetail::FinishNpc),
            corpse_us = per_frame(EntitySystemDetail::CorpseUpdates),
            frame_sounds_us = per_frame(EntitySystemDetail::FrameSounds),
            build_views_us = per_frame(EntitySystemDetail::BuildEntityViews),
            build_views_calls = calls(EntitySystemDetail::BuildEntityViews),
            world_view_us = per_frame(EntitySystemDetail::BuildWorldView),
            world_view_calls = calls(EntitySystemDetail::BuildWorldView),
            detection_us = per_frame(EntitySystemDetail::RefreshDetection),
            detection_calls = calls(EntitySystemDetail::RefreshDetection),
            "entity systems detail timing"
        );
        *stats = EntitySystemDetailStats::default();
    });
}

fn time_hourglass_phase<T>(phase: HourglassPhase, f: impl FnOnce() -> T) -> T {
    trace_hourglass_phase(phase);
    let timer = tracing::enabled!(
        target: "robin_engine::engine::tick::phase_perf",
        tracing::Level::INFO
    )
    .then(web_time::Instant::now);
    let result = f();
    if let Some(timer) = timer {
        HOURGLASS_PHASE_STATS.with(|cell| {
            let mut stats = cell.borrow_mut();
            stats.total_us[phase as usize] += timer.elapsed().as_micros();
            if phase == HourglassPhase::DeferredEffectsEnd {
                stats.count += 1;
                if stats.count >= HOURGLASS_LOG_INTERVAL {
                    tracing::info!(
                        target: "robin_engine::engine::tick::phase_perf",
                        count = stats.count,
                        deferred_start_us = stats.total_us[0] / stats.count as u128,
                        mission_us = stats.total_us[1] / stats.count as u128,
                        npc_orders_us = stats.total_us[2] / stats.count as u128,
                        paths_us = stats.total_us[3] / stats.count as u128,
                        entities_us = stats.total_us[4] / stats.count as u128,
                        entity_systems_us = stats.total_us[5] / stats.count as u128,
                        npcs_us = stats.total_us[6] / stats.count as u128,
                        gameplay_us = stats.total_us[7] / stats.count as u128,
                        sequences_us = stats.total_us[8] / stats.count as u128,
                        deferred_end_us = stats.total_us[9] / stats.count as u128,
                        "perform_hourglass phase timing"
                    );
                    *stats = HourglassPhaseStats::default();
                }
            }
        });
    }
    result
}

#[cfg(test)]
thread_local! {
    static CAPTURED_HOURGLASS_PHASES: std::cell::RefCell<Option<Vec<HourglassPhase>>> =
        const { std::cell::RefCell::new(None) };
}

fn trace_hourglass_phase(phase: HourglassPhase) {
    tracing::trace!(
        target: "robin_engine::engine::tick::phases",
        ?phase,
        "perform_hourglass phase"
    );
    #[cfg(test)]
    CAPTURED_HOURGLASS_PHASES.with(|captured| {
        if let Some(phases) = captured.borrow_mut().as_mut() {
            phases.push(phase);
        }
    });
}

#[cfg(test)]
pub(super) fn begin_hourglass_phase_capture() {
    CAPTURED_HOURGLASS_PHASES.with(|captured| {
        let previous = captured.borrow_mut().replace(Vec::new());
        assert!(previous.is_none(), "hourglass phase capture already active");
    });
}

#[cfg(test)]
pub(super) fn end_hourglass_phase_capture() -> Vec<HourglassPhase> {
    CAPTURED_HOURGLASS_PHASES.with(|captured| {
        captured
            .borrow_mut()
            .take()
            .expect("hourglass phase capture was not active")
    })
}

#[cfg(test)]
thread_local! {
    static CAPTURED_ORDERED_GAMEPLAY_ENTITIES: std::cell::RefCell<Option<Vec<EntityId>>> =
        const { std::cell::RefCell::new(None) };
}

fn observe_ordered_gameplay_entity(entity_id: EntityId) {
    tracing::trace!(
        target: "robin_engine::engine::tick::ordered_gameplay",
        ?entity_id,
        "ordered gameplay slot"
    );
    #[cfg(test)]
    CAPTURED_ORDERED_GAMEPLAY_ENTITIES.with(|captured| {
        if let Some(entities) = captured.borrow_mut().as_mut() {
            entities.push(entity_id);
        }
    });
}

#[cfg(test)]
pub(super) fn capture_ordered_gameplay_entities<T>(f: impl FnOnce() -> T) -> (T, Vec<EntityId>) {
    CAPTURED_ORDERED_GAMEPLAY_ENTITIES.with(|captured| {
        assert!(
            captured.borrow_mut().replace(Vec::new()).is_none(),
            "ordered gameplay capture is not re-entrant"
        );
    });
    let result = f();
    let entities = CAPTURED_ORDERED_GAMEPLAY_ENTITIES.with(|captured| {
        captured
            .borrow_mut()
            .take()
            .expect("ordered gameplay capture must remain active")
    });
    (result, entities)
}

/// Move exclamations whose decoded-duration deadline has arrived into
/// the callback queue consumed as the first mutation of the next
/// `PerformHourglass` deferred-effects phase.
pub(super) fn drain_matured_exclamations(
    sound_sim: &mut crate::sound::SoundSimState,
    cur_frame: u32,
) {
    let mut still_playing = Vec::new();
    let mut finished = Vec::new();
    for p in sound_sim.playing_exclamations.drain(..) {
        if p.finish_frame <= cur_frame {
            finished.push((p.actor_id, p.exclamation_id));
        } else {
            still_playing.push(p);
        }
    }
    sound_sim.playing_exclamations = still_playing;
    sound_sim.finished_exclamations = finished;
}

#[derive(Default)]
struct HourglassStats {
    count: u32,
    total_us: u128,
    min_us: u128,
    max_us: u128,
}

impl HourglassStats {
    fn record(&mut self, us: u128) {
        if self.count == 0 {
            self.min_us = us;
            self.max_us = us;
        } else {
            self.min_us = self.min_us.min(us);
            self.max_us = self.max_us.max(us);
        }
        self.count += 1;
        self.total_us += us;
    }

    fn flush(&mut self) {
        if self.count == 0 {
            return;
        }
        let avg = self.total_us / self.count as u128;
        tracing::info!(
            target: "robin_engine::engine::tick::perf",
            count = self.count,
            avg_us = avg,
            min_us = self.min_us,
            max_us = self.max_us,
            "perform_hourglass timing"
        );
        *self = Self::default();
    }
}

/// RAII guard: timer.start() at construction, records on drop. Logs a
/// summary every `HOURGLASS_LOG_INTERVAL` ticks.
struct HourglassTimer {
    start: web_time::Instant,
}

impl HourglassTimer {
    fn start() -> Option<Self> {
        if !tracing::enabled!(target: "robin_engine::engine::tick::perf", tracing::Level::INFO) {
            return None;
        }
        Some(Self {
            start: web_time::Instant::now(),
        })
    }
}

impl Drop for HourglassTimer {
    fn drop(&mut self) {
        let us = self.start.elapsed().as_micros();
        HOURGLASS_STATS.with(|cell| {
            let mut s = cell.borrow_mut();
            s.record(us);
            if s.count >= HOURGLASS_LOG_INTERVAL {
                s.flush();
            }
        });
    }
}

impl EngineInner {
    pub(crate) fn perform_frame_hourglass(
        &mut self,
        assets: &LevelAssets,
        simulation_body_allowed: bool,
    ) -> super::SideEffects {
        let mut display = std::mem::take(&mut self.feedback.cutscene_camera.display);
        let effects =
            self.perform_hourglass_authoritative(&mut display, assets, simulation_body_allowed);
        self.feedback.cutscene_camera.display = display;
        effects
    }

    pub(crate) fn perform_frame_post_initialize(
        &mut self,
        assets: &LevelAssets,
    ) -> Option<super::SideEffects> {
        let mut display = std::mem::take(&mut self.feedback.cutscene_camera.display);
        let effects = self.perform_post_initialize_authoritative(&mut display, assets);
        self.feedback.cutscene_camera.display = display;
        effects
    }

    /// Expose the exact actor/sprite/sequence identities around the PC Drop
    /// Execute boundary without changing any authoritative state.
    fn debug_drop_owner_boundary(
        &self,
        phase: &'static str,
        owner: EntityId,
        selected_order: Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)>,
    ) {
        let frame = self.control.frame_counter;
        if !drop_owner_boundary_matches(frame, owner) {
            return;
        }
        let entity = self
            .world
            .entities
            .get(owner)
            .unwrap_or_else(|| panic!("Drop boundary owner {owner:?} disappeared"));
        let actor = entity
            .actor_data()
            .unwrap_or_else(|| panic!("Drop boundary owner {owner:?} is not an actor"));
        let ability = &actor.active_ability;
        let selected_state = selected_order.and_then(|(seq, elem, _)| {
            self.orders
                .sequence_manager
                .get_element(seq, elem)
                .map(|element| element.state)
        });
        eprintln!(
            "DROPBOUND frame={frame} phase={phase} owner={owner:?} execute_initialising={} active_kind={:?} active_seq={:?} active_elem={} active_order={:?} selected={selected_order:?} selected_state={selected_state:?} installed={:?} actor_last_execute={:?} sprite_last_processed={} sprite_action={:?}",
            actor.execute_order_initialising,
            ability.kind,
            ability.sequence_id,
            ability.element_index,
            ability.order_id,
            actor.installed_order,
            actor.last_execute_order_id,
            entity.element_data().sprite.last_processed_order_id,
            entity.element_data().sprite.last_action,
        );
    }
    // ─── Main update tick ────────────────────────────────────────

    /// Test-only adapter for the main per-frame logic update.
    ///
    /// Returns the game state code — normally `LevelInProgress`, but can
    /// return `LevelSucceeded`, `LevelFailed`, or `LevelInterrupted` to
    /// signal that the mission is over.
    ///
    /// Production callers must use [`super::rollback_safe::Engine::advance_frame`].
    /// Low-level engine tests use this adapter to preserve the legacy
    /// command/hourglass boundary while applying emitted host events to an
    /// explicit caller-owned input state.
    ///
    /// Called once per frame from the game loop, gated by:
    /// - console not displayed
    /// - no UI transition in progress
    /// - not paused
    /// - not in LEVEL_NEXT or LEVEL_LOAD state
    ///
    /// Supplies [`EngineInner::perform_hourglass_inner`] with an explicit
    /// simulation context and drains the deferred sound queue so all
    /// gameplay-affecting randomness is pulled from the engine-owned stream
    /// (deterministic across clients) and all audio is
    /// flushed *after* the sim is done (letting rollback replay the tick
    /// without duplicating playback).
    #[cfg(test)]
    pub(crate) fn perform_hourglass(
        &mut self,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        dev: &mut DevState,
    ) -> super::SideEffects {
        let mut camera = self.feedback.cutscene_camera.display.clone();
        let effects = self.perform_hourglass_authoritative(&mut camera, assets, true);
        self.feedback.cutscene_camera.display = camera;
        for event in effects.host_events.iter().cloned() {
            display.apply_host_event(input, event);
        }
        if dev.projectile_cheat_rain >= 0 {
            dev.projectile_cheat_rain = -1;
        }
        effects
    }

    /// Run an hourglass while optionally forcing the simulation-body gate
    /// closed for this tick.
    ///
    /// A closed gate still runs the mission script/message phase and advances
    /// the mission clock, exactly like the engine's persistent lock, but does
    /// not mutate that persistent lock state.
    fn perform_hourglass_authoritative(
        &mut self,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        simulation_body_allowed: bool,
    ) -> super::SideEffects {
        let _hourglass_timer = HourglassTimer::start();

        let sim = self.control.simulation_context();
        let sim = &sim;

        // RHGame records parity immediately after PerformHourglass, then its
        // render pass calls RHElementArrow::Refresh. Reproduce that
        // between-frame mutation here, before any draw from the next engine
        // frame. A restored mission starts with no pending pass because its
        // serialized sprites already crossed the preceding Refresh boundary.
        self.apply_pending_arrow_refresh(sim);

        // RHScript::FadeToBlack presents its ramp in a tight loop without
        // calling PerformHourglass. Drain the corresponding presentation
        // count before lending the explicit simulation context or touching any simulation,
        // display-state, or sound timer. A frame-counter deadline cannot
        // represent this: advancing that clock would mature every deadline
        // that is supposed to remain frozen during the blocking native.
        if self.consume_fade_freeze_frame() {
            let mut fx = self.feedback.drain_side_effects();
            fx.code = GameCode::LevelInProgress;
            // Fast-forward render skipping must not strand the host fade.
            fx.skip_render = false;
            return fx;
        }

        // Director work runs after the preceding PerformHourglass and can
        // complete a CameraGoto/ZoomLevel sequence element there.  Original
        // `SetState(Terminated) -> Ready() -> Go()` executes immediate
        // successors before the next actor Hourglass.  Close that between-
        // frame callback stack now: this preserves the post-Hourglass state
        // boundary while ensuring LockUser/SendMessage/Timer successors run
        // before any actor receives the next movement tick.
        self.drain_pending_immediate_actions_sync(sim, display, assets);

        let code = self.perform_hourglass_inner(sim, display, assets, simulation_body_allowed);
        self.advance_auto_quick_action_queues(sim, display, assets);
        self.control.arrow_refresh_pending = true;

        // Post-tick sim mutations that used to live in `game_session`
        // between the hourglass and the render pass. They have to run
        // inside `perform_hourglass` for rollback determinism: replay
        // only re-runs `perform_hourglass`, so anything advancing engine
        // state outside it would diverge from the live timeline.
        self.update_overall_villain_alert(&assets.profile_manager);
        // Forbidden-expression timers age in the Original's per-frame PC
        // render refresh, which runs after the whole simulation frame.  Keep
        // the decrement here (not inside a mid-hourglass melee phase) so a
        // bark queued by any hourglass phase still ages this frame; otherwise
        // the 75-frame forbid window ends one frame late and a repeat bark
        // the Original accepted at exactly +75 frames is wrongly rejected.
        self.tick_refresh_hero_mouth();
        self.feedback
            .pending_side_effects
            .host_events
            .push(HostEvent::Minimap(MinimapHostEvent::Tick));
        self.feedback
            .pending_side_effects
            .host_events
            .push(HostEvent::MacroUi(MacroUiHostEvent::Tick {
                slots: self.macro_slot_lengths(),
                pc_ids: self.world.pc_ids.clone(),
            }));
        // Advance destination-marker animation and retire finished
        // marks.  Used to run during rendering, which broke rollback
        // determinism — the render path is now read-only.
        {
            let view_pos = self.feedback.cutscene_camera.view_position;
            let zoom = self.feedback.cutscene_camera.zoom_factor;
            let screen = Self::director_camera_view_size();
            let screen_w = screen.x as i32;
            let screen_h = screen.y as i32;
            let frame_counter = self.control.frame_counter;
            self.feedback.ground_mark.tick(
                view_pos.to_geo(),
                zoom,
                screen_w,
                screen_h,
                frame_counter,
            );
        }
        // Sound-source delay state machine. Original queues playback at zero
        // and re-rolls only when that playback finishes (`RHSound::StopSoundSource`),
        // so keep a deterministic sim-side finish deadline rather than
        // consuming gameplay RNG immediately when playback starts.
        let num_sources = self.feedback.sound_sim.sources.num_sources();
        for i in 0..num_sources {
            let Some(src) = self.feedback.sound_sim.sources.get_mut(i) else {
                continue;
            };
            if !src.active || src.source_kind != crate::sound_source::SoundSourceKind::Delayed {
                continue;
            }
            if src.timer > 0 {
                src.timer -= 1;
            }
            if src.timer == 0 {
                if self
                    .feedback
                    .sound_sim
                    .playing_sources
                    .iter()
                    .any(|playing| playing.source_index as usize == i)
                {
                    continue;
                }
                let duration = assets.source_durations.get(&src.id).copied().unwrap_or(0);
                self.feedback
                    .sound_sim
                    .playing_sources
                    .push(crate::sound::PlayingSource {
                        source_index: i as u32,
                        finish_frame: self.control.frame_counter + duration,
                    });
                self.feedback
                    .pending_side_effects
                    .sounds
                    .push(super::SoundCommand::PlayDelayedSource(i));
            }
        }

        // `perform_frame_hourglass` temporarily moves the authoritative
        // camera display state into this argument. Advance that exact value;
        // taking `cutscene_camera.display` again here would tick a fresh
        // default and then overwrite it when the outer value is restored.
        let skip_render = self.tick_display_state(display);

        // Original's portrait refresh mirrors these fields from canonical
        // profile/status/interface state. Event-driven open, burn, and
        // quick-icon fields are intentionally not derived here.
        let portrait_updates: Vec<_> = self
            .world
            .pc_ids
            .iter()
            .copied()
            .map(|pc_id| {
                let pc = self
                    .get_entity(pc_id)
                    .and_then(|entity| entity.pc_data())
                    .unwrap_or_else(|| panic!("PC list entry {pc_id:?} is not a PC"));
                let profile = assets
                    .profile_manager
                    .get_character(pc.profile_index)
                    .unwrap_or_else(|| {
                        panic!("PC {pc_id:?} has missing profile {}", pc.profile_index)
                    });
                let description = self
                    .pc_description_for_pc_data(pc)
                    .unwrap_or_else(|| panic!("PC {pc_id:?} has no campaign description"));
                (
                    pc_id,
                    profile
                        .actions
                        .map(|action| description.status.get_ammo(action)),
                    profile.actions[2] == crate::profiles::Action::NoAction,
                    !pc.interface_hidden,
                    f32::from(pc.life_points),
                    pc.trumpet_enabled,
                )
            })
            .collect();
        for (pc_id, quantities, two_buttons, displayed, life, trumpet) in portrait_updates {
            let pc = self
                .get_entity_mut(pc_id)
                .and_then(|entity| entity.pc_data_mut())
                .unwrap_or_else(|| panic!("PC list entry {pc_id:?} is not a PC"));
            pc.portrait.quantities = quantities;
            pc.portrait.two_buttons_mode = two_buttons;
            pc.portrait.displayed = displayed;
            pc.portrait.life_level = life;
            pc.portrait.trumpet_enabled = trumpet;
        }

        // Reset per-frame scroll dedupe after the camera display tick.
        // Host-local viewport scroll is host-side and never enters engine
        // state, so peer-2's held scroll doesn't gate the host's, and vice
        // versa.
        display.frame_scrolled = [false; 4];

        let mut fx = self.feedback.drain_side_effects();
        fx.code = code;
        // The trigger tick supplies the first FadeToBlack presentation.
        // Force that render even when the camera state machine requested a
        // fast-forward skip; the remaining presentations are forced by the
        // early-return path above.
        let starts_fade = matches!(fx.fade_to_black, Some(Some(_)));
        fx.skip_render = !starts_fade && skip_render != 0;
        fx
    }

    pub(crate) fn apply_pending_arrow_refresh(&mut self, sim: &crate::sim_rng::SimulationContext) {
        if !std::mem::take(&mut self.control.arrow_refresh_pending) {
            return;
        }

        self.refresh_arrows_for_presentation(sim);
    }

    /// Run the arrow portion of `RHGame::Refresh` immediately.
    ///
    /// Besides the ordinary post-snapshot refresh, Original can re-enter
    /// `RHGame::Refresh(false, false)` while constructing an in-game modal.
    /// Dialogue commands do that synchronously, so a newly-created arrow can
    /// publish its orientation before the same frame's parity snapshot.
    pub(crate) fn refresh_arrows_for_presentation(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
    ) {
        // Refresh walks the full SortForDisplay result. Its FX-polyline merge
        // can interleave (and even reverse) two non-animation arrows that an
        // arrow-only depth sort would leave together. That exact order is
        // authoritative because every falling-arrow Refresh consumes one
        // global RNG draw.
        let arrows: Vec<_> = self
            .compute_display_order()
            .ids
            .into_iter()
            .filter(|&id| {
                matches!(
                    self.world.entities.get(id),
                    Some(Entity::Projectile(projectile))
                        if projectile.object.object_type == crate::element::ObjectType::Arrow
                )
            })
            .collect();

        for id in arrows {
            let Some(Entity::Projectile(projectile)) = self.world.entities.get_mut(id) else {
                panic!("arrow {id:?} vanished during deferred Refresh");
            };
            crate::bow_shot::refresh_arrow_after_previous_hourglass(sim, projectile);
        }
    }

    /// Run the one-shot mission-script `PostInitialize` stage.
    ///
    /// The original `RHGame::GameLoop` calls this after the first
    /// `Refresh(true, true)` and `RHSound::Hourglass`, not from inside
    /// `RHEngine::PerformHourglass`.  The host therefore invokes this
    /// explicit stage after its first refresh/sound boundary.  Rollback
    /// replay invokes the same stage after replaying frame zero so the
    /// resulting pre-frame-one simulation state remains deterministic.
    fn perform_post_initialize_authoritative(
        &mut self,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
    ) -> Option<super::SideEffects> {
        let needs_post_initialize = self
            .scripts
            .mission
            .as_ref()
            .is_some_and(|script| !script.post_initialized);
        if !needs_post_initialize {
            return None;
        }

        // PostInitialize can call randomising natives, so keep it on the same
        // engine-owned deterministic stream while moving only the scheduling
        // boundary.
        let sim = self.control.simulation_context();
        let sim = &sim;

        // This explicit host stage is defined to run after the first native
        // Refresh. Cross the same pending arrow boundary before PostInitialize
        // can consume RNG or inspect sprite state.
        self.apply_pending_arrow_refresh(sim);

        self.run_post_initialize_if_needed(sim, assets);
        self.drain_pending_immediate_actions_sync(sim, display, assets);

        let mut fx = self.feedback.drain_side_effects();
        fx.code = GameCode::LevelInProgress;
        Some(fx)
    }

    #[cfg(test)]
    pub(crate) fn perform_post_initialize(
        &mut self,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
    ) -> Option<super::SideEffects> {
        let mut camera = self.feedback.cutscene_camera.display.clone();
        let effects = self.perform_post_initialize_authoritative(&mut camera, assets);
        self.feedback.cutscene_camera.display = camera;
        if let Some(effects) = &effects {
            let mut input = InputState::default();
            for event in effects.host_events.iter().cloned() {
                display.apply_host_event(&mut input, event);
            }
        }
        effects
    }

    /// Whether any PC is currently guarded.
    pub fn is_pc_guarded(&self) -> bool {
        for &pc_id in &self.world.pc_ids {
            if let Some(Entity::Pc(pc)) = self.get_entity(pc_id)
                && pc.pc.guard.is_some()
            {
                return true;
            }
        }
        false
    }

    fn perform_hourglass_inner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        simulation_body_allowed: bool,
    ) -> GameCode {
        let pc_guarded = time_hourglass_phase(HourglassPhase::DeferredEffectsStart, || {
            self.hourglass_phase_deferred_effects_start(sim, assets)
        });

        if let Some(code) = time_hourglass_phase(HourglassPhase::MissionAndMessages, || {
            self.hourglass_phase_mission_and_messages(
                sim,
                display,
                assets,
                pc_guarded,
                simulation_body_allowed,
            )
        }) {
            return code;
        }

        time_hourglass_phase(HourglassPhase::NpcOrders, || {
            self.hourglass_phase_npc_orders(sim, assets)
        });

        time_hourglass_phase(HourglassPhase::Paths, || {
            self.hourglass_phase_paths(sim, assets)
        });

        let was_swordfighting = time_hourglass_phase(HourglassPhase::Entities, || {
            self.hourglass_phase_entities(sim, assets)
        });

        let positions_before_movement = time_hourglass_phase(HourglassPhase::EntitySystems, || {
            self.hourglass_phase_entity_systems(sim, display, assets)
        });

        time_hourglass_phase(HourglassPhase::Npcs, || {
            self.hourglass_phase_npcs(sim, assets, &positions_before_movement)
        });

        time_hourglass_phase(HourglassPhase::GameplaySystems, || {
            self.hourglass_phase_gameplay_systems(sim, display, assets)
        });

        time_hourglass_phase(HourglassPhase::Sequences, || {
            self.hourglass_phase_sequences_authoritative(sim, display, assets)
        });

        // `RHSequenceElement::SetState(Terminated)` calls the owner's
        // `SendCondolationCard`, then `RHSequence::Ready`, synchronously
        // (`original-code/RHsequenceelement.cpp:280-296`). `Ready` immediately
        // starts the next command level (`RHsequence.cpp:304-313`), so an
        // immediate Timer successor must be installed before RHEngine reaches
        // its anonymous-timer scan at RHengine.cpp:3755-3771. Rust defers the
        // borrow-reentrant card itself, but this barrier must stay on the
        // sequence-manager side of that scan.
        self.dispatch_condolations(sim, assets);

        // `RHSequenceManager::Hourglass` runs before the anonymous-timer
        // scan. If a deferred command terminates and advances its sequence
        // to an immediate Timer, C++ executes that Timer re-entrantly, adds
        // it to `mlistTimerElements`, and decrements it later in this same
        // tick. Drain that immediate continuation here so Rust preserves the
        // same launch-frame decrement. Waiting until DeferredEffectsEnd's
        // final drain makes every such timer one frame late.
        self.drain_pending_immediate_actions_sync(sim, display, assets);

        time_hourglass_phase(HourglassPhase::DeferredEffectsEnd, || {
            self.hourglass_phase_deferred_effects_end(sim, display, assets, was_swordfighting)
        });

        GameCode::LevelInProgress
    }

    /// Consume the host sound-manager update that completed after the
    /// preceding Original engine frame.
    ///
    /// Parity traces attach these between-frame resolutions to the following
    /// frame record: `RHGame` records the engine frame before calling
    /// `RHSound::Hourglass` (`original-code/RHgame.cpp:1879-1915`). Replay
    /// must therefore be able to run this boundary before applying the
    /// following frame's recorded input commands.
    pub(super) fn hourglass_phase_sound_boundary(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) -> Result<(), String> {
        // Sound Hourglass completes after the preceding Engine frame in the
        // Original. Its callbacks therefore finish before the next
        // PerformHourglass begins and must be the first mutation here.
        let cur_frame = self.control.frame_counter;
        drain_matured_exclamations(&mut self.feedback.sound_sim, cur_frame);
        // Original invokes SoundIsFinished inline while walking the pending
        // sound list. That callback may synchronously Think/Say and append a
        // request which a later resolution in this same boundary consumes.
        self.settle_npc_speech_completions(sim, assets);
        let replay_injected_resolutions = std::mem::take(
            &mut self
                .feedback
                .sound_sim
                .replay_injected_resolved_exclamations,
        );
        let resolutions = std::mem::take(&mut self.feedback.sound_sim.resolved_exclamations);
        for resolution in resolutions {
            self.debug_speech_lifecycle(
                resolution.actor_id,
                "resolution_enter",
                (
                    resolution.exclamation_id,
                    resolution.identifier,
                    resolution.duration_frames,
                ),
            );
            let pending = self
                .feedback
                .sound_sim
                .pending_exclamations
                .first()
                .cloned();
            let matches_pending = pending.as_ref().is_some_and(|pending| {
                (pending.actor_id, pending.exclamation_id)
                    == (resolution.actor_id, resolution.exclamation_id)
            });
            if matches_pending {
                let pending = pending.expect("matching pending exclamation disappeared");
                let expected_identifier =
                    (pending.profile_id & 0xFFFF_0000) | u32::from(pending.exclamation_id);
                if expected_identifier != resolution.identifier {
                    return Err(format!(
                        "sound manager resolved identifier {} for actor {}, but pending request expects {}",
                        resolution.identifier, resolution.actor_id, expected_identifier
                    ));
                }
                self.feedback.sound_sim.pending_exclamations.remove(0);
            } else if replay_injected_resolutions {
                // The host sound queue is not serialized in legacy saves.
                // Schema-16 records its concrete between-frame completions,
                // while Rust can independently reconstruct a different
                // logical request from adopted NPC state.  An authoritative
                // Original completion must not consume that unrelated Rust
                // FIFO entry; process its timing below exactly as in the
                // empty-pending case.
                tracing::warn!(
                    actor_id = resolution.actor_id,
                    exclamation_id = resolution.exclamation_id,
                    identifier = resolution.identifier,
                    duration_frames = resolution.duration_frames,
                    pending = ?self.feedback.sound_sim.pending_exclamations,
                    "replay injected an authoritative Original host exclamation that does not match the Rust logical FIFO"
                );
            } else if pending.is_some() {
                return Err(format!(
                    "sound-manager resolution order diverged for actor {} exclamation {}; pending FIFO: {:?}",
                    resolution.actor_id,
                    resolution.exclamation_id,
                    self.feedback.sound_sim.pending_exclamations
                ));
            } else {
                return Err(format!(
                    "live sound manager resolved exclamation {} for actor {} with no pending request",
                    resolution.exclamation_id, resolution.actor_id
                ));
            }
            if resolution.duration_frames == 0 {
                self.feedback
                    .sound_sim
                    .finished_exclamations
                    .push((resolution.actor_id, u32::from(resolution.exclamation_id)));
                self.settle_npc_speech_completions(sim, assets);
            } else {
                self.feedback.sound_sim.playing_exclamations.push(
                    crate::sound::PlayingExclamation {
                        actor_id: resolution.actor_id,
                        exclamation_id: u32::from(resolution.exclamation_id),
                        finish_frame: cur_frame + resolution.duration_frames,
                    },
                );
            }
        }
        Ok(())
    }

    /// Drain effects deferred by the preceding tick before any mission,
    /// entity, path, NPC, or sequence work observes this frame's state.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3446-3548` starts
    /// `RHEngine::PerformHourglass` with host/widget and mission-state work.
    /// These Rust-owned queues have no one-to-one original equivalent; their
    /// relative placement is retained from the pre-decomposition Rust tick.
    pub(super) fn hourglass_phase_deferred_effects_start(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) -> bool {
        self.hourglass_phase_sound_boundary(sim, assets)
            .unwrap_or_else(|reason| panic!("internal sound boundary rejected: {reason}"));
        let cur_frame = self.control.frame_counter;
        // Drain deferred console-cheat / death reinforcement spawns and
        // scroll-reveal amulet spawns. Both used to live in
        // `Game::run_engine_tick` because they needed `&mut LevelAssets`
        // to load sprites; the two sprite families are now preloaded at
        // mission start (`preload_campaign_peasant_sprites`,
        // `preload_scroll_amulet_sprite`) so the spawn paths read the
        // scriptor cache via `&LevelAssets` and the whole flow lives
        // inside `perform_hourglass` — keeping the "sim mutation only
        // during perform_hourglass" invariant intact.
        self.drain_pending_reinforcements(sim, assets);
        self.drain_pending_scroll_amulets(sim, assets);
        self.drain_pending_hero_speeches(assets);
        self.drain_pending_hades_kills(sim, assets);
        self.drain_pending_concussion_side_effects(sim, assets);

        // Drain matured sound-source finishes.  Replaces the
        // `stop_sound_source` logic the Rust host used to run on
        // Audio-backend playback-completion events: for each scheduled
        // source whose sim-frame deadline has arrived, `Single` sources
        // flip to `active = false` and `Volatile` sources are deleted
        // from the manager.  `Delayed` / `Looped` never land in
        // `playing_sources` (Delayed re-rolls itself below; Looped
        // doesn't terminate on its own), so this drain only ever sees
        // Single/Volatile; still match exhaustively to fail loudly if
        // a kind ever leaks into the queue.
        let mut still_playing_sources = Vec::new();
        let mut source_deactivations: Vec<usize> = Vec::new();
        let mut source_deletions: Vec<usize> = Vec::new();
        let mut delayed_restarts: Vec<usize> = Vec::new();
        for p in self.feedback.sound_sim.playing_sources.drain(..) {
            if p.finish_frame > cur_frame {
                still_playing_sources.push(p);
                continue;
            }
            let Some(src) = self.feedback.sound_sim.sources.get(p.source_index as usize) else {
                // Slot already cleared (e.g. Destroy command ran this
                // tick); drop the stale entry silently.
                continue;
            };
            match src.source_kind {
                crate::sound_source::SoundSourceKind::Single => {
                    source_deactivations.push(p.source_index as usize);
                }
                crate::sound_source::SoundSourceKind::Volatile => {
                    source_deletions.push(p.source_index as usize);
                }
                crate::sound_source::SoundSourceKind::Delayed => {
                    delayed_restarts.push(p.source_index as usize);
                }
                crate::sound_source::SoundSourceKind::Looped => {
                    tracing::warn!(
                        source_index = p.source_index,
                        kind = ?src.source_kind,
                        "sound source scheduled finish fired for Looped/Delayed kind — \
                         should never happen (schedule_source_finish skips them)"
                    );
                }
            }
        }
        self.feedback.sound_sim.playing_sources = still_playing_sources;
        for idx in source_deactivations {
            if let Some(src) = self.feedback.sound_sim.sources.get_mut(idx) {
                src.active = false;
            }
        }
        for idx in source_deletions {
            self.feedback.sound_sim.sources.delete(idx);
        }
        for idx in delayed_restarts {
            let Some(src) = self.feedback.sound_sim.sources.get_mut(idx) else {
                continue;
            };
            if src.delay_stepping > 0 && src.max_delay > src.min_delay {
                let seed = (u64::from(cur_frame) << 32) ^ (u64::from(src.id) << 8) ^ idx as u64;
                let step = crate::sim_rng::with_auxiliary_seed(
                    crate::sim_rng::AuxiliaryRngSite::DelayedSoundTimer,
                    seed,
                    |rng| rng.u32(0..u32::from(src.delay_stepping)),
                ) as u16;
                let range = src.max_delay - src.min_delay;
                src.timer = (u32::from(step) * u32::from(range) / u32::from(src.delay_stepping))
                    as u16
                    + src.min_delay;
            } else {
                src.timer = src.min_delay;
            }
        }

        // PC-guarded state drives start/quit mission widget enable and
        // guard-portrait blinking.  The
        // widget-enable side is applied from `Game::run_engine_tick`
        // before `perform_hourglass` runs so both consumers see the
        // same value for this tick.  The guard-portrait blink is
        // rendered live by `ui_panel.rs` directly from
        // `mission.mission_won` + `PcData::guard`, so there's nothing
        // to do here for (b).

        self.is_pc_guarded()
    }

    /// Run mission gates, the once-per-second script, clock advancement, and
    /// the tick's messenger drain. Returning a code short-circuits every later
    /// phase exactly where the monolithic implementation did.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3470-3664` performs
    /// mission/UI gates, script callbacks, counter advancement, lock checks,
    /// loss checks, and reinforcement notification in this order.
    fn hourglass_phase_mission_and_messages(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        pc_guarded: bool,
        simulation_body_allowed: bool,
    ) -> Option<GameCode> {
        // ── Anti-chorus timer ────────────────────────────────────
        if self.control.chorus_timer > 0 {
            self.control.chorus_timer -= 1;
        }

        // ── First-time mission-won message ───────────────────────
        // Fire the mission-state banner ("leave mission now" / quit
        // mission popup) and disable the quit-mission widget once the
        // player has reached a guarded exit AND no PC is currently
        // being guarded (guarded PCs can't lead everyone out yet).
        // We signal both via `SideEffects.pending_mission_state_notice`;
        // the host flips the widget-enable flag and shows the popup.
        if self.mission_domain.state.mission_won_first_time && !pc_guarded {
            self.mission_domain.state.mission_won_first_time = false;
            self.feedback
                .pending_side_effects
                .pending_mission_state_notice = true;
        }

        // ── Check quit conditions ────────────────────────────────
        // Each of the three quit branches displays the full minimap.
        if self.mission_domain.state.quit_won {
            self.feedback
                .pending_side_effects
                .host_events
                .push(HostEvent::Minimap(MinimapHostEvent::DisplayMap {
                    show: false,
                    restore_position: true,
                }));
            self.finalize_mission_script(sim, assets, false);
            return Some(GameCode::LevelSucceeded);
        }
        if self.mission_domain.state.quit_lost {
            self.feedback
                .pending_side_effects
                .host_events
                .push(HostEvent::Minimap(MinimapHostEvent::DisplayMap {
                    show: false,
                    restore_position: true,
                }));
            self.quit_mission();
            return Some(GameCode::LevelFailed);
        }
        if self.mission_domain.state.quit_interrupted {
            self.feedback
                .pending_side_effects
                .host_events
                .push(HostEvent::Minimap(MinimapHostEvent::DisplayMap {
                    show: false,
                    restore_position: true,
                }));
            self.finalize_mission_script(sim, assets, true);
            return Some(GameCode::LevelInterrupted);
        }

        // ── Cheat display all dialogs/briefings ──────────────────
        // After the engine/host carve-out (Decision 9) level descriptors
        // live host-side.  `all_dialogues`, `all_popup_texts` and
        // `all_debriefings` are expanded by `game_session` after the
        // tick returns — it has the descriptor on hand and pushes every
        // registered ID straight onto the host-side pending queues.

        // ── Script tick (once per game-second) ──────────────────────
        // The main loop runs at 25 Hz (40 ms frame time), and the
        // script's Hourglass fires only when
        // `frame_counter % 25 == 0` — i.e. once per real second — with
        // the game-second index as its argument.
        if self.control.sim_config.script_enabled
            && self.control.frame_counter.is_multiple_of(FRAMES_PER_SECOND)
        {
            let game_seconds = self.control.frame_counter / FRAMES_PER_SECOND;

            if let Err(error) = self.call_script_vm(
                sim,
                assets,
                super::ScriptVmKey::Global,
                "Hourglass",
                &[game_seconds as i32],
                crate::natives::ScriptCallFrame::default(),
            ) {
                tracing::warn!("Script Hourglass error: {error}");
            }

            // Check victory/defeat conditions every 3 game-seconds
            // (or immediately if force_check was set by a native call).
            if game_seconds.is_multiple_of(VICTORY_CHECK_INTERVAL)
                || self.script_domains.mission_ui.force_check
            {
                self.script_domains.mission_ui.force_check = false;

                {
                    let victory_result = self.call_script_vm(
                        sim,
                        assets,
                        super::ScriptVmKey::Global,
                        "CheckVictoryCondition",
                        &[game_seconds as i32],
                        crate::natives::ScriptCallFrame::default(),
                    );
                    match victory_result {
                        Ok(1) => {
                            // Mission won!
                            if !self.mission_domain.state.mission_won {
                                // Don't show the "leave mission" message for
                                // ambush or tactical missions (they end immediately).
                                let show_window = !matches!(
                                    self.mission_type(&assets.profile_manager),
                                    Some(MissionType::Ambush | MissionType::Tactical)
                                );
                                self.win(show_window);
                            }
                        }
                        Ok(2) => {
                            // Script says mission lost
                            self.quit_mission();
                            return Some(GameCode::LevelFailed);
                        }
                        Ok(_) => {} // 0 or other = still in progress
                        Err(e) => {
                            tracing::warn!("Script CheckVictoryCondition error: {e}");
                        }
                    }
                }
            }
        }

        // ── Increment frame counter ──────────────────────────────
        self.advance_mission_clock();

        // ── Skip logic if engine is locked (zoom, sequence, etc) ─
        if display.background_transform.zoom_to_up
            || display.background_transform.zoom_to_down
            || self.engine_locked()
            || !simulation_body_allowed
        {
            return Some(GameCode::LevelInProgress);
        }

        // ── Default lose condition check ─────────────────────────
        // Guarded by `ignore_default_loose`.
        // Missions that keep-an-NPC-alive (e.g. "protect the cart")
        // set this flag to true so the default "all PCs dead/guarded /
        // dead-PC / civilian-killed" loss checks are skipped; the
        // script's `CheckVictoryCondition` is the authority instead.
        let ignore_default_loose = self.control.sim_config.ignore_default_loose;
        if !ignore_default_loose {
            // Original: RHEngine::PerformHourglass checks the PC's explicit
            // IsPlayable() flag and guard state. Death paths are responsible
            // for clearing playability; do not substitute an HP/posture test.
            // Only actors authored as player-party members participate in
            // the ordinary party-defeat rule. Controller, allegiance, and
            // hero body type are deliberately independent from this role.
            let has_player_party = self.world.pc_ids.iter().any(|&pc_id| {
                matches!(
                    self.world.entities.get(pc_id),
                    Some(Entity::Pc(pc))
                        if pc.pc.mission_role == crate::human_control::MissionRole::PlayerParty
                )
            });
            if has_player_party {
                let any_playable_and_free = self.world.pc_ids.iter().any(|&pc_id| {
                    if let Some(Entity::Pc(pc)) = self.world.entities.get(pc_id) {
                        let guarded = pc.pc.guard.is_some();
                        pc.pc.mission_role == crate::human_control::MissionRole::PlayerParty
                            && pc.pc.playable
                            && !guarded
                    } else {
                        false
                    }
                });
                if !any_playable_and_free {
                    tracing::info!("No playable, unguarded PC remains; mission lost");
                    self.quit_mission();
                    return Some(GameCode::LevelFailed);
                }
            }

            // Check if a dead PC was flagged for mission failure
            if let Some(dead_id) = self.mission_domain.dead_pc.take() {
                if let Some(entity) = self.get_entity(dead_id) {
                    let pos = entity.element_data().position_map();
                    self.center_on_point(0, pos);
                }
                self.quit_mission();
                return Some(GameCode::LevelFailed);
            }

            // Check if any civilian was killed (not by accident) → mission failure
            let mut killed_civilian = None;
            for (npc_id, civilian) in self.world.entities.civilians() {
                if civilian.element.posture.is_dead() {
                    let npc_id: EntityId = npc_id.into();
                    // Check killed_by_accident via the civilian's human data
                    let accident = civilian.human.killed_by_accident;
                    if !accident {
                        killed_civilian = Some(npc_id);
                        break;
                    }
                }
            }
            if let Some(civ_id) = killed_civilian {
                if let Some(entity) = self.get_entity(civ_id) {
                    let pos = entity.element_data().position_map();
                    self.center_on_point(0, pos);
                }
                self.quit_mission();
                return Some(GameCode::LevelFailed);
            }
        }

        // ── Send reinforcement messages ──────────────────────────
        //
        // For every PC, decrement `time_till_reinforcement` and, the
        // tick it hits zero, enqueue a reinforcement spawn directly
        // (skipping the messenger round-trip the original used).
        // `drain_pending_reinforcements` already handles the
        // `&mut LevelAssets` needed for sprite loading, and the
        // intermediate message was never observed by anything else.
        let pc_ids_for_reinf: Vec<EntityId> = self.world.pc_ids.clone();
        for pc_id in pc_ids_for_reinf {
            let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id) else {
                continue;
            };
            let arrived = match pc.pc.time_till_reinforcement {
                0xFFFF_FFFF => false,
                0 => {
                    pc.pc.time_till_reinforcement = 0xFFFF_FFFF;
                    true
                }
                ref mut t => {
                    *t -= 1;
                    false
                }
            };
            if arrived {
                self.orders.pending_reinforcements.push(Some(pc_id));
            }
        }

        // ── Process messenger (engine-state messages) ────────────
        // Handle pending messages that mutate engine state. Other
        // messages (UI/mission flow) are left in the queue for their
        // respective consumers (UI layer, tests, etc.) to observe.
        // We only consume the ones that actually affect engine state.
        {
            // `RHMessenger::ForwardMessage` is synchronous and recursive:
            // a message emitted while handling another message completes
            // before the outer call resumes.  Keep host/UI-only messages for
            // their downstream consumer, but prepend newly emitted messages
            // to the remaining engine work so their observable state changes
            // happen depth-first in this frame.
            let mut messages: std::collections::VecDeque<_> = self.orders.messenger.drain().into();
            let mut downstream = std::collections::VecDeque::new();
            while let Some(msg) = messages.pop_front() {
                match msg.msg_type {
                    MessageType::Simple(SimpleMessage::LockAlt) => {
                        self.players.seats[0].is_lock_alt = true;
                    }
                    MessageType::Simple(SimpleMessage::UnlockAlt) => {
                        self.players.seats[0].is_lock_alt = false;
                    }
                    // Macro recording state machine.  The PC id is
                    // passed via the message: a present id targets one
                    // specific PC; an absent id arms every currently-
                    // selected PC.
                    MessageType::Pc(crate::messenger::PcMessage::StartRecordingMacro, pc) => {
                        let slot = self.players.qa_recording_slot;
                        let targets: Vec<crate::element::EntityId> = match pc {
                            Some(id) => vec![id],
                            None => self.players.seats[0].selection.clone(),
                        };
                        for pc_id in &targets {
                            self.players
                                .macro_store
                                .get_or_insert(*pc_id)
                                .begin_recording(slot);
                            let pc = self
                                .get_entity_mut(*pc_id)
                                .and_then(|entity| entity.pc_data_mut())
                                .unwrap_or_else(|| {
                                    panic!("quick-action recording target {pc_id:?} is not a PC")
                                });
                            pc.portrait.quick_icons[slot as usize] = Default::default();
                        }
                        self.players.qa_recording_for = targets;
                        // Snapshot the currently-armed action so the
                        // MSG_STOP_RECORDING_MACRO post-process can
                        // restore it.
                        self.players.action_before_recording_macro = self.get_selected_action();
                    }
                    MessageType::Pc(crate::messenger::PcMessage::StopRecordingMacro, _) => {
                        // Suppress the post-process restore unless
                        // something was actually recording.
                        let was_recording = !self.players.qa_recording_for.is_empty();
                        self.stop_recording_macro();

                        // Post-process: re-select the action that was
                        // armed before recording started.  Apply the
                        // saved action to each selected PC directly —
                        // we do not route MSG_SELECT_ACTION through
                        // the messenger drain.
                        if was_recording {
                            let restore = self.players.action_before_recording_macro;
                            self.players.action_before_recording_macro =
                                crate::profiles::Action::NoAction;
                            self.players.seats[0].selected_action = restore;
                            for id in self.players.seats[0].selection.clone() {
                                if let Some(entity) = self.get_entity_mut(id)
                                    && let Some(pc) = entity.pc_data_mut()
                                {
                                    pc.current_action = restore;
                                }
                            }
                            // Emit the message for script /
                            // edge-subscriber observation.
                            self.orders
                                .messenger
                                .send(crate::messenger::Message::pc_with_value(
                                    crate::messenger::PcMessage::SelectAction,
                                    None,
                                    restore as u32,
                                ));
                        }
                    }
                    MessageType::Pc(crate::messenger::PcMessage::UpdateRecordingMacro, _) => {
                        // When a recording is live, end it on PCs no
                        // longer selected and start it on any newly-
                        // selected PC — keeping the slot index stable
                        // across selection changes.
                        if !self.players.qa_recording_for.is_empty() {
                            let slot = self.players.qa_recording_slot;
                            let selected: Vec<crate::element::EntityId> =
                                self.players.seats[0].selection.clone();
                            // End on PCs that left the selection.
                            let current = self.players.qa_recording_for.clone();
                            for pc_id in &current {
                                if !selected.contains(pc_id)
                                    && let Some(state) = self.players.macro_store.get_mut(*pc_id)
                                {
                                    state.stop_recording();
                                }
                            }
                            // Start on PCs newly selected.
                            for pc_id in &selected {
                                if !current.contains(pc_id) {
                                    self.players
                                        .macro_store
                                        .get_or_insert(*pc_id)
                                        .begin_recording(slot);
                                }
                            }
                            self.players.qa_recording_for = selected;
                        }
                    }
                    MessageType::Pc(crate::messenger::PcMessage::SendReinforcement, pc) => {
                        // `MSG_SEND_REINFORCEMENT` plays the "new peasant
                        // called" jingle and sets the PC's cooldown to
                        // 100 ticks.  The cooldown poll in the tick
                        // above spawns the replacement when the counter
                        // hits zero.
                        if let Some(pc_id) = pc
                            && let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id)
                        {
                            pc.pc.time_till_reinforcement = 100;
                        }
                        self.feedback.pending_side_effects.sounds.push(
                            super::SoundCommand::Jingle(crate::sound::Jingle::NewPeasantCalled),
                        );
                    }
                    // PC-info hover popup is HQ-only (Sherwood) — go
                    // through `request_pc_info_overlay` so that gate
                    // is honored.
                    //
                    // UI-has-focus: another UI widget grabbed input
                    // focus — hide any live PC-info hover popup.
                    // Emitted from the minimap drag handler
                    // (commands.rs) and should be emitted from any
                    // future in-game widget that grabs focus.
                    //
                    // The Rust port keeps the mouse focus gate on
                    // host-owned `InputState`; `run_engine_tick_core`
                    // consumes the side effect below and clears that
                    // latch before later mouse dispatch can see it.
                    MessageType::Simple(crate::messenger::SimpleMessage::UiHasFocus) => {
                        self.request_pc_info_overlay(assets, None);
                        // Raise the host-side per-frame `ui_focus`
                        // latch; the host clears it at end of
                        // `update_mouse`.
                        self.feedback.pending_side_effects.ui_has_focus = true;
                    }
                    MessageType::Pc(crate::messenger::PcMessage::ShowPcInformation, pc) => {
                        self.request_pc_info_overlay(assets, pc);
                    }
                    MessageType::Pc(crate::messenger::PcMessage::HidePcInformation, _) => {
                        self.request_pc_info_overlay(assets, None);
                    }
                    // The four `SelectCharacter[Add][WithEcho]` arms
                    // all route through `select_pc` with the
                    // appropriate (multi-select, speak) flags.
                    MessageType::Pc(crate::messenger::PcMessage::SelectCharacter, Some(pc_id)) => {
                        // Tick messenger drains: ambient single-seat
                        // semantics; LOCAL seat for now.
                        self.select_pc(assets, 0, pc_id, false, false);
                        self.emit_character_selection_followups();
                    }
                    MessageType::Pc(
                        crate::messenger::PcMessage::SelectCharacterWithEcho,
                        Some(pc_id),
                    ) => {
                        self.select_pc(assets, 0, pc_id, false, true);
                        self.emit_character_selection_followups();
                    }
                    MessageType::Pc(
                        crate::messenger::PcMessage::SelectAddCharacter,
                        Some(pc_id),
                    ) => {
                        self.select_pc(assets, 0, pc_id, true, false);
                        self.emit_character_selection_followups();
                    }
                    MessageType::Pc(
                        crate::messenger::PcMessage::SelectAddCharacterWithEcho,
                        Some(pc_id),
                    ) => {
                        self.select_pc(assets, 0, pc_id, true, true);
                        self.emit_character_selection_followups();
                    }
                    // `pc == None` drops the whole selection;
                    // otherwise remove the specific PC.  Producers:
                    // `tick.rs:L4279` (dying / KO'd PC), `LockUser`,
                    // `DisableCharacter` (below).
                    MessageType::Pc(crate::messenger::PcMessage::UnselectCharacter, pc) => {
                        // Sherwood-only: on `pc == None`, mark every
                        // PC's interface hidden; otherwise hide just
                        // that PC's.  Engine side clears the selection
                        // list separately.
                        if self.is_sherwood(&assets.profile_manager) {
                            match pc {
                                None => {
                                    let ids = self.world.pc_ids.clone();
                                    for id in ids {
                                        if let Some(crate::element::Entity::Pc(pc)) =
                                            self.get_entity_mut(id)
                                        {
                                            pc.pc.interface_hidden = true;
                                        }
                                    }
                                }
                                Some(pc_id) => {
                                    if let Some(crate::element::Entity::Pc(pc)) =
                                        self.get_entity_mut(pc_id)
                                    {
                                        pc.pc.interface_hidden = true;
                                    }
                                }
                            }
                        }
                        match pc {
                            None => self.unselect_all_pcs(0),
                            Some(pc_id) => self.unselect_single_pc(pc_id),
                        }
                        self.emit_character_selection_followups();
                    }
                    // The engine drops the PC from the selection and
                    // (outside Sherwood) removes the portrait.  The
                    // portrait strip in Rust immediate-mode renders
                    // from `pc_ids` filtered by `pc.playable`, so the
                    // "portrait disappears" side effect is covered by
                    // the native already writing `pc.playable = false`
                    // at `natives/mod.rs:1546`.  Here we only need the
                    // selection-drop plus the Sherwood interface flag.
                    MessageType::Pc(crate::messenger::PcMessage::DisableCharacter, pc) => {
                        if let Some(pc_id) = pc {
                            self.unselect_single_pc(pc_id);
                            // Net effect: flip the interface-hidden
                            // flag only when we are NOT in Sherwood.
                            // Previously the gate was inverted; the
                            // effect was masked because
                            // `interface_hidden` is not read by the
                            // HUD path, but parity still matters for
                            // the `STATUS PC` cheat and future HUD
                            // wiring.
                            if !self.is_sherwood(&assets.profile_manager)
                                && let Some(crate::element::Entity::Pc(pc)) =
                                    self.get_entity_mut(pc_id)
                            {
                                pc.pc.interface_hidden = true;
                            }
                        }
                    }
                    // The portrait widget is re-added only outside
                    // Sherwood.  In Rust, the live HUD reads
                    // `pc.interface_hidden`; clear it whenever the
                    // portrait would have been re-added.  Sherwood
                    // also gets the same clear so the HUD panel
                    // re-shows the PC when re-activated mid-Sherwood.
                    MessageType::Pc(crate::messenger::PcMessage::EnableCharacter, pc) => {
                        if let Some(pc_id) = pc
                            && let Some(crate::element::Entity::Pc(pc)) = self.get_entity_mut(pc_id)
                        {
                            pc.pc.interface_hidden = false;
                        }
                    }
                    // After a modal (dialogue, popup, Sherwood report)
                    // closes, zero the cached mouse/keyboard state,
                    // clear the rubber-band selection and
                    // pending-drag / click suppression flags, and drop
                    // pressed-key edges queued during the modal.  The
                    // Rust equivalents live host-side across two
                    // InputState groups: ThreadedInput pressed-key
                    // cache (`pending_reset_input`) and the
                    // rubber-band / click-suppression flags
                    // (`reset_input`).
                    MessageType::Simple(crate::messenger::SimpleMessage::ResetInput) => {
                        self.feedback.pending_side_effects.pending_reset_input = true;
                        self.feedback.pending_side_effects.reset_input = true;
                        // Clear the alt-lock latch along with the
                        // modifier cache; without this, an alt-lock
                        // toggled before a console-hide / task-switch
                        // / save-load / unlock-user would persist
                        // past the reset.
                        self.players.seats[0].is_lock_alt = false;
                    }
                    // Ctrl-press saves the current action on every
                    // selected PC so the follow-on move command can
                    // run without the action overriding it (and the
                    // action is restored on ctrl-release).  Emitted
                    // by the host input layer when
                    // `GameAction::KeyControl` fires.
                    MessageType::Simple(crate::messenger::SimpleMessage::KeyControl) => {
                        self.save_action_for_selected_pcs(0);
                    }
                    // `LockUser` / `UnlockUser` flip `user_locked`.
                    // Scripts already set it directly via
                    // `Command::LockUser` (see tick.rs sequence-manager
                    // handler), but wiring the messenger variants
                    // keeps any non-script producer in sync with the
                    // engine-side flag that gates mouse events in
                    // `handle_mouse_input`.  Unlock also raises the
                    // `pending_reset_input` side-effect so held-key
                    // edges from the locked period are dropped.
                    MessageType::Simple(crate::messenger::SimpleMessage::LockUser) => {
                        self.players.user_locked = true;
                    }
                    MessageType::Simple(crate::messenger::SimpleMessage::UnlockUser) => {
                        self.players.user_locked = false;
                        self.feedback.pending_side_effects.pending_reset_input = true;
                    }
                    // After hiding the console or switching task,
                    // emit `MSG_RESET_INPUT` so the held-key edges
                    // and modifier latches don't bleed across the
                    // task boundary.
                    MessageType::Simple(crate::messenger::SimpleMessage::HideConsole)
                    | MessageType::Simple(crate::messenger::SimpleMessage::SwitchTask) => {
                        self.feedback.pending_side_effects.pending_reset_input = true;
                        self.feedback.pending_side_effects.reset_input = true;
                        // Same `is_lock_alt` clear as the explicit
                        // `ResetInput` arm above.
                        self.players.seats[0].is_lock_alt = false;
                    }
                    // `SelectActionSimple` and `DisableAction` both
                    // clear the aim-trajectory preview so a dropped /
                    // replaced action doesn't leave a stale trajectory
                    // overlay on screen.  `valid_trajectory` lives on
                    // `host` in the Rust split, so raise the
                    // side-effect flag.
                    MessageType::Pc(crate::messenger::PcMessage::SelectActionSimple, _)
                    | MessageType::Pc(crate::messenger::PcMessage::DisableAction, _) => {
                        if matches!(
                            msg.msg_type,
                            MessageType::Pc(crate::messenger::PcMessage::SelectActionSimple, _)
                        ) {
                            self.players.seats[0].selected_action =
                                crate::profiles::Action::try_from(msg.value).unwrap_or_else(|_| {
                                    panic!(
                                        "MSG_SELECT_ACTION_SIMPLE carried invalid action {}",
                                        msg.value
                                    )
                                });
                        }
                        self.feedback
                            .pending_side_effects
                            .invalidate_trajectory_preview = true;
                    }
                    // A macro fizzled on a PC's QA slot, so arm the
                    // per-slot titbit blink strobe.  Typed `pc` slot
                    // carries the PC id; `msg.value` is the QA slot
                    // index.  A `None` PC is treated as a no-op with
                    // a warning (the producer must always set one).
                    MessageType::Pc(crate::messenger::PcMessage::FizzleMacro, pc) => {
                        let slot = msg.value as usize;
                        match pc {
                            None => tracing::warn!(
                                "FizzleMacro received with no PC; \
                                 producer must set the PC id"
                            ),
                            Some(pc_id) => {
                                self.feedback.pending_side_effects.host_events.push(
                                    HostEvent::MacroUi(MacroUiHostEvent::BlinkQa { pc_id, slot }),
                                );
                            }
                        }
                    }
                    // `QaFocus` flashes the macro titbit for the
                    // focused QA slot.  Typed `pc` slot carries the
                    // PC (None = all PCs); `msg.value` encodes the
                    // slot index.
                    MessageType::Pc(crate::messenger::PcMessage::QaFocus, pc) => {
                        let slot = msg.value as usize;
                        match pc {
                            None => {
                                let pc_ids = self.world.pc_ids.clone();
                                for pc_id in pc_ids {
                                    self.set_blinking_for_slot(pc_id, slot);
                                }
                            }
                            Some(pc_id) => self.set_blinking_for_slot(pc_id, slot),
                        }
                    }
                    // Bulk-flip `disabled_actions_temp` on a specific
                    // PC (`Some(pc_id)`) or every selected PC
                    // (`None`).
                    MessageType::Pc(crate::messenger::PcMessage::DisableAllActionsTemp, pc) => {
                        // Tick messenger drain: ambient single-seat
                        // semantics; LOCAL seat for now.
                        self.apply_disable_all_actions_temp(0, pc);
                    }
                    MessageType::Pc(crate::messenger::PcMessage::EnableAllActionsTemp, pc) => {
                        self.apply_enable_all_actions_temp(assets, 0, pc);
                    }
                    // Other messages are consumed by downstream systems
                    // (UI layer, mission flow). Re-enqueue so those
                    // consumers can still observe them.
                    _ => downstream.push_back(msg),
                }

                // Preserve the send order of recursive calls while placing
                // them ahead of pre-existing sibling messages.
                for nested in self.orders.messenger.drain().into_iter().rev() {
                    messages.push_front(nested);
                }
            }
            for msg in downstream {
                self.orders.messenger.send(msg);
            }
        }

        None
    }

    /// Promote queued NPC intents before entity refresh and sequence dispatch.
    ///
    /// Original provenance: NPC AI was primarily reached through each NPC's
    /// `RHElement::Hourglass` in the original entity loop
    /// (`original-code/RHengine.cpp:3715-3723`). The Rust pre-pass is an
    /// architectural split; its exact parity remains audited separately.
    fn hourglass_phase_npc_orders(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_tactical_control(sim, assets);

        // ── Sequence manager cleanup ─────────────────────────────
        // Run every 256 frames (or every frame in debug).
        if self.control.frame_counter.is_multiple_of(256) {
            // Human's shoot list stores raw RHSequenceElement pointers. A
            // retail save can retain a terminal pointer past Friday cleanup;
            // the allocation then remains readable as stale legacy state.
            // Keep the Rust backing sequence alive while that explicit pointer
            // emulation exists, rather than turning the next ProcessShootList
            // call into a missing-element panic.
            let retained_shoot_sequences = self
                .world
                .entities
                .occupied()
                .filter_map(|(_, entity)| entity.human_data())
                .flat_map(|human| {
                    human
                        .pending_shoots
                        .iter()
                        .map(|element_ref| element_ref.sequence_id)
                })
                .collect::<std::collections::BTreeSet<_>>();
            self.orders
                .sequence_manager
                .friday_evening_cleanup_preserving(&retained_shoot_sequences);
        }

        // ── Process pending AI orders ─────────────────────────────
        //
        // AI Move intents collected by `launch_pending_orders_for_npc`
        // route through `launch_ai_move`, which just enqueues into
        // `pending_move_requests` (dedup-per-actor).  The drain below
        // promotes one Move sequence element per unique actor this
        // tick — absorbing redundant re-fires that would otherwise
        // launch a fresh Move each frame and `InterruptCurrent` the
        // in-flight one. A*-requiring elements enter the frame-paced
        // path-request queue advanced by the following `Paths` phase.
        self.process_pending_ai_orders(sim, assets);
        self.drain_pending_move_requests(sim);

        // ── Dispatch per-waypoint ReachPoint scripts ─────────────
        // When the AI reaches a scripted waypoint it queues the
        // dispatch on `pending_waypoint_script_reach_point`; we drain
        // the queue here, call `ReachPoint(actor)` on the waypoint's
        // VM, and push `EventAfterScriptGoOn` as a self-stimulus
        // unless the script pulled the NPC into `DefaultScriptDriven`.
        // Runs before `process_pending_cross_npc_actions` so the
        // self-stimulus drain at the end of that pass picks up the
        // `EventAfterScriptGoOn` in the same tick.
        self.dispatch_pending_waypoint_scripts(sim, assets);

        // ── Process cross-NPC actions (phalanx coordination) ────
        self.process_pending_cross_npc_actions(sim, assets);

        // ── Process AI animation orders ─────────────────────────
        // Drain Pointing/RaisingShield/etc orders from NPC order queues
        // and start them as active_ai_anim. EventDone fires when the
        // sprite animation completes (detected in tick_actor_animation_for).
        self.process_animation_orders();

        // TODO(original-parity): determine which queued NPC-order effects must
        // remain inside an individual NPC's creation-ordered Hourglass call.
    }

    /// Refresh every entity in stable entity-table (creation) order.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3715-3723` iterates
    /// `marrayElements`, which `SortForEngine` orders by creation order at
    /// `original-code/RHengine.cpp:7909-7944`, and removes dead elements inline.
    fn hourglass_phase_entities(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        _assets: &LevelAssets,
    ) -> bool {
        // Snapshot pre-hourglass swordfight state so we can detect a
        // swordfight→non-swordfight transition across this tick and
        // raise the ignore-mouse-event bracket on the falling edge.
        // The per-element / sequence-manager hourglass passes below may
        // flip the selected PC out of `Swordfighting`; when that
        // happens mid-drag the in-flight drag must be suppressed so it
        // doesn't bleed into the next click-release action.
        let was_swordfighting = self.is_selected_pc_swordfighting();

        // RHElementActorSoldier::Hourglass performs its subclass prelude
        // before delegating to RHElementActorNPC::Hourglass: apple smell,
        // primary-target tracking, and the reaction-time EnemyNear test.
        // In particular, keep the target snap introduced by 24c43efde ahead
        // of RefreshView without moving it into the base NPC phases.
        observe_npc_hourglass_phase(NpcHourglassPhase::SoldierPrelude);
        // Work runs at each soldier's live owner slot below.

        // First base-NPC phase in RHElementActorNPC::Hourglass. Patrol
        // history observes the actor before RHElementActorHuman::Hourglass
        // executes its movement/order work.
        observe_npc_hourglass_phase(NpcHourglassPhase::Patrol);
        // Work runs before the Human/Actor slices of each NPC owner below.

        // ── Element hourglass (per-element update) ───────────────
        observe_npc_hourglass_phase(NpcHourglassPhase::BaseHuman);
        // Human concussion healing runs synchronously in each owner's
        // pre-Actor hook below.
        // Concrete entity Hourglasses and their virtual retain/remove results
        // execute in the live owner walk below; there is no legacy base pass.

        // ── PC selection outline fade ────────────────────────────
        // The hulk state-machine block runs during the per-element
        // refresh pass.
        self.refresh_pc_selection_hulk();
        self.refresh_tactical_selection_hulks();

        // Tick the cheat-teleport hulk-rebuild fade counter on every
        // PC.  Decrementing here (rather than from the per-PC render
        // path) lets rollback / replay see bit-identical state (the
        // counter is serde'd `PcData`).
        self.tick_pc_teleport_fades();

        was_swordfighting
    }

    /// Advance queued pathfinding and failed-path deadlines before any entity
    /// refresh observes their state.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3697-3702` calls
    /// `ProcessPathRequests` once before collision and entity hourglasses;
    /// `original-code/RHpathfinder.cpp:710-765` returns at most one completed
    /// request and begins at most one successor at that scheduling point.
    pub(super) fn hourglass_phase_paths(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        // Rust computes A* synchronously, but the queue retains the original
        // one-call latency and one-completion-per-frame observation order.
        // Original starts its successor before returning the completed head
        // to RHEngine, so the scheduler closes that operation before the
        // coordinator applies cross-owner consequences.
        self.trace_path_barrier("enter");
        let completed = self
            .path_schedule_context()
            .process_requests(assets, sim.config().synchronous_pathfinding);
        self.trace_path_barrier("after_schedule");
        self.trace_path_barrier_completed("completed", &completed);
        self.apply_completed_path_work(sim, assets, completed);

        // ── Failed-path timeout ───────────────────────────────────
        // Move / Seek elements whose pathfind failed stay in `InProgress`
        // with empty orders for up to 100 frames without redispatch. Timeouts
        // mark the element `Impossible` and fire
        // `HERO_UNABLE_TO_DO_SOMETHING` for PCs. Classify one entry at a time
        // because each owner's condolation is synchronous and may invalidate
        // a later failed request before Original inspects it.
        while let Some(expired) = self.path_schedule_context().take_next_expired_failure() {
            let request = expired.request;
            if expired.owner_is_pc {
                self.hero_speaking(
                    assets,
                    request.owner,
                    crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                );
            }

            if let Some(element) = self
                .orders
                .sequence_manager
                .get_element_mut(request.seq_id, request.elem_idx)
            {
                element.command = crate::element::Command::MoveOk;
            }
            self.orders
                .sequence_manager
                .element_impossible(request.seq_id, request.elem_idx);
            // RHSequenceElement::SetState(RHSEQ_IMPOSSIBLE) invokes the
            // owner's SendCondolationCard synchronously inside
            // RHEngine::ProcessPathRequests. Close only this timeout's owner
            // boundary here, before collision and every element Hourglass;
            // leaving the card queued until the actor's insertion-order slot
            // lets earlier actors consume RNG before EVENT_COULDNT_REACHPOINT.
            self.dispatch_condolations_for_owner_boundary(sim, request.owner, assets);
            tracing::debug!(
                actor = ?request.owner,
                seq_id = ?request.seq_id,
                elem_idx = request.elem_idx,
                age = expired.age,
                "failed_path: 100-frame timeout expired — marking Impossible",
            );
        }

        // Original `CheckForCollision` follows ProcessPathRequests. Its only
        // implemented response is a human standing inside a non-stopped
        // mobile's motion polygon: launch RECEIVE_MOBILE_DAMAGE for 50/50
        // while the mobile moved last tick, otherwise 10/10.
        let mut humans: Vec<(EntityId, crate::coordinates::MapPoint)> = self
            .world
            .entities
            .humans()
            .map(|(id, human)| (id.into(), human.element_data().position_map()))
            .collect();
        humans.reverse();
        let mut impacts = Vec::new();
        for (human_id, position) in humans {
            for mobile in &self.world.mobile_elements {
                if !mobile.stopped && mobile.contains_point(position) {
                    let amount = if mobile.is_moving() { 50 } else { 10 };
                    impacts.push((human_id, mobile.sprite_ids[0], amount));
                }
            }
        }
        for (human_id, mobile_child, amount) in impacts {
            self.launch_element(crate::sequence::SequenceElement::new_damage(
                1,
                Command::ReceiveMobileDamage,
                Some(human_id),
                Some(mobile_child),
                amount,
                amount,
            ));
        }
    }

    /// Construct the path scheduler from exact leaf borrows of its two
    /// persistent owners. Cross-domain consequences deliberately remain in
    /// [`Self::hourglass_phase_paths`] after each scheduler operation returns.
    fn path_schedule_context(&mut self) -> PathScheduleContext<'_> {
        let frame_counter = self.control.frame_counter;
        let (entities, fast_grid, pathfinder) = self.world.path_schedule_parts();
        let (pending, failed, sequence_manager) = self.orders.path_schedule_parts();
        PathScheduleContext::new(
            frame_counter,
            entities,
            fast_grid,
            pathfinder,
            pending,
            failed,
            sequence_manager,
        )
    }

    fn trace_path_barrier(&self, stage: &str) {
        if std::env::var_os("PARITY_DEBUG_PATH_BARRIER").is_none() {
            return;
        }
        let pending = self
            .orders
            .pending_path_requests
            .parity_state(&self.world.fast_grid);
        let brief: Vec<_> = pending
            .1
            .iter()
            .map(|entry| {
                (
                    entry.request.actor,
                    entry.sequence_id,
                    entry.element_index,
                    entry.in_flight,
                    entry.waypoints.as_ref().map(|w| w.len()),
                )
            })
            .collect();
        eprintln!(
            "[PATH_BARRIER frame={} stage={stage} ignore={} queue={brief:?}]",
            self.control.frame_counter, pending.0
        );
    }

    fn trace_path_barrier_completed(&self, stage: &str, completed: &Option<CompletedPathWork>) {
        if std::env::var_os("PARITY_DEBUG_PATH_BARRIER").is_none() {
            return;
        }
        let brief = completed.as_ref().map(|work| match work {
            CompletedPathWork::Ready { request, waypoints } => (
                "ready",
                request.owner,
                request.seq_id,
                request.elem_idx,
                waypoints.len(),
            ),
            CompletedPathWork::Failed(request) => {
                ("failed", request.owner, request.seq_id, request.elem_idx, 0)
            }
        });
        eprintln!(
            "[PATH_BARRIER frame={} stage={stage} completed={brief:?}]",
            self.control.frame_counter
        );
    }

    fn apply_completed_path_work(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        completed: Option<CompletedPathWork>,
    ) {
        if let Some(owner) = completed.as_ref().map(|work| match work {
            CompletedPathWork::Ready { request, .. } | CompletedPathWork::Failed(request) => {
                request.owner
            }
        }) {
            assert!(
                self.world.entities.get(owner).is_some(),
                "completed path request for {owner:?} retains a live sequence element but its owner entity is missing"
            );
        }
        match completed {
            Some(CompletedPathWork::Ready { request, waypoints }) => {
                if let Some(element) = self
                    .orders
                    .sequence_manager
                    .get_element_mut(request.seq_id, request.elem_idx)
                {
                    element.command = crate::element::Command::MoveOk;
                }
                let _ = self.finish_move_path(sim, request, waypoints);
            }
            Some(CompletedPathWork::Failed(request)) => {
                tracing::warn!(
                    actor = ?request.owner,
                    seq_id = ?request.seq_id,
                    elem_idx = request.elem_idx,
                    src_x = request.source.x,
                    src_y = request.source.y,
                    dst_x = request.dest.x,
                    dst_y = request.dest.y,
                    layer = request.layer,
                    sector = request.sector,
                    "path scheduling barrier: pathfind FAILED",
                );
                if let Some(fallback) = self.tactical_path_failure_fallback(request.owner) {
                    if let Some(element) = self
                        .orders
                        .sequence_manager
                        .get_element_mut(request.seq_id, request.elem_idx)
                    {
                        element.command = crate::element::Command::MoveOk;
                    }
                    self.orders
                        .sequence_manager
                        .element_impossible(request.seq_id, request.elem_idx);
                    self.dispatch_condolations_for_owner_boundary(sim, request.owner, assets);
                    if let Some(destination) = fallback {
                        tracing::info!(
                            actor = ?request.owner,
                            failed_x = request.dest.x,
                            failed_y = request.dest.y,
                            fallback_x = destination.x,
                            fallback_y = destination.y,
                            "allied formation slot unreachable; moving toward shared command center",
                        );
                        self.perform_group_move(
                            sim,
                            assets,
                            &[request.owner],
                            destination,
                            false,
                            false,
                            None,
                            None,
                            None,
                            &[],
                            &[],
                        );
                    }
                } else {
                    self.orders.failed_path_requests.push(
                        super::movement::FailedPathRequest::from_pending(
                            request,
                            self.control.frame_counter,
                        ),
                    );
                }
            }
            None => {}
        }
    }

    /// Advance movement, animations, scripts, and the NPC-facing state that
    /// must be refreshed before the main AI pass.
    ///
    /// Original provenance: these responsibilities were distributed across
    /// individual `RHElement::Hourglass` implementations inside the original
    /// creation-ordered entity loop (`original-code/RHengine.cpp:3715-3723`).
    fn hourglass_phase_entity_systems(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
    ) -> EntitySlots<Option<crate::entities::BoundaryPosition>> {
        // Preserve the position each element exposed before the globally
        // batched movement pass. The original does not have this batch:
        // RHElementActorNPC::Hourglass calls RHElementActorHuman::Hourglass
        // (and therefore the observer's own movement) before RefreshView,
        // while actors with a later creation order have not run yet.
        let positions_before_movement = {
            let _detail = entity_system_detail_guard(EntitySystemDetail::BoundarySnapshot);
            let mut positions = EntitySlots::filled(self.world.entities.len(), None);
            for (entity_id, entity) in self.world.entities.occupied() {
                positions[entity_id] =
                    Some(crate::entities::BoundaryPosition::of(entity.element_data()));
            }
            positions
        };

        // ── Per-frame movement tick ─────────────────────────────
        // Actor movement runs later, inside the live legacy-slot owner walk.

        // `quit_swordfight_with_far_opponents` is called ONLY during
        // walking-with-sword movement, NOT for stationary entities.
        // Only check entities actively moving in sword state.
        // Owned by the selected sword-movement Execute arm in
        // `tick_entity_movement_owner`.

        // ── PC sword-walk pinch abort ───────────────────────────
        // During `WalkingWithSword` / `RunningWithSword`, after the
        // per-frame sprite motion the PC checks whether two opponents
        // are pinching its forward corridor and, if so, marks the
        // current sequence element `Impossible`.  Runs only on PCs in
        // sword movement with an active movement element and an
        // in-flight position delta (`is_moving_map()`).
        // `element_impossible` itself silently no-ops when the
        // element is `NonInterruptable`, which is the desired
        // behaviour.
        // Owned by the selected PC sword-movement Execute arm too.

        // ── Dispatch EventReachPoint to NPCs that just finished walking ──
        // Fires `Think(EVENT_REACHPOINT)` when a MOVE sequence
        // element terminates.

        // Separate Rust reconciliation boundary: the cited Original actor
        // Execute arms do not establish zone occupancy as owner-local work.
        // Fires EnterZone/ExitZone on zone scripts when occupancy changes.

        // ── Per-frame animation tick ────────────────────────────
        // Advance sprite animations for idle actors, FX, and other entities.
        // Supported moving actors are animated inside their live owner Execute arm.
        // Line-jump step advance runs inside each actor's own owner
        // envelope below, not as a batch ahead of the walk.

        // Every supported nonactor virtual Hourglass now runs below at its
        // live legacy slot: mobile boundary first, then static owners, then
        // projectile/net dispatch.
        self.tick_actor_owner_envelopes_with_display(
            sim,
            display,
            assets,
            &positions_before_movement,
        );
        // ── Corpse-intersection repulsion hook ────────────────────
        // Scan for lying↔non-lying posture transitions and fire
        // `update_intersecting_corpses` so stacked corpses get the
        // smaller repulsive radius and don't shove each other out
        // of their hitboxes.  Runs after animations have had a
        // chance to change postures this frame and before the next
        // frame's movement (which reads `small_repulsive_radius`
        // via `compute_repulsive_force`).
        {
            let _detail = entity_system_detail_guard(EntitySystemDetail::CorpseUpdates);
            self.process_corpse_intersection_updates();
        }

        // ── Per-frame animation sound dispatch ──────────────────
        // Now that every sprite has advanced (both movement-driven
        // and idle/one-shot animations), check each entity's current
        // sprite frame for an attached sound ID and queue it as an
        // FX (the `current_sound_id()` block every element type
        // runs during refresh / execute).
        {
            let _detail = entity_system_detail_guard(EntitySystemDetail::FrameSounds);
            self.dispatch_frame_sounds();
        }

        // TODO(original-parity): the followed-target position oracle below
        // proves one movement/NPC-refresh interleaving, but the rest of this
        // system-oriented pass still lacks per-entity dispatch boundaries.
        // Keep those responsibilities batched until each consumer has the
        // mixed pre/post inputs required at an individual creation slot.

        finish_entity_system_detail_frame();
        positions_before_movement
    }

    /// Execute an `RHElementMobile` at its first `RHElementFXMasked` child's
    /// creation slot, then execute that one child. Later child slots animate
    /// only themselves and therefore cannot retrigger the master.
    fn tick_mobile_child_owner_boundary(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        child_id: EntityId,
    ) -> bool {
        let Some(mobile_index_u16) = self
            .world
            .entities
            .get(child_id)
            .and_then(crate::element::Entity::as_fx)
            .and_then(|fx| fx.fx.mobile_index)
        else {
            return false;
        };
        let mobile_index = usize::from(mobile_index_u16);
        let (first_child, child_offset) = {
            let mobile = self
                .world
                .mobile_elements
                .get(mobile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "mobile child {child_id} at its Hourglass slot references missing master index {mobile_index}"
                    )
                });
            let first_child = *mobile.sprite_ids.first().unwrap_or_else(|| {
                panic!("mobile {mobile_index} has no first masked child for its owner boundary")
            });
            let child_offset = mobile
                .sprite_ids
                .iter()
                .position(|&candidate| candidate == child_id)
                .unwrap_or_else(|| {
                    panic!(
                        "FXMasked child {child_id} claims mobile {mobile_index}, but the master does not own it"
                    )
                });
            (first_child, child_offset)
        };

        if child_id == first_child {
            let first_slot = child_id.index();
            let sprite_ids = self.world.mobile_elements[mobile_index].sprite_ids.clone();
            for (offset, &expected_child) in sprite_ids.iter().enumerate() {
                let slot = first_slot.checked_add(offset as u32).unwrap_or_else(|| {
                    panic!(
                        "mobile {mobile_index} child adjacency overflows after slot {first_slot}"
                    )
                });
                let actual_child = self.world.entities.id_at_legacy_slot(slot).unwrap_or_else(|| {
                    panic!(
                        "mobile {mobile_index} child {expected_child} is missing from required adjacent slot {slot}"
                    )
                });
                assert_eq!(
                    actual_child, expected_child,
                    "mobile {mobile_index} child {expected_child} expected at adjacent slot {slot}, found {actual_child}"
                );
                let actual_index = self
                    .world
                    .entities
                    .get(actual_child)
                    .and_then(crate::element::Entity::as_fx)
                    .unwrap_or_else(|| {
                        panic!(
                            "mobile {mobile_index} child {actual_child} at adjacent slot {slot} is missing or non-FX"
                        )
                    })
                    .fx
                    .mobile_index;
                assert_eq!(
                    actual_index,
                    Some(mobile_index_u16),
                    "mobile {mobile_index} child {actual_child} at adjacent slot {slot} has wrong master index {actual_index:?}"
                );
            }

            let path_index = self.world.mobile_elements[mobile_index].path_index;
            let path = assets
                .hiking_paths
                .get(usize::from(path_index))
                .unwrap_or_else(|| panic!("mobile {mobile_index} lost hiking path {path_index}"));
            if let Some(motion) = self.world.mobile_elements[mobile_index].begin_hourglass_motion()
            {
                let movement_animation_speed =
                    self.world.mobile_elements[mobile_index].animation_speed();
                // Original `Update` translates every masked child before
                // `CheckForLineCrossing` and before the goal/waypoint arm. Its
                // adaptive-speed branch also fixes this frame's child
                // modulation now; a reached waypoint speed macro applies to
                // the master immediately but not to child animation until the
                // next Update.
                for &sprite_id in &sprite_ids {
                    let fx = self
                        .world
                        .entities
                        .get_mut(sprite_id)
                        .and_then(crate::element::Entity::as_fx_mut)
                        .unwrap_or_else(|| {
                            panic!("mobile {mobile_index} child {sprite_id} became stale during master motion")
                        });
                    if motion.movement != crate::coordinates::MapVec::ZERO {
                        fx.element
                            .set_position_map(fx.element.position_map() + motion.movement);
                    }
                    fx.fx.animation_speed = movement_animation_speed;
                }

                // This deliberately precedes waypoint execution. Projection
                // fallback probes with the increment that produced this move,
                // not a direction selected by the newly reached waypoint.
                self.check_mobile_line_crossing(assets, mobile_index);
                self.world.mobile_elements[mobile_index]
                    .finish_hourglass_waypoint(sim, path, motion.reached_goal)
                    .unwrap_or_else(|error| {
                        panic!(
                            "mobile {mobile_index} waypoint Hourglass at child {child_id} failed: {error}"
                        )
                    });

                let mobile = &self.world.mobile_elements[mobile_index];
                let active = mobile.active;
                let layer = mobile.layer;
                let sector = mobile.sector;
                for sprite_id in sprite_ids {
                    let fx = self
                        .world
                        .entities
                        .get_mut(sprite_id)
                        .and_then(crate::element::Entity::as_fx_mut)
                        .unwrap_or_else(|| {
                            panic!("mobile {mobile_index} child {sprite_id} became stale during waypoint completion")
                    });
                    fx.element.active = active;
                    fx.element.set_layer(layer);
                    fx.element
                        .set_sector(crate::position_interface::SectorHandle::new(sector));
                }
            }
        } else {
            assert!(
                child_offset > 0,
                "mobile {mobile_index} first-child boundary bookkeeping failed for {child_id}"
            );
        }

        let stopped = self.world.mobile_elements[mobile_index].stopped;
        let frozen = self.actors_frozen();
        let fx = self
            .world
            .entities
            .get_mut(child_id)
            .and_then(crate::element::Entity::as_fx_mut)
            .unwrap_or_else(|| {
                panic!("mobile {mobile_index} child {child_id} vanished before FXMasked Hourglass")
            });
        if fx.element.active && !stopped && !frozen {
            fx.element
                .sprite
                .increment_frame_modulated(fx.fx.animation_speed);
        }
        true
    }

    pub(super) fn tick_static_entity_hourglass_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        use crate::element::OriginalBonusConcreteClass;
        use crate::sprite::{FrameProgression, MotionState};

        let frozen = self.actors_frozen();
        let entity = self.world.entities.get(owner).unwrap_or_else(|| {
            panic!(
                "static Hourglass owner {owner:?} disappeared immediately after live legacy-slot resolution"
            )
        });
        match entity {
            Entity::Fx(fx) if fx.fx.mobile_index.is_some() => return,
            Entity::Fx(_) => {
                if !entity.is_active() || frozen {
                    return;
                }
                let patch_idx = entity.as_fx().and_then(|fx| fx.fx.patch_index);
                let (progression, in_transition) = if let Some(patch_idx) = patch_idx {
                    if self.scripts.mission.is_none() {
                        (FrameProgression::Default, false)
                    } else {
                        let patch = self
                            .script_domains
                            .interactables
                            .patches
                            .get(usize::from(patch_idx))
                            .unwrap_or_else(|| panic!("FX {owner:?} references missing patch {patch_idx} at its live Hourglass slot"));
                        (
                            if patch.applied && patch.in_transition {
                                FrameProgression::Reversed
                            } else {
                                FrameProgression::Default
                            },
                            patch.in_transition,
                        )
                    }
                } else {
                    (FrameProgression::Default, false)
                };
                let motion = self
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap_or_else(|| panic!("FX {owner:?} vanished before sprite Hourglass"))
                    .element_data_mut()
                    .sprite
                    .perform_virgin_increment(sim, progression);
                if matches!(motion, MotionState::Terminated) && in_transition {
                    self.finish_patch_transition_for(
                        sim,
                        assets,
                        patch_idx.expect("transitioning FX must retain its patch"),
                    );
                }
            }
            Entity::Target(target) => {
                let active = target.element.active;
                let progression = FrameProgression::from_ordinal(target.target.progression);
                if active && !frozen {
                    self.world
                        .entities
                        .get_mut(owner)
                        .unwrap()
                        .element_data_mut()
                        .sprite
                        .perform_virgin_increment(sim, progression);
                }
            }
            Entity::Scroll(scroll) => {
                if !scroll.element.active {
                    return;
                }
                self.dispatch_scroll_hourglass_for(sim, assets, owner);
                // RHSprite samples the engine FreezeAll state after the
                // synchronous Scroll VM returns, not at Hourglass entry.
                if !self.actors_frozen()
                    && let Some(entity) = self.world.entities.get_mut(owner)
                {
                    let Entity::Scroll(scroll) = entity else {
                        panic!(
                            "scroll {owner:?} changed concrete type before entry-active sprite Hourglass"
                        )
                    };
                    // Original tests IsActive only once on entry. A due VM
                    // callback may deactivate this surviving scroll, but its
                    // sprite still advances before this Hourglass returns.
                    scroll
                        .element
                        .sprite
                        .perform_virgin_increment(sim, FrameProgression::Default);
                }
            }
            Entity::Bonus(bonus) => match bonus.original_concrete_class() {
                OriginalBonusConcreteClass::Bonus => {
                    if !frozen {
                        self.world
                            .entities
                            .get_mut(owner)
                            .unwrap()
                            .element_data_mut()
                            .sprite
                            .perform_virgin_increment(sim, FrameProgression::Default);
                    }
                    self.refresh_bonus_discovered_for(assets, owner);
                }
                // RHElementAle::Hourglass returns false once inactive, but
                // RHEngine calls RemoveElement with its default
                // bOnlyDeactivate=true. The pointer stays in marrayElements
                // because other elements may still reference it.
                OriginalBonusConcreteClass::Ale => {}
                OriginalBonusConcreteClass::Cape => {
                    if !frozen {
                        self.world
                            .entities
                            .get_mut(owner)
                            .unwrap()
                            .element_data_mut()
                            .sprite
                            .perform_virgin_increment(sim, FrameProgression::Default);
                    }
                }
                OriginalBonusConcreteClass::Unsupported => panic!(
                    "Entity::Bonus {owner:?} has unsupported Original concrete-class mapping for {:?}",
                    bonus.object.object_type
                ),
            },
            _ => {}
        }
    }

    /// Run the bounded base-actor Hourglass slice in live Original element
    /// order: generic animation/Execute, synchronous combat-injury Think,
    /// completion/priority effects, then `ActionChange`.
    ///
    /// `RHEngine::SerializeElements` sorts `marrayElements` by
    /// `mulCreationOrder` before writing a save, and the loaded compact array
    /// retains that order. Rust entity IDs keep the initialized mission's
    /// stable sparse slots, so their numeric order is not the loaded
    /// `marrayElements` order. Walk the authoritative Original creation
    /// identities instead. The local vector is compacted after every callback
    /// and newly constructed elements are appended, preserving the Original
    /// loop's observable mutation behavior.
    ///
    /// Generic animation eligibility does not gate `ActionChange`; inactive,
    /// frozen, moving, active-shot, and active-melee actors still reach the
    /// callback boundary.
    #[cfg(test)]
    pub(super) fn tick_actor_animation_action_change_slots(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_actor_animation_action_change_slots_with_hooks(
            sim,
            assets,
            |_, _| {},
            |_, _| {},
            |_, _, _, _, _, _, _| {},
            |_, _, _| {},
        );
    }

    #[cfg(test)]
    pub(super) fn tick_actor_animation_action_change_slots_with_after_slot(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        mut after_slot: impl FnMut(&mut Self, EntityId),
    ) {
        self.tick_actor_animation_action_change_slots_with_hooks(
            sim,
            assets,
            |_, _| {},
            |_, _| {},
            |_, _, _, _, _, _, _| {},
            |engine, owner, _| after_slot(engine, owner),
        );
    }

    pub(super) fn tick_actor_animation_action_change_slots_with_hooks<ExecuteMotion>(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        mut non_actor_slot: impl FnMut(&mut Self, EntityId),
        mut before_actor: impl FnMut(&mut Self, EntityId),
        mut execute_owner_arm: impl FnMut(
            &mut Self,
            EntityId,
            Option<super::movement::MovementOwnerSelection>,
            Option<MeleeOwnerSelection>,
            Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)>,
            Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)>,
            Option<std::num::NonZeroU32>,
        ) -> ExecuteMotion,
        mut after_slot: impl FnMut(&mut Self, EntityId, crate::order::OrderType),
    ) where
        ExecuteMotion: IntoExplicitExecuteMotion,
    {
        let mut original_slots = self
            .world
            .entities
            .occupied()
            .map(|(entity_id, _)| entity_id)
            .collect::<Vec<_>>();
        original_slots.sort_by_key(|&entity_id| self.world.original_creation_order(entity_id));
        let mut observed_creation_counter = self.world.next_original_creation_order;
        let mut slot = 0;
        while slot < original_slots.len() {
            let entity_id = original_slots[slot];
            if self.world.entities.get(entity_id).is_some() {
                observe_ordered_gameplay_entity(entity_id);
                let entity = self
                    .world
                    .entities
                    .get(entity_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "actor animation coordinator lost entity {entity_id:?} resolved from Original element slot {slot}"
                        )
                    });
                let actor_enters_hourglass = entity.actor_data().is_some()
                    && !matches!(entity, Entity::Pc(pc) if pc.pc.fried_psykokwack);
                if actor_enters_hourglass {
                    'actor_hourglass: {
                        // Detach work that predates this actor slot. Lazy Wait
                        // initialization and completion callbacks below may drain
                        // only work they synchronously create; they must not steal
                        // a global/later-owner continuation.
                        let preexisting_sequence_work = self
                            .orders
                            .sequence_manager
                            .take_pending_synchronous_actions();

                        // RHElementActor::Hourglass consumes one queued base
                        // position update before it inspects the current
                        // sequence/order.
                        self.apply_delayed_actor_position(sim, assets, entity_id);
                        self.debug_patrol_turn_lifecycle("actor_slot_before_prelude", entity_id);
                        before_actor(self, entity_id);
                        self.debug_patrol_turn_lifecycle("actor_slot_after_prelude", entity_id);
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::BaseActor(entity_id));

                        let frozen_without_order = self
                            .world
                            .entities
                            .get(entity_id)
                            .and_then(Entity::actor_data)
                            .is_some_and(|actor| actor.execution_frozen)
                            && self
                                .orders
                                .sequence_manager
                                .current_order_for_actor(entity_id)
                                .is_none();
                        if frozen_without_order {
                            // Actor::Hourglass refreshes mpOrder after applying
                            // the delayed position. With no selected order it
                            // clears the pointer, then an execution freeze returns
                            // before lazy Wait and the second NewMove snapshot.
                            self.world
                                .entities
                                .get_mut(entity_id)
                                .and_then(Entity::actor_data_mut)
                                .expect("frozen actor disappeared before mpOrder clear")
                                .installed_order = None;
                            self.debug_refresh_view_lifecycle(
                                "derived_tail_frozen_without_order",
                                entity_id,
                                Some(crate::order::OrderType::NonanimationEnd),
                            );
                            after_slot(self, entity_id, crate::order::OrderType::NonanimationEnd);
                            let leaked_slot_work = self
                                .orders
                                .sequence_manager
                                .take_pending_synchronous_actions();
                            assert!(
                                leaked_slot_work.is_empty(),
                                "frozen actor {entity_id:?} leaked synchronous sequence work after its derived Hourglass tail: {leaked_slot_work:?}"
                            );
                            self.orders
                                .sequence_manager
                                .restore_pending_synchronous_actions(preexisting_sequence_work);
                            break 'actor_hourglass;
                        }

                        // `RHEngine::Hourglass` calls every element's virtual
                        // Hourglass regardless of `IsActive()`. Actor::Hourglass
                        // then installs Wait whenever its order is empty. Active
                        // controls world presence/rendering, not sequence time.
                        self.ensure_wait_element(entity_id);
                        // Original Wait -> LaunchSequenceElement ->
                        // Sequence::Launch -> SequenceElement::Go -> Instruct is
                        // synchronous. A command registered for later manager or
                        // deferred processing cannot suppress this transient
                        // Execute: Wait may publish its START sprite row before
                        // that later command interrupts it in the same frame.
                        // Preexisting Rust work is detached above, so this drain
                        // consumes only the newly launched Wait.
                        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                        .unwrap_or_else(|error| {
                            panic!(
                                "actor {entity_id:?} Wait initialization at legacy slot {slot} failed to drain its synchronous sequence work: {error:?}"
                            )
                        });
                        observe_actor_animation_boundary(ActorAnimationBoundaryPhase::WaitReady(
                            entity_id,
                        ));

                        // RHElementActor::Hourglass calls NewMove after lazy Wait
                        // installation and immediately before it samples the
                        // current order ID and enters Execute. The delayed-position
                        // branches perform an earlier NewMove for their crossing
                        // segment, then reach this second snapshot as well. Keep
                        // PositionInterface's old-position latch frame-local;
                        // movement and combat helpers use IsMoving[Map] later in
                        // this same owner slot.
                        self.world
                            .entities
                            .get_mut(entity_id)
                            .expect("actor disappeared before Hourglass NewMove")
                            .position_iface_mut()
                            .new_move();

                        let selected_order = self
                            .orders
                            .sequence_manager
                            .current_order_for_actor(entity_id)
                            .map(|(seq_id, elem_idx, order)| (seq_id, elem_idx, order.order_id));
                        let selected_order_type = self
                            .orders
                            .sequence_manager
                            .current_order_for_actor(entity_id)
                            .map(|(_, _, order)| order.order_type);
                        let selected_order_compute_direction = self
                            .orders
                            .sequence_manager
                            .current_order_for_actor(entity_id)
                            .map(|(_, _, order)| order.compute_direction);
                        // Actor::Hourglass refreshes mpOrder from the selected
                        // element immediately before Execute. Preserve that
                        // pointer publication independently of manager selection:
                        // later DoNextOrder or Instruct calls update the explicit
                        // mirror at their own boundaries.
                        let installed_at_entry = self
                            .orders
                            .sequence_manager
                            .current_order_for_actor(entity_id)
                            .map(|(_, _, order)| crate::element::InstalledActorOrder {
                                order_id: order.order_id,
                                order_type: order.order_type,
                            });
                        self.world
                            .entities
                            .get_mut(entity_id)
                            .and_then(Entity::actor_data_mut)
                            .expect("actor disappeared before installing its Hourglass order")
                            .installed_order = installed_at_entry;
                        let selected_owner_family = self
                            .orders
                            .sequence_manager
                            .current_order_for_actor(entity_id)
                            .and_then(|(_, _, order)| {
                                classify_live_actor_execute_arm(entity_id, order.order_type)
                            });
                        if let Some((_, _, order_id)) = selected_order {
                            let actor = self
                                .world
                                .entities
                                .get_mut(entity_id)
                                .and_then(Entity::actor_data_mut)
                                .unwrap_or_else(|| {
                                    panic!("selected Execute owner {entity_id:?} lost actor data")
                                });
                            actor.execute_order_initialising =
                                actor.last_execute_order_id != Some(order_id);
                            actor.last_execute_order_id = Some(order_id);
                        }
                        self.debug_drop_owner_boundary(
                            "execute_latch_published",
                            entity_id,
                            selected_order,
                        );
                        // PC::Execute handles the carrying-corpse exit for an
                        // ENTER_SWORDFIGHT before the default validity arm: on
                        // the transition's first Execute it drops immediately
                        // and returns TERMINATED (RHelementactorpc.cpp:
                        // 4905-4917). Translation still has to register the
                        // transition, so key this to the entry-latched order
                        // rather than consuming it during GenerateTransition.
                        let enter_swordfight_corpse_exit = selected_order_type
                            == Some(
                                crate::order::OrderType::TransitionCarryingCorpseWaitingUpright,
                            )
                            && self.world.entities.get(entity_id).is_some_and(|entity| {
                                entity.is_pc()
                                    && entity.actor_data().is_some_and(|actor| {
                                        actor.execute_order_initialising && !actor.execution_frozen
                                    })
                            })
                            && selected_order.is_some_and(|(seq_id, elem_idx, _)| {
                                self.orders
                                    .sequence_manager
                                    .get_element(seq_id, elem_idx)
                                    .is_some_and(|element| {
                                        element.command == crate::element::Command::EnterSwordfight
                                    })
                            });
                        // Human/PC validity belongs to the live Execute entry,
                        // after Actor::Hourglass has established mbNewOrder for
                        // this exact selected order. Earlier actor callbacks may
                        // replace the selected order in the same owner walk, so
                        // sampling in a global pre-pass would validate stale work.
                        let validity_short_circuited = !enter_swordfight_corpse_exit
                            && self.pre_tick_human_execute_validity_for(assets, entity_id);
                        if !validity_short_circuited
                            && !enter_swordfight_corpse_exit
                            && selected_order_type
                                == Some(
                                    crate::order::OrderType::TransitionCarryingCorpseWaitingUpright,
                                )
                            && self
                                .world
                                .entities
                                .get(entity_id)
                                .and_then(Entity::actor_data)
                                .is_some_and(|actor| actor.execute_order_initialising)
                        {
                            // RHElementActorPC::Execute owns this initialization,
                            // not the DROP_CORPSE command builder. Posture
                            // transitions inserted for another PC command enter
                            // the same animation arm without an ActiveAbility.
                            // Original aligns mpCarried after validity and before
                            // PerformAction (RHelementactorpc.cpp:4905-4955).
                            let (carried, carried_direction) = {
                                let carrier = self.world.entities.get(entity_id).unwrap_or_else(|| {
                                    panic!(
                                        "corpse-exit transition owner {entity_id:?} vanished before initialization"
                                    )
                                });
                                let carried = carrier
                                    .pc_data()
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "corpse-exit transition owner {entity_id:?} is not a PC"
                                        )
                                    })
                                    .carried
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "corpse-exit transition owner {entity_id:?} has no carried body"
                                        )
                                    });
                                (
                                    carried,
                                    carrier.element_data().direction().wrapping_sub(4) & 15,
                                )
                            };
                            self.world
                                .entities
                                .get_mut(carried)
                                .unwrap_or_else(|| {
                                    panic!(
                                        "corpse-exit transition target {carried:?} vanished before initialization"
                                    )
                                })
                                .element_data_mut()
                                .set_direction_instantly(carried_direction);
                        }
                        let movement_selection = (!validity_short_circuited)
                            .then_some(selected_order)
                            .flatten()
                            .and_then(|(seq_id, elem_idx, order_id)| {
                                self.orders
                                    .sequence_manager
                                    .get_element(seq_id, elem_idx)
                                    .filter(|element| {
                                        selected_owner_family == Some(ExecuteOwnerFamily::Movement)
                                            && element.data.is_movement()
                                            && !matches!(
                                                element.command,
                                                crate::element::Command::WaitTimer
                                                    | crate::element::Command::WaitFreeLift
                                            )
                                    })
                                    .map(|_| super::movement::MovementOwnerSelection {
                                        seq_id,
                                        elem_idx,
                                        order_id,
                                    })
                            });
                        let movement_entity_target_seek =
                            movement_selection.is_some_and(|selection| {
                                self.orders
                                    .sequence_manager
                                    .get_element(selection.seq_id, selection.elem_idx)
                                    .is_some_and(|element| {
                                        let crate::sequence::SequenceElementData::Movement {
                                            element: target,
                                            flags,
                                            ..
                                        } = &element.data
                                        else {
                                            return false;
                                        };
                                        // The seek wrapper is chosen per animation
                                        // arm, not per element: wall and ladder
                                        // orders keep the SEEK flag while their
                                        // Execute arms drive the sprite directly
                                        // and hand the raw START edge back.
                                        flags.contains(crate::sequence::MoveFlags::SEEK)
                                            && target.is_some()
                                            && element.current_order().is_some_and(|order| {
                                                super::movement::perform_seek_calls_per_execute(
                                                    order.order_type,
                                                ) > 0
                                            })
                                    })
                            });
                        let melee_selection = (!validity_short_circuited)
                            .then_some(selected_order)
                            .flatten()
                            .and_then(|(seq_id, elem_idx, order_id)| {
                                let order_type = self
                                    .orders
                                    .sequence_manager
                                    .get_element(seq_id, elem_idx)
                                    .and_then(|element| element.current_order())
                                    .map(|order| order.order_type)?;
                                (selected_owner_family == Some(ExecuteOwnerFamily::Melee)
                                    && MELEE_ORDERS.contains(&order_type))
                                .then_some(MeleeOwnerSelection {
                                    seq_id,
                                    elem_idx,
                                    order_id,
                                })
                            });
                        // Bow belongs to the same entry-latched Execute choice as
                        // movement and melee. If its terminal callback exposes a
                        // successor order, that successor must wait until the
                        // actor's next Hourglass rather than entering generic
                        // Execute later in this same slot.
                        let bow_selection = (!validity_short_circuited
                            && selected_owner_family == Some(ExecuteOwnerFamily::Bow))
                        .then(|| self.selected_bow_order(entity_id))
                        .flatten();
                        let ability_selection = selected_order.filter(|(seq, elem, order_id)| {
                            !validity_short_circuited
                                && selected_owner_family == Some(ExecuteOwnerFamily::Ability)
                                && self
                                    .world
                                    .entities
                                    .get(entity_id)
                                    .and_then(Entity::actor_data)
                                    .is_some_and(|actor| {
                                        let Some(expected_type) = active_ability_order_type(actor)
                                        else {
                                            return false;
                                        };
                                        actor.active_ability.is_active()
                                            && actor.active_ability.sequence_id == Some(*seq)
                                            && actor.active_ability.element_index == *elem
                                            && actor.active_ability.order_id == Some(*order_id)
                                            && self
                                                .orders
                                                .sequence_manager
                                                .get_element(*seq, *elem)
                                                .and_then(|element| element.current_order())
                                                .is_some_and(|order| {
                                                    order.order_type == expected_type
                                                })
                                    })
                        });
                        let beggar_selection = selected_order.and_then(|(seq, elem, order_id)| {
                            if validity_short_circuited
                                || selected_owner_family != Some(ExecuteOwnerFamily::Beggar)
                            {
                                return None;
                            }
                            self.orders
                                .sequence_manager
                                .get_element(seq, elem)
                                .and_then(|element| element.current_order())
                                .and_then(|order| {
                                    (order.order_id == order_id
                                        && order.order_type
                                            == crate::order::OrderType::SimulatingBeggar)
                                        .then_some(order_id)
                                })
                        });
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::MovementExecute(
                            entity_id,
                        ));
                        self.debug_drop_owner_boundary(
                            "tick_ability_entry",
                            entity_id,
                            selected_order,
                        );
                        if let Some(entity) = self.world.entities.get(entity_id) {
                            super::animation::direction_provenance_snapshot(
                                entity.position_iface(),
                                entity_id,
                                self.control.frame_counter,
                                "owner_execute_entry",
                            );
                        }
                        let explicit_execute = execute_owner_arm(
                            self,
                            entity_id,
                            movement_selection,
                            melee_selection,
                            bow_selection,
                            ability_selection,
                            beggar_selection,
                        )
                        .into_explicit_execute_motion();
                        let explicit_execute_motion = explicit_execute.initial;
                        let post_completion_execute_override =
                            explicit_execute.post_completion_override;
                        if let Some(entity) = self.world.entities.get(entity_id) {
                            super::animation::direction_provenance_snapshot(
                                entity.position_iface(),
                                entity_id,
                                self.control.frame_counter,
                                "owner_post_execute",
                            );
                        }
                        let mut specialized_execute_motion =
                            explicit_execute_motion.or_else(|| {
                                (!validity_short_circuited)
                                    .then_some(selected_owner_family)
                                    .flatten()
                                    .filter(|family| {
                                        *family != ExecuteOwnerFamily::GenericAnimation
                                    })
                                    .and_then(|_| {
                                        specialized_execute_motion(
                                            self.world.entities.get(entity_id).and_then(|entity| {
                                                entity.element_data().sprite.last_motion_state
                                            }),
                                            beggar_selection.is_some(),
                                            movement_entity_target_seek,
                                        )
                                    })
                            });
                        let mut specialized_wait_modifier_terminated = false;
                        if let (Some(motion), Some((entry_seq_id, entry_elem_idx, _))) =
                            (specialized_execute_motion.as_mut(), selected_order)
                        {
                            // Actor::Hourglass applies WAIT_TIMER / WAIT_FREE_LIFT
                            // after the complete virtual Execute call. Derived
                            // movement, combat, ability, and beggar arms therefore
                            // pass through the same base modifier as generic sprite
                            // animation, exactly once.
                            let motion_before_modifier = *motion;
                            self.apply_actor_post_execute_wait_modifier_to_motion(
                                entity_id,
                                entry_seq_id,
                                entry_elem_idx,
                                motion,
                            );
                            specialized_wait_modifier_terminated = motion_before_modifier
                                != crate::sprite::MotionState::Terminated
                                && *motion == crate::sprite::MotionState::Terminated;
                        }
                        let explicit_execute_in_progress = matches!(
                            explicit_execute_motion,
                            Some(crate::sprite::MotionState::InProgress)
                        );
                        let explicit_execute_terminated = matches!(
                            explicit_execute_motion,
                            Some(crate::sprite::MotionState::Terminated)
                        );
                        if let Some(motion) = specialized_execute_motion {
                            // Movement/combat/ability owners are derived Execute
                            // arms just like the generic animation switch below.
                            // Their sprite result is therefore the initial value
                            // assigned to Actor::mmotionState before Hourglass
                            // applies Done/DoNextOrder handling.
                            self.world
                            .entities
                            .get_mut(entity_id)
                            .and_then(Entity::actor_data_mut)
                            .expect(
                                "specialized Execute owner disappeared before motion-state latch",
                            )
                            .continuation
                            .motion_state = motion;
                        }
                        if !validity_short_circuited
                            && explicit_execute_motion.is_none()
                            // Generic animation owns its completion through
                            // `tick_actor_animation_for` below.  In
                            // particular TURNING deliberately ignores the
                            // visual sprite's Done edge while `Turn()` still
                            // reports that the body rotated this frame.  A
                            // stale Done retained by the looping alerted-turn
                            // sprite must therefore not complete the order
                            // ahead of that authoritative Execute result.
                            && selected_owner_family.is_some_and(|family| {
                                family != ExecuteOwnerFamily::GenericAnimation
                            })
                            && self.world.entities.get(entity_id).is_some_and(|entity| {
                                entity.element_data().sprite.last_motion_state
                                    == Some(crate::sprite::MotionState::Done)
                            })
                        {
                            let (entry_seq_id, entry_elem_idx, entry_order_id) =
                            selected_order.unwrap_or_else(|| {
                                panic!(
                                    "specialized actor owner {entity_id:?} recorded Done without an entry-latched order"
                                )
                            });
                            self.mark_entry_order_done(
                                entity_id,
                                entry_seq_id,
                                entry_elem_idx,
                                entry_order_id,
                            );
                        }

                        observe_actor_animation_boundary(
                            ActorAnimationBoundaryPhase::GenericExecute(entity_id),
                        );
                        let (combat_injury_terminated, mut outcomes, mut execute_result) =
                            if validity_short_circuited
                                || movement_selection.is_some()
                                || melee_selection.is_some()
                                || bow_selection.is_some()
                                || ability_selection.is_some()
                                || beggar_selection.is_some()
                            {
                                (Vec::new(), Default::default(), None)
                            } else if enter_swordfight_corpse_exit {
                                let (seq_id, elem_idx, _) = selected_order.unwrap_or_else(|| {
                                    panic!(
                                        "ENTER_SWORDFIGHT corpse-exit Execute lost its entry order"
                                    )
                                });
                                self.world
                                    .entities
                                    .get(entity_id)
                                    .and_then(Entity::pc_data)
                                    .and_then(|pc| pc.carried)
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "ENTER_SWORDFIGHT corpse-exit Execute owner {entity_id:?} has no carried body"
                                        )
                                    });
                                self.force_drop_carried_corpse_instant(entity_id);
                                (
                                    Vec::new(),
                                    Default::default(),
                                    Some(super::animation::ActorExecuteResult {
                                        order_type: crate::order::OrderType::TransitionCarryingCorpseWaitingUpright,
                                        entry_seq_id: seq_id,
                                        entry_elem_idx: elem_idx,
                                        motion: crate::sprite::MotionState::Terminated,
                                    }),
                                )
                            } else if selected_order_type == Some(crate::order::OrderType::Rolling)
                            {
                                self.tick_rolling_owner(sim, assets, entity_id)
                            } else {
                                self.tick_actor_animation_for(sim, assets, entity_id)
                            };
                        if specialized_wait_modifier_terminated {
                            let (entry_seq_id, entry_elem_idx, entry_order_id) = selected_order
                                .expect("specialized wait modifier lost its entry order");
                            self.stage_actor_execute_completion(
                                entity_id,
                                Some(entry_order_id),
                                super::animation::ActorExecuteResult {
                                    order_type: selected_order_type.expect(
                                        "specialized wait modifier lost its entry order type",
                                    ),
                                    entry_seq_id,
                                    entry_elem_idx,
                                    motion: crate::sprite::MotionState::Terminated,
                                },
                                &mut outcomes,
                            );
                        }
                        if explicit_execute_terminated {
                            let (seq_id, elem_idx, _) = selected_order.unwrap_or_else(|| {
                                panic!(
                                    "actor {entity_id:?} returned explicit Terminated without an entry-latched order"
                                )
                            });
                            outcomes.seq_advance.push((seq_id, elem_idx));
                        }
                        // Falling-hit/pushed/lift flight is part of this
                        // actor's selected Execute arm in Original. Advance it
                        // before the derived NPC tail so later creation slots
                        // observe the committed flight position.
                        let flight_motion = self.tick_push_flight_for_owner(sim, assets, entity_id);
                        if let (Some(result), Some(motion)) =
                            (execute_result.as_mut(), flight_motion)
                        {
                            // FallingLadderWall returns Terminated directly
                            // from Execute when its countdown reaches zero.
                            // The split flight tail owns that terminal edge,
                            // so replace the earlier sprite Start result before
                            // Actor::Hourglass latches it.
                            result.motion = motion;
                        }
                        if execute_result.as_ref().is_some_and(|result| {
                            result.motion == crate::sprite::MotionState::Start
                        }) && self
                            .world
                            .entities
                            .get(entity_id)
                            .is_some_and(Entity::is_pc)
                        {
                            // RHElementActorPC::Execute owns eventual strike /
                            // execution remarks. Their 50% RNG draw and speech
                            // side effects occur synchronously before the next
                            // element's Hourglass slot.
                            self.tick_pc_combat_anim_speech_for_owner(sim, assets, entity_id);
                        }
                        // Original clears mbSequenceElementStarted immediately
                        // after Execute returns. It means "the selected element
                        // has not had its first owner slot yet", not "this
                        // element has ever started". In particular, a Move issued
                        // while an already-running non-interruptable PassDoor is
                        // postponed; only a PassDoor newly installed since the
                        // actor's last slot rejects that Move as impossible.
                        if let Some(actor) = self
                            .world
                            .entities
                            .get_mut(entity_id)
                            .and_then(Entity::actor_data_mut)
                        {
                            actor.sequence_element_started = false;
                        }
                        for injured_id in combat_injury_terminated.iter().copied() {
                            self.dispatch_combat_injury_think_for_actor_hourglass(
                                sim, injured_id, assets,
                            );
                        }
                        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                        .unwrap_or_else(|error| {
                            panic!(
                                "actor {entity_id:?} combat-injury Think at legacy slot {slot} failed to drain synchronous sequence work: {error:?}"
                            )
                        });
                        for injured_id in combat_injury_terminated {
                            observe_actor_animation_boundary(
                                ActorAnimationBoundaryPhase::CombatInjuryThink(injured_id),
                            );
                        }

                        // RHElementActorHuman::Execute performs this work inside
                        // the WaitingSword arm, after PerformAction and before
                        // returning its motion result to Actor::Hourglass. Keep
                        // launches and cross-actor mutations live so later slots
                        // observe them and earlier slots do not.
                        // This is part of Human::Execute's selected
                        // WAITING_SWORD arm, not an animation-completion
                        // callback.  In particular, the arm still runs when
                        // the generic sprite helper has no completion record
                        // for this slot.  Key it to Actor::Hourglass's
                        // entry-latched order, as Original does, while keeping
                        // the two Execute entry exits above intact.
                        let execution_frozen = self
                            .world
                            .entities
                            .get(entity_id)
                            .and_then(Entity::actor_data)
                            .is_some_and(|actor| actor.execution_frozen);
                        if waiting_sword_execute_reaches_evaluation(
                            selected_order_type,
                            validity_short_circuited,
                            execution_frozen,
                        ) {
                            self.tick_waiting_sword_execute_for(sim, assets, entity_id);
                        }

                        // RHElementActorHuman::Execute decrements the parry hold
                        // counter and queues StopParry before this actor yields
                        // its legacy slot. Preserve that ordering relative to
                        // sword hits performed by later-created actors.
                        if let Some(result) = execute_result.as_mut() {
                            self.tick_parry_counter_for_execute(entity_id, result);
                        }

                        // Original Actor::Hourglass modifies the just-produced
                        // Execute result for WAIT_TIMER / WAIT_FREE_LIFT before
                        // completion/DoNextOrder. Sampling the current element
                        // here is intentional: WaitingSword callbacks above may
                        // have synchronously replaced it.
                        if let Some(result) = execute_result.as_mut() {
                            self.apply_actor_post_execute_wait_modifier(entity_id, result);
                        }
                        // Base Actor::Hourglass calls CheckForLineCrossing
                        // after the complete virtual Execute chain and its wait
                        // modifier, but before interpreting the motion result.
                        // Movement owners and Rolling close this boundary in
                        // their specialized executors; generic animation
                        // (including FindPlaceToDie and flight) reaches it here.
                        if selected_owner_family != Some(ExecuteOwnerFamily::Movement)
                            && selected_order_type != Some(crate::order::OrderType::Rolling)
                        {
                            self.dispatch_actor_post_execute_line_crossing(
                                sim,
                                assets,
                                entity_id,
                                selected_order_compute_direction,
                            );
                        }
                        if let Some(result) = execute_result.take() {
                            // RHElementActor::Hourglass stores every Execute
                            // return in serialized `mmotionState` before it
                            // handles Done/Terminated/Aborted. Keeping only the
                            // transient Sprite result leaves the save-loaded
                            // value frozen forever and makes the very first
                            // post-load frame diverge whenever an animation
                            // crosses a motion boundary.
                            self.world
                                .entities
                                .get_mut(entity_id)
                                .and_then(Entity::actor_data_mut)
                                .expect("Execute owner disappeared before motion-state latch")
                                .continuation
                                .motion_state = result.motion;
                            self.stage_actor_execute_completion(
                                entity_id,
                                selected_order.map(|(_, _, order_id)| order_id),
                                result,
                                &mut outcomes,
                            );
                        }

                        // Original soldier Execute calls Think before returning
                        // Terminated to the base Actor Hourglass. Only after that
                        // synchronous Think finishes may DoNextOrder/completion
                        // promote the actor's successor order.
                        self.process_anim_completion_outcomes(sim, outcomes, assets);
                        // `RHSequenceElement::SetState(TERMINATED)` calls the
                        // actor's virtual SendCondolationCard and then Ready()
                        // synchronously inside this Hourglass slot. Close only
                        // this owner's newly terminated stack before its derived
                        // NPC tail runs; leaving it in the global queue delays
                        // immediate successors such as UnlockAI until after
                        // detection and changes observable AI state.
                        self.dispatch_condolations_for_owner_boundary(sim, entity_id, assets);
                        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                        .unwrap_or_else(|error| {
                            panic!(
                                "actor {entity_id:?} completion at legacy slot {slot} failed to drain synchronous sequence work: {error:?}"
                            )
                        });
                        let selected_element_state =
                            selected_order.and_then(|(entry_seq, entry_idx, _)| {
                                self.orders
                                    .sequence_manager
                                    .get_element(entry_seq, entry_idx)
                                    .map(|element| element.state)
                            });
                        let selected_element_retired = selected_order.is_some()
                            && selected_element_state.is_none_or(|state| {
                                !matches!(
                                    state,
                                    crate::sequence::SequenceState::Todo
                                        | crate::sequence::SequenceState::InProgress
                                        | crate::sequence::SequenceState::Postponed
                                )
                            });
                        let selected_element_interrupted = selected_element_state
                            == Some(crate::sequence::SequenceState::Interrupted);
                        let selected_element_impossible =
                            selected_order.is_some_and(|(entry_seq, entry_idx, _)| {
                                self.orders
                                    .sequence_manager
                                    .get_element(entry_seq, entry_idx)
                                    .is_some_and(|element| {
                                        element.state == crate::sequence::SequenceState::Impossible
                                    })
                            });
                        let selected_order_rewritten_by_stop = specialized_execute_motion
                            .zip(selected_order_type)
                            .is_some_and(|(motion, entry_order_type)| {
                                selected_order.is_some_and(
                                    |(entry_seq, entry_idx, entry_order_id)| {
                                        self.orders
                                            .sequence_manager
                                            .current_order_for_actor(entity_id)
                                            .is_some_and(|(live_seq, live_idx, live_order)| {
                                                live_seq == entry_seq
                                                    && live_idx == entry_idx
                                                    && live_order.order_id != entry_order_id
                                                    && is_start_stop_movement_rewrite(
                                                        entry_order_id,
                                                        entry_order_type,
                                                        live_order.order_id,
                                                        live_order.order_type,
                                                        motion,
                                                    )
                                            })
                                    },
                                )
                            });
                        let selected_entry_order_still_current =
                            selected_order.is_some_and(|(entry_seq, entry_idx, entry_order)| {
                                self.orders
                                    .sequence_manager
                                    .current_order_for_actor(entity_id)
                                    .is_some_and(|(live_seq, live_idx, live_order)| {
                                        live_seq == entry_seq
                                            && live_idx == entry_idx
                                            && live_order.order_id == entry_order
                                    })
                            });
                        let selected_specialized_order_advanced = !explicit_execute_in_progress
                            && specialized_order_advanced_after_execute(
                                specialized_execute_motion,
                                selected_order_rewritten_by_stop,
                                selected_element_retired,
                                selected_element_interrupted,
                                selected_entry_order_still_current,
                            );
                        // Terminal sequence elements retain their allocated
                        // orders for diagnostics/save parity. Do not mistake
                        // that same retired entry order for a successor, while
                        // still accepting a distinct order installed by a
                        // synchronous condolence-card callback.
                        // DoNextOrder changes the retained Execute result to
                        // IN_PROGRESS only when Proceed returns a non-null
                        // mpOrder. Manager residency is not sufficient: queue
                        // exhaustion can terminate the selected element while
                        // leaving a fallback Wait discoverable in the manager,
                        // yet the actor's mpOrder remains null until its next
                        // Hourglass entry. `installed_order` is the explicit
                        // mirror updated by DoNextOrder and accepted Instruct.
                        let installed_successor_exists = self
                            .world
                            .entities
                            .get(entity_id)
                            .and_then(Entity::actor_data)
                            .and_then(|actor| actor.installed_order)
                            .is_some_and(|installed| {
                                !selected_order.is_some_and(|(_, _, entry_order)| {
                                    selected_element_retired && installed.order_id == entry_order
                                })
                            });
                        let motion_latch_debug = motion_latch_debug_config().filter(|config| {
                            config.frame == self.control.frame_counter
                                && config.creation_order
                                    == self.world.original_creation_order(entity_id)
                        });
                        let installed_order = motion_latch_debug.and_then(|_| {
                            self.world
                                .entities
                                .get(entity_id)
                                .and_then(Entity::actor_data)
                                .and_then(|actor| actor.installed_order)
                        });
                        if let Some(actor) = self
                            .world
                            .entities
                            .get_mut(entity_id)
                            .and_then(Entity::actor_data_mut)
                        {
                            // Actor::DoNextOrder overwrites a TERMINATED Execute
                            // result with IN_PROGRESS when Proceed exposes another
                            // order.  Instruct does the same when terminating the
                            // old element synchronously installs a successor.
                            // Specialized owners retire their order internally,
                            // so an entry-identity change is their equivalent of
                            // the base Hourglass TERMINATED branch even when the
                            // last raw sprite edge was START/DONE/IN_PROGRESS.
                            // ABORTED is tied to the sequence element captured
                            // on Actor::Hourglass entry. Its synchronous
                            // Impossible condolence may install Wait and
                            // overwrite the sprite's last edge, but it cannot
                            // rewrite the Execute return already held by Actor.
                            let motion_before_projection = actor.continuation.motion_state;
                            actor.continuation.motion_state = project_post_completion_motion(
                                motion_before_projection,
                                selected_element_impossible && !explicit_execute_in_progress,
                                installed_successor_exists,
                                selected_specialized_order_advanced,
                            );
                            actor.continuation.motion_state =
                                apply_post_completion_execute_override(
                                    actor.continuation.motion_state,
                                    post_completion_execute_override,
                                    selected_element_interrupted,
                                    installed_successor_exists,
                                );
                            if let Some(config) = motion_latch_debug {
                                eprintln!(
                                    "[MOTION_LATCH frame={} co={} owner={} entry_order={:?} entry_state={:?} specialized_motion={:?} explicit_in_progress={} retired={} interrupted={} impossible={} specialized_advanced={} installed_order={:?} installed_successor={} motion_before={:?} motion_after={:?}]",
                                    config.frame,
                                    config.creation_order,
                                    entity_id.index(),
                                    selected_order,
                                    selected_element_state,
                                    specialized_execute_motion,
                                    explicit_execute_in_progress,
                                    selected_element_retired,
                                    selected_element_interrupted,
                                    selected_element_impossible,
                                    selected_specialized_order_advanced,
                                    installed_order,
                                    installed_successor_exists,
                                    motion_before_projection,
                                    actor.continuation.motion_state,
                                );
                            }
                            tracing::trace!(
                                target: "parity_motion_state",
                                entity = ?entity_id,
                                family = ?selected_owner_family,
                                entry_order = ?selected_order,
                                specialized_motion = ?specialized_execute_motion,
                                element_retired = selected_element_retired,
                                element_interrupted = selected_element_interrupted,
                                specialized_advanced = selected_specialized_order_advanced,
                                installed_successor = installed_successor_exists,
                                motion_state = ?actor.continuation.motion_state,
                                "actor motion-state latch",
                            );
                        }
                        // DoNextOrder may synchronously expose a real postponed
                        // successor through SetState/Ready. If it does not,
                        // Original leaves mpOrder null for the rest of this
                        // Actor::Hourglass call. The fallback Wait is created
                        // only by the null-order guard at the start of the next
                        // actor frame, so ActionChange observes NONANIMATION_END
                        // on this completion frame.
                        observe_actor_animation_boundary(
                            ActorAnimationBoundaryPhase::CompletionEffects(entity_id),
                        );

                        // Release every animation/completion borrow before the VM:
                        // ActionChange can synchronously replace this or a later
                        // actor's order and the next slot must sample that live.
                        observe_actor_animation_boundary(
                            ActorAnimationBoundaryPhase::ActionChange(entity_id),
                        );
                        self.dispatch_actor_action_change_for(sim, assets, entity_id);
                        // Do not derive mpOrder from the manager at the tail. The
                        // exact pointer was published at Hourglass entry and is
                        // subsequently changed only by DoNextOrder, selected
                        // element cleanup, or a synchronous accepted Instruct.
                        let installed_tail_order_type = self
                            .world
                            .entities
                            .get(entity_id)
                            .and_then(Entity::actor_data)
                            .and_then(|actor| actor.installed_order)
                            .map(|order| order.order_type)
                            .unwrap_or(crate::order::OrderType::NonanimationEnd);
                        self.debug_refresh_view_lifecycle(
                            "derived_tail_normal",
                            entity_id,
                            Some(installed_tail_order_type),
                        );
                        after_slot(self, entity_id, installed_tail_order_type);
                        if let Some(entity) = self.world.entities.get(entity_id) {
                            super::animation::direction_provenance_snapshot(
                                entity.position_iface(),
                                entity_id,
                                self.control.frame_counter,
                                "owner_tail_after_derived",
                            );
                        }

                        if let Some(actor) = self
                            .world
                            .entities
                            .get_mut(entity_id)
                            .and_then(Entity::actor_data_mut)
                        {
                            actor.execute_order_initialising = false;
                        }
                        self.debug_drop_owner_boundary(
                            "execute_latch_cleared",
                            entity_id,
                            selected_order,
                        );

                        // Human::SetPosture updates intersecting-corpse state
                        // synchronously in Original. Close the owner-local
                        // boundary before the next creation slot samples this
                        // actor for anti-collision.
                        self.process_corpse_intersection_update_for(entity_id);

                        let leaked_slot_work = self
                            .orders
                            .sequence_manager
                            .take_pending_synchronous_actions();
                        assert!(
                            leaked_slot_work.is_empty(),
                            "actor {entity_id:?} leaked synchronous sequence work after ActionChange at legacy slot {slot}: {leaked_slot_work:?}"
                        );
                        self.orders
                            .sequence_manager
                            .restore_pending_synchronous_actions(preexisting_sequence_work);
                    }
                } else {
                    non_actor_slot(self, entity_id);
                }
            }

            // Original RemoveElement compacts marrayElements immediately, so
            // incrementing the loop index skips the element shifted into the
            // removed position. Retaining before incrementing reproduces that
            // behavior. RegisterElement appends newly created elements; their
            // monotonically increasing creation identities let us discover
            // only the new tail without confusing stable Rust slots for
            // Original array positions.
            original_slots.retain(|&id| self.world.entities.get(id).is_some());
            if self.world.next_original_creation_order != observed_creation_counter {
                assert!(
                    self.world.next_original_creation_order > observed_creation_counter,
                    "Original creation counter moved backwards during Hourglass"
                );
                original_slots.extend(
                    self.world
                        .original_creation_order_by_entity
                        .iter()
                        .filter_map(|(&id, &creation_order)| {
                            (creation_order >= observed_creation_counter
                                && self.world.entities.get(id).is_some())
                            .then_some(id)
                        }),
                );
                original_slots.sort_by_key(|&id| self.world.original_creation_order(id));
                observed_creation_counter = self.world.next_original_creation_order;
            }
            slot += 1;
        }

        // The Original Execute override chain is closed here: generic sprite
        // arms use tick_actor_animation_for; selected movement, melee, bow,
        // ability, beggar, and WaitingSword work use their live owner arms;
        // the human/PC/NPC derived tail hook runs before the slot advances.
    }

    /// Fuse the supported Actor → Human → PC/NPC Hourglass slices into one
    /// live Original-element walk. The underlying actor coordinator owns the
    /// compact creation-ordered loop, including removals and callback-spawned
    /// tail elements; this hook closes the derived tail before it increments
    /// the slot.
    pub(super) fn tick_actor_owner_envelopes_with_display(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
    ) {
        self.tick_actor_owner_envelopes_with_owner_hook(
            sim,
            display,
            assets,
            positions_before_movement,
            |_, _| {},
        )
    }

    #[cfg(test)]
    pub(super) fn tick_actor_owner_envelopes_with_test_owner_hook(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        owner_hook: impl FnMut(&mut Self, EntityId),
    ) {
        let mut display = CameraDisplayState::default();
        self.tick_actor_owner_envelopes_with_owner_hook(
            sim,
            &mut display,
            assets,
            positions_before_movement,
            owner_hook,
        )
    }

    fn tick_actor_owner_envelopes_with_owner_hook(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        mut owner_hook: impl FnMut(&mut Self, EntityId),
    ) {
        let mut prepared = {
            let _detail = entity_system_detail_guard(EntitySystemDetail::PrepareNpc);
            self.prepare_npc_owner_pass(sim, assets)
        };
        self.tick_actor_animation_action_change_slots_with_hooks(
            sim,
            assets,
            |engine, owner| {
                let _detail = entity_system_detail_guard(EntitySystemDetail::StaticOwners);
                use crate::element::OriginalHourglassClass as Class;

                // Original-derived nonactor nesting: the mobile master/child
                // boundary runs before the independent static owner, followed
                // by projectile/net virtual dispatch.
                let class = engine
                    .get_entity(owner)
                    .unwrap_or_else(|| {
                        panic!(
                            "Hourglass owner {owner:?} disappeared immediately after live legacy-slot resolution"
                        )
                    })
                    .original_hourglass_class();
                match class {
                    Class::FxMasked => assert!(
                        engine.tick_mobile_child_owner_boundary(sim, assets, owner),
                        "mapped FXMasked owner {owner:?} lost its mobile boundary"
                    ),
                    Class::Fx
                    | Class::Target
                    | Class::Bonus
                    | Class::Ale
                    | Class::Cape
                    | Class::Scroll => {
                        engine.tick_static_entity_hourglass_for(sim, assets, owner)
                    }
                    Class::Arrow
                    | Class::Apple
                    | Class::Stone
                    | Class::Purse
                    | Class::Coin
                    | Class::Net
                    | Class::WaspNest
                    | Class::Wasp => {
                        engine.tick_projectile_or_net_hourglass(sim, assets, owner)
                    }
                    Class::ActorPc | Class::ActorSoldier | Class::ActorCivilian => {}
                }
            },
            |engine, owner| {
                let _detail = entity_system_detail_guard(EntitySystemDetail::OwnerPrelude);
                // The jump step lifecycle is the jump order's own work: the
                // step that starts here is the order this actor executes a few
                // lines later, and the landing posture it publishes is visible
                // to every later creation slot and to none of the earlier ones.
                engine.tick_active_jump_for(assets, owner);
                if matches!(owner, EntityId::Soldier(_)) {
                    observe_actor_owner_envelope(ActorOwnerEnvelopePhase::SoldierPrelude(owner));
                    engine.tick_apple_smell_for(owner);
                    engine.tick_soldier_track_primary_target_for(owner);
                    engine.tick_attacking_reactiontime_enemy_near_for(sim, assets, owner);
                }
                if matches!(owner, EntityId::Soldier(_) | EntityId::Civilian(_))
                    && !engine.actors_frozen()
                {
                    observe_actor_owner_envelope(ActorOwnerEnvelopePhase::Patrol(owner));
                    engine.tick_patrol_coordination_for_npc(
                        sim,
                        assets,
                        owner,
                        positions_before_movement,
                    );
                }
                if engine
                    .world
                    .entities
                    .get(owner)
                    .is_some_and(|entity| entity.human_data().is_some())
                {
                    observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanPrelude(owner));
                    engine.tick_concussion_healing_for(sim, owner, assets);
                    engine.process_shoot_list_for(sim, assets, owner);
                }
            },
            |engine, owner, movement, melee, bow, ability, selected_beggar| {
                let _detail = entity_system_detail_guard(EntitySystemDetail::OwnerExecute);
                let execution_frozen = engine
                    .get_entity(owner)
                    .and_then(Entity::actor_data)
                    .is_some_and(|actor| actor.execution_frozen);
                if execution_frozen {
                    return ExplicitExecuteMotion::default();
                }
                // PerformSeek's "wait for the post seek sequence to be
                // launched" arm runs ahead of every other seek step: Execute
                // returns TERMINATED before any motion, countdown ageing, or
                // RefreshSeek, and Actor::Hourglass then runs DoNextOrder.
                if let Some(selection) = movement
                    && super::refresh_seek::perform_seek_lost_actor_target(
                        engine, owner, selection,
                    )
                {
                    return ExplicitExecuteMotion {
                        initial: Some(crate::sprite::MotionState::Terminated),
                        post_completion_override: None,
                    };
                }
                // RefreshSeek is part of this exact actor's PerformSeek
                // Execute arm. Sampling here preserves creation-order
                // visibility of the moving target, and a replacement does
                // not itself execute until this owner returns next frame.
                if movement.is_some() {
                    if let Some(motion) =
                        engine.tick_refreshing_seek_for_owner(sim, assets, owner)
                    {
                        return ExplicitExecuteMotion {
                            initial: Some(motion),
                            post_completion_override: None,
                        };
                    }
                    // FaceOpponent / FaceDangerPoint run inside the Execute
                    // arm *before* PerformSeek (RHelementactorhuman.cpp:3662,
                    // RHelementactorpc.cpp:5514), so their facing write and
                    // Turn still happen on the frame PerformSeek's
                    // moved-target RefreshSeek branch preempts the motion.
                    if engine.selected_seek_refresh_decision(owner).is_some() {
                        engine.apply_pre_perform_seek_facing_prologue(owner);
                    }
                    if engine.tick_refresh_seek_for_owner(sim, assets, owner) {
                        return ExplicitExecuteMotion {
                            initial: Some(crate::sprite::MotionState::InProgress),
                            post_completion_override: None,
                        };
                    }
                }
                // PerformSeek's completion-time RefreshSeek branches return
                // RHMOTION_IN_PROGRESS explicitly
                // (`original-code/RHelementactor.cpp:7963-7970`, `:8002-8007`),
                // so Actor::Hourglass runs none of its DONE / TERMINATED /
                // ABORTED tail for that slot.
                let movement_motion =
                    engine.tick_entity_movement_owner(sim, assets, owner, movement);
                if movement_motion.initial.is_some()
                    || movement_motion.post_completion_override.is_some()
                {
                    return ExplicitExecuteMotion {
                        initial: movement_motion.initial,
                        post_completion_override: movement_motion.post_completion_override,
                    };
                }
                if let Some(selection) = melee {
                    engine.tick_selected_melee_owner(sim, assets, owner, selection);
                    if engine
                        .world
                        .entities
                        .get(owner)
                        .is_some_and(Entity::is_pc)
                    {
                        // The PC override wraps Human::Execute. Therefore its
                        // START-edge remark follows Human's strike warning,
                        // but still belongs to this actor's live slot.
                        engine.tick_pc_combat_anim_speech_for_owner(sim, assets, owner);
                    }
                }
                if let Some((_, _, order_id)) = bow {
                    engine.tick_bow_shot_for(sim, assets, owner, order_id);
                }
                if ability.is_some() {
                    let listen_phase = engine
                        .get_entity(owner)
                        .and_then(Entity::actor_data)
                        .filter(|actor| {
                            actor.active_ability.kind
                                == Some(crate::movement::AbilityKind::Listen)
                        })
                        .map(|actor| actor.listen_phase);
                    let listen_counting = listen_phase
                        == Some(crate::element::ListenPhase::CountingDown);
                    let listen_advanced = listen_phase.is_some()
                        && engine.tick_enemy_ai_blip_detection_for_owner(sim, assets, owner);
                    // Original's RHANIMATION_LISTENING Execute arm ignores
                    // the sprite's DONE/TERMINATED states and remains in
                    // progress until mulWaitTime reaches zero. The detection
                    // owner arm above is the complete Execute implementation
                    // while CountingDown; running generic tick_ability as
                    // well would let the short looping sprite terminate the
                    // order and enter the exit transition early.
                    if !listen_counting && !listen_advanced {
                        engine.tick_ability_for(sim, display, assets, owner);
                    }
                }
                if let Some(order_id) = selected_beggar {
                    engine.tick_beggar_bid_for(sim, assets, owner, order_id);
                }
                ExplicitExecuteMotion::default()
            },
            |engine, owner, derived_tail_order_type| {
                let _detail = entity_system_detail_guard(EntitySystemDetail::NpcTail);
                let is_human = engine
                    .world
                    .entities
                    .get(owner)
                    .unwrap_or_else(|| {
                        panic!(
                            "actor owner {} disappeared before its derived Hourglass tail",
                            owner.index()
                        )
                    })
                    .human_data()
                    .is_some();
                if !is_human {
                    return;
                }
                match owner {
                    EntityId::Pc(_) => {
                        engine.refresh_pc_produced_noise_for_with_order(
                            owner,
                            derived_tail_order_type,
                        );
                        prepared.invalidate_after_pc_noise_refresh();
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanNoise(owner));
                        engine.tick_tiredness_for(owner, assets);
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanTiredness(
                            owner,
                        ));
                        if engine
                            .world
                            .entities
                            .get(owner)
                            .is_some_and(|entity| entity.ai_controller().is_some())
                        {
                            engine.tick_npc_owner_pass(
                                sim,
                                assets,
                                positions_before_movement,
                                &mut prepared,
                                owner,
                            );
                        }
                        engine.tick_pc_auto_heal_for(sim, owner);
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::PcTail(owner));
                    }
                    EntityId::Soldier(_) | EntityId::Civilian(_) => {
                        engine.tick_tiredness_for(owner, assets);
                        // NPC humans have no produced-noise refresh, so
                        // their Human tail begins at tiredness.
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanTiredness(
                            owner,
                        ));
                        engine.tick_npc_owner_pass(
                            sim,
                            assets,
                            positions_before_movement,
                            &mut prepared,
                            owner,
                        );
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::NpcTail(owner));
                    }
                    _ => panic!(
                        "human actor owner {} has unsupported entity kind",
                        owner.index()
                    ),
                }
                owner_hook(engine, owner);
            },
        );
        {
            let _detail = entity_system_detail_guard(EntitySystemDetail::FinishNpc);
            self.finish_npc_owner_pass();
        }
    }

    #[cfg(test)]
    pub(super) fn tick_actor_owner_envelopes(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
    ) {
        let mut display = CameraDisplayState::default();
        self.tick_actor_owner_envelopes_with_display(
            sim,
            &mut display,
            assets,
            positions_before_movement,
        );
    }

    /// Dispatch the exact Original virtual `Hourglass` chain for a live
    /// projectile/net creation slot.  Entity kind and `ObjectType` together
    /// are the Rust vtable: accepting any other pairing here would fabricate
    /// subtype behaviour that the loaded object never had.
    pub(super) fn tick_projectile_or_net_hourglass(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        id: EntityId,
    ) {
        let Some(entity) = self.get_entity(id) else {
            return;
        };
        // Validate the Rust kind/ObjectType vtable pairing before the base
        // inactive-removal rule. Otherwise an impossible inactive object
        // silently disappears while the same active object panics.
        match entity {
            Entity::Projectile(projectile)
                if !matches!(
                    projectile.object.object_type,
                    crate::element::ObjectType::Arrow
                        | crate::element::ObjectType::Apple
                        | crate::element::ObjectType::Stone
                        | crate::element::ObjectType::Purse
                        | crate::element::ObjectType::Coin
                        | crate::element::ObjectType::WaspNest
                        | crate::element::ObjectType::BonusWaspNest
                        | crate::element::ObjectType::Wasp
                ) =>
            {
                panic!(
                    "projectile entity {id:?} has unsupported ObjectType::{:?}; TODO(PA-013): map its Original concrete class",
                    projectile.object.object_type
                )
            }
            Entity::Net(net)
                if !matches!(
                    net.object.object_type,
                    crate::element::ObjectType::Net | crate::element::ObjectType::BonusNet
                ) =>
            {
                panic!(
                    "net entity {id:?} has unsupported ObjectType::{:?}; expected Net or BonusNet",
                    net.object.object_type
                )
            }
            _ => {}
        }
        let dispatch = match entity {
            Entity::Projectile(projectile) => Some((
                true,
                projectile.object.object_type,
                projectile.element.active,
            )),
            Entity::Net(net) => Some((false, net.object.object_type, net.element.active)),
            _ => None,
        };
        let Some((is_projectile, object_type, base_active)) = dispatch else {
            return;
        };
        let retain = if is_projectile {
            match object_type {
                crate::element::ObjectType::Arrow => {
                    if base_active {
                        let flying = self
                            .get_entity(id)
                            .and_then(|entity| match entity {
                                Entity::Projectile(projectile) => {
                                    Some(projectile.projectile.flying)
                                }
                                _ => None,
                            })
                            .expect("arrow owner changed concrete entity kind");
                        if flying {
                            self.tick_existing_projectile(sim, assets, id);
                        } else if let Some(Entity::Projectile(projectile)) =
                            self.world.entities.get_mut(id)
                        {
                            // RHElementProjectile::Hourglass calls NewMove
                            // before testing mbFlying. Active stopped arrows
                            // therefore settle old=current on every owner tick
                            // until the later Refresh retires them.
                            projectile.element.sprite.position_iface.new_move();
                        }
                    }
                    base_active
                }
                crate::element::ObjectType::Apple | crate::element::ObjectType::Stone => {
                    if base_active {
                        self.tick_existing_projectile(sim, assets, id);
                    }
                    let frozen = self.actors_frozen();
                    if let Some(Entity::Projectile(projectile)) = self.get_entity_mut(id)
                        && !projectile.projectile.flying
                        && !frozen
                    {
                        observe_projectile_derived_tail(id, object_type);
                        let motion = projectile.element.sprite.perform_virgin_increment(
                            sim,
                            crate::sprite::FrameProgression::Default,
                        );
                        projectile.element.active =
                            motion != crate::sprite::MotionState::Terminated;
                    }
                    // Apple/Stone return the Projectile base result even
                    // though their grounded sprite tail may have changed
                    // active state afterward.
                    base_active
                }
                crate::element::ObjectType::Purse | crate::element::ObjectType::Coin => {
                    self.tick_purse_or_coin(sim, assets, id)
                }
                crate::element::ObjectType::WaspNest
                | crate::element::ObjectType::BonusWaspNest
                | crate::element::ObjectType::Wasp => {
                    self.tick_wasp_nest_or_wasp(sim, assets, id);
                    base_active
                }
                unsupported => panic!(
                    "projectile entity {id:?} has unsupported ObjectType::{unsupported:?}; TODO(PA-013): map its Original concrete class"
                ),
            }
        } else {
            match object_type {
                crate::element::ObjectType::Net | crate::element::ObjectType::BonusNet => {
                    self.tick_net(sim, assets, id);
                    true
                }
                unsupported => panic!(
                    "net entity {id:?} has unsupported ObjectType::{unsupported:?}; expected Net or BonusNet"
                ),
            }
        };
        if !retain && let Some(entity) = self.get_entity_mut(id) {
            // RHEngine::RemoveElement is called with its default
            // bOnlyDeactivate=true from the element Hourglass loop. The
            // projectile remains in the element array as an inactive
            // tombstone so outstanding references and creation order stay
            // valid; physical removal is reserved for teardown/load paths.
            entity.element_data_mut().active = false;
        }
    }

    /// Apply the two sequence-command motion modifiers owned by
    /// `RHElementActor::Hourglass` after one actor's Execute call.
    fn apply_actor_post_execute_wait_modifier(
        &mut self,
        owner: EntityId,
        execute_result: &mut super::animation::ActorExecuteResult,
    ) {
        self.apply_actor_post_execute_wait_modifier_to_motion(
            owner,
            execute_result.entry_seq_id,
            execute_result.entry_elem_idx,
            &mut execute_result.motion,
        );
    }

    fn apply_actor_post_execute_wait_modifier_to_motion(
        &mut self,
        owner: EntityId,
        entry_seq_id: crate::sequence::SequenceId,
        entry_elem_idx: usize,
        motion: &mut crate::sprite::MotionState,
    ) {
        let entry_command = self
            .orders
            .sequence_manager
            .get_element(entry_seq_id, entry_elem_idx)
            .map(|element| element.command);
        let live_element = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner);
        let live_command = live_element.and_then(|(seq_id, elem_idx)| {
            self.orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .map(|element| element.command)
        });

        // Execute is selected from mpSequenceElement before entering the
        // actor's derived virtual method. A WaitingSword callback may stop
        // that element before control returns to Actor::Hourglass, but the
        // C++ member retains its pointer while this Hourglass stack unwinds.
        // Rust's live-element scan then returns None, so fall back to the
        // Execute-entry identity. A genuinely instructed synchronous
        // replacement remains live and takes precedence. Completion itself
        // is still resolved against the then-live element by
        // stage_actor_execute_completion.
        let effective_command = live_command.or(entry_command);
        if effective_command == Some(crate::element::Command::WaitTimer) {
            let actor = self
                .world
                .entities
                .get_mut(owner)
                .unwrap_or_else(|| panic!("WAIT_TIMER post-Execute owner {owner:?} is missing"))
                .actor_data_mut()
                .unwrap_or_else(|| {
                    panic!("WAIT_TIMER post-Execute owner {owner:?} is not an actor")
                });
            if actor.wait_time == 0 {
                actor.seek_refresh_wait = 0;
                *motion = crate::sprite::MotionState::Terminated;
            } else {
                actor.wait_time -= 1;
                actor.seek_refresh_wait = actor.wait_time;
            }
            return;
        }

        if live_command == Some(crate::element::Command::WaitFreeLift) {
            if let Some((seq_id, elem_idx)) = live_element {
                let world = &mut self.world;
                let authorized = super::sequence_runtime::LiftWaitCommandContext {
                    entities: &mut world.entities,
                    fast_grid: std::sync::Arc::make_mut(&mut world.fast_grid),
                    doors: self.script_domains.interactables.doors.as_slice(),
                    sequence_manager: &mut self.orders.sequence_manager,
                }
                .authorize_and_reserve(owner, seq_id, elem_idx);
                if authorized {
                    *motion = crate::sprite::MotionState::Terminated;
                }
            }
        }
    }

    /// Resolve the retained base-Actor motion after derived Execute callbacks
    /// and wait modifiers. Original TERMINATED calls DoNextOrder through the
    /// owner's live `mpSequenceElement`; ABORTED alone uses the sequence
    /// element snapshot captured before Execute.
    fn stage_actor_execute_completion(
        &mut self,
        owner: EntityId,
        entry_order_id: Option<std::num::NonZeroU32>,
        execute_result: super::animation::ActorExecuteResult,
        outcomes: &mut super::animation::AnimCompletionOutcomes,
    ) {
        match execute_result.motion {
            crate::sprite::MotionState::Aborted => outcomes
                .seq_impossible
                .push((execute_result.entry_seq_id, execute_result.entry_elem_idx)),
            crate::sprite::MotionState::Terminated => {
                let Some((seq_id, elem_idx, order)) =
                    self.orders.sequence_manager.current_order_for_actor(owner)
                else {
                    return;
                };
                match order.completion.clone() {
                    crate::order::OrderCompletion::AdvanceElement => {
                        outcomes.seq_advance.push((seq_id, elem_idx));
                    }
                    crate::order::OrderCompletion::UnlockDoor { door_id } => {
                        let _ = door_id;
                        outcomes.seq_advance.push((seq_id, elem_idx));
                    }
                    crate::order::OrderCompletion::ResumeDoorPass => {
                        outcomes.resume_door_pass.push(owner);
                    }
                    crate::order::OrderCompletion::NextJumpStep => {
                        outcomes.next_jump_step.push(owner);
                    }
                    crate::order::OrderCompletion::WaspStruggleCycle { cycles_remaining } => {
                        if cycles_remaining <= 1 {
                            outcomes.seq_terminate.push((seq_id, elem_idx));
                        } else {
                            outcomes
                                .wasp_next_cycle
                                .push((seq_id, elem_idx, cycles_remaining - 1));
                        }
                    }
                }
            }
            crate::sprite::MotionState::Done => {
                // RHElementActorPC::Execute creates a dropped ale bottle at
                // the DROPPING_ALE action point. Stage this on the retained
                // Actor::Hourglass result rather than inside the generic
                // animation dispatcher: DONE is written back through this
                // lifecycle seam after derived Execute callbacks, and save-
                // loaded orders can otherwise lose the earlier transient
                // side-outcome while still marking the order done.
                if matches!(
                    execute_result.order_type,
                    crate::order::OrderType::DroppingAle
                        | crate::order::OrderType::DroppingAleCrouched
                ) {
                    outcomes.execute_sides.drop_ale_done.push(owner);
                }
                let order_id = entry_order_id.unwrap_or_else(|| {
                    panic!(
                        "actor {owner:?} returned Done without an entry-latched order for {:?}/{}",
                        execute_result.entry_seq_id, execute_result.entry_elem_idx
                    )
                });
                self.mark_entry_order_done(
                    owner,
                    execute_result.entry_seq_id,
                    execute_result.entry_elem_idx,
                    order_id,
                );
            }
            crate::sprite::MotionState::Start | crate::sprite::MotionState::InProgress => {}
            crate::sprite::MotionState::Error => panic!(
                "actor {owner:?} Execute returned MotionState::Error from entry {:?}/{}",
                execute_result.entry_seq_id, execute_result.entry_elem_idx
            ),
        }
    }

    fn mark_entry_order_done(
        &mut self,
        owner: EntityId,
        entry_seq_id: crate::sequence::SequenceId,
        entry_elem_idx: usize,
        order_id: std::num::NonZeroU32,
    ) {
        let Some(element) = self
            .orders
            .sequence_manager
            .get_element_mut(entry_seq_id, entry_elem_idx)
        else {
            // Execute may synchronously terminate and collect its own entry
            // element before returning. Original still writes through the
            // retained mpOrder allocation, but no later priority decision can
            // observe that detached order.
            tracing::trace!(
                ?owner,
                ?entry_seq_id,
                entry_elem_idx,
                %order_id,
                "Done entry element was synchronously collected before Actor::Hourglass write-back"
            );
            return;
        };
        let Some(order) = element
            .orders
            .iter_mut()
            .find(|order| order.order_id == order_id)
        else {
            // The same re-entrant teardown can retain the terminal element
            // shell while deleting its order list.
            tracing::trace!(
                ?owner,
                ?entry_seq_id,
                entry_elem_idx,
                %order_id,
                "Done entry order was synchronously removed before Actor::Hourglass write-back"
            );
            return;
        };
        // Original Actor::Hourglass sets mpOrder->bDone immediately after
        // Execute returns. Later callbacks in this same owner slot and
        // SequenceManager::Hourglass can therefore terminate a blocker
        // instead of postponing behind an animation which already reached its
        // action point.
        order.done = true;
    }

    /// Run the NPC Hourglass tail and its immediately adjacent notification
    /// passes in the exact order established by the original implementation.
    ///
    /// Original provenance: `RHElementActorNPC::Hourglass` in
    /// `original-code/RHelementactornpc.cpp:3495-3614`.
    fn hourglass_phase_npcs(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        _assets: &LevelAssets,
        _positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
    ) {
        // Listen/object reveal and Target Heard are actor-owned Execute work.
        // ── Creation-ordered pre-detection boundary ──────────────
        // These observations remain coarse labels for the original nested
        // order. The coordinator below interleaves the actual operations per
        // NPC: own synchronous FITAGAIN + resurrection/eye apply, own body
        // broadcast, own view refresh, then that same NPC's RefreshDetection.
        observe_npc_hourglass_phase(NpcHourglassPhase::Broadcasts);

        observe_npc_hourglass_phase(NpcHourglassPhase::View);

        observe_npc_hourglass_phase(NpcHourglassPhase::Detection);
        // Production work already ran inside the live actor-owner walk in the
        // preceding EntitySystems phase. Keep these coarse observations for
        // the PA-016 tick-spine contract only.

        // The phase observations below retain the coarse PA-016 ordering
        // contract. Production work no longer runs here: PA-013 executes the
        // complete post-detection tail inside each NPC's creation slot before
        // the next NPC enters RefreshDetection.
        observe_npc_hourglass_phase(NpcHourglassPhase::Ambush);

        // ── Per-tick AILOCK_BUSY edge detector ─────────────────
        // Lock or unlock AILOCK_BUSY based on the live
        // `is_very_very_busy` predicate (posture or active PassDoor /
        // Fall element).  Runs after the view refresh.
        observe_npc_hourglass_phase(NpcHourglassPhase::Busy);

        // ── Stuck-on-ladder emergency counter ──────────────────
        // Bump per frame for non-script-locked NPCs on outdoor
        // ladders idling in CMD_WAIT/CMD_MOVE_WAITING; after 25
        // frames force a ReturnToDuty so the actor can self-recover.
        // Runs after the BUSY edge detector.
        observe_npc_hourglass_phase(NpcHourglassPhase::Ladder);

        // ── Locked-frame timer bumps ───────────────────────────
        // When any lock is held the entire Hourglass tail
        // short-circuits while the three timer ring-frames
        // (`when_does_timer_ring`, `when_does_macro_timer_ring`,
        // `emoticon_expiration_date`) tick forward by +1.  This both
        // keeps the relative timer offset stable across the lock
        // window and acts as the "skip the fire" gate for the
        // downstream macro-timer / EVENT_TIMER fire checks (which
        // compare against the live `frame_counter`).
        observe_npc_hourglass_phase(NpcHourglassPhase::LockGate);

        // The unlocked tail is ordered exactly like the original callee:
        // The16thFrame, normal EVENT_TIMER, macro timer, then stimuli held
        // by a prior AI/script lock.
        observe_npc_hourglass_phase(NpcHourglassPhase::SixteenthFrame);

        observe_npc_hourglass_phase(NpcHourglassPhase::NormalTimer);

        // ── Macro-timer hourglass ──────────────────────────────
        // Poll the macro-specific timer each frame and, when it
        // rings, call `execute_next_macro_command` directly —
        // bypassing the stimulus queue so CMD_WAIT / CMD_BEND
        // resume cleanly. Any resulting movement-order / substate change
        // is visible to the queued-stimulus drain in the same frame.
        observe_npc_hourglass_phase(NpcHourglassPhase::MacroTimer);

        observe_npc_hourglass_phase(NpcHourglassPhase::QueuedStimuli);

        // Every engine-entered AI call closes its ordered owner-local
        // SetState/Say boundary before returning to effects/orders. Nothing
        // may survive into the obsolete post-NPC speech batch position.
        let frame_counter = self.control.frame_counter;
        for (npc_id, entity) in self.world.entities.npcs() {
            let leaked = entity
                .ai_controller()
                .map(|ai| ai.outbox.reentrant.owner_work.as_slice())
                .unwrap_or_default();
            assert!(
                leaked.is_empty(),
                "NPC {} leaked owner-local AI work past its Hourglass slot on frame {}: {leaked:?}",
                npc_id.index(),
                frame_counter,
            );
        }

        // ── HUD speech-log decay ────────────────────────────────
        // Decrement the per-remark display timer and evict expired
        // entries every frame regardless of `speech_display` so the
        // Vec does not grow unbounded when the overlay is off.
        self.tick_screen_remarks();

        // TODO(PA-013): pure-Rust handlers still enqueue until their AI borrow
        // returns, so arbitrary reads between Say/SetState statements cannot
        // yet observe the Original's fully inline engine/audio call stack.
    }

    /// Advance combat, projectiles, abilities, and other gameplay systems that
    /// consume the entity/sequence/NPC state established above.
    pub(super) fn hourglass_phase_gameplay_systems(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        _display: &mut CameraDisplayState,
        assets: &LevelAssets,
    ) {
        // Active abilities, Listen/Heard, projectiles, and beggar simulation
        // already executed in their live owner slots.

        // Combat progression without a proven cross-subsystem ordering
        // discrepancy remains batched. Fallback-timed completions already
        // cleared at their owning actor slots above and are skipped here.
        self.tick_melee_combat(sim, assets);

        // Preserve only the terminal shoulder-climb sprite synchronization
        // before the motion latch is consumed. Carried transforms remain in
        // their established post-propagation phase below.
        abilities::sync_terminal_shoulder_animations(
            &mut self.world.entities,
            &self.world.original_creation_order_by_entity,
        );

        // ── Per-actor `Order::done` propagation ────────────────
        // Runs after every per-system sprite-advance tick this frame
        // (movement, jumps, animations, bow shots, melee, abilities),
        // each of which has already stashed its result on the sprite
        // via `Sprite::record_motion_state`.  The pass flips
        // `Order::done` on every actor whose sprite reported
        // `MotionState::Done`, then clears `last_motion_state` so the
        // next tick starts fresh.  Read by the postpone-race guard in
        // `EngineInner::engine_postpone`.
        self.propagate_done_to_current_orders();

        // Keep bodies carried by Little John positioned on the carrier and
        // drive their sprite animation synchronized with the carrier.
        abilities::sync_carried_positions(&mut self.world.entities, &assets.profile_manager);

        // TODO(original-parity): move further gameplay maintenance into the
        // ordered pass only when a concrete observable discrepancy is proven.
    }

    /// Apply work intentionally deferred until every entity, path, sequence,
    /// NPC, and gameplay-system update has completed.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3729-3775` performs the
    /// swordfight falling-edge check, titbit update, dead-selection scan, and
    /// anonymous timers after the sequence manager. Rust adds deterministic
    /// condolation, self-stimulus, and immediate-action drains.
    fn hourglass_phase_deferred_effects_end(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        was_swordfighting: bool,
    ) {
        // ── Swordfight-drag IgnoreMouseEvent bracket ────────────
        // If the selected PC was swordfighting at entry to
        // `perform_hourglass` but is no longer swordfighting after
        // the per-element / sequence-manager hourglass, raise the
        // ignore-mouse-event bracket so a drag in flight when the
        // swordfight ended this tick is suppressed.  We push the
        // request as a side effect; the host gates it on
        // `InputState::is_dragging` in `apply_side_effects`.
        if was_swordfighting && !self.is_selected_pc_swordfighting() {
            self.feedback
                .pending_side_effects
                .pending_swordfight_drag_ignore = true;
        }

        // ── Titbit sync + per-frame update ──────────────────────
        // First, sync persistent titbits (emoticons, unconscious
        // stars, alert indicators) with current entity state.
        self.sync_titbits(assets);

        // Then run the titbit update to advance animations and
        // expire finished titbits.
        {
            let query = EntityTitbitQuery {
                sim,
                entities: &self.world.entities,
                sequence_manager: &self.orders.sequence_manager,
                follow_element: self.players.seats[0].follow_element,
            };
            self.feedback.titbit_manager.update(&query);
            // PrepareRefresh: advance blink counter, sort by
            // display order using each supplier entity's Y position
            // as a stand-in (we don't compute display order yet).
            self.feedback.titbit_manager.prepare_refresh(|handle| {
                self.world
                    .entities
                    .id_at_legacy_slot(handle.0)
                    .and_then(|entity_id| self.world.entities.get(entity_id))
                    .map(|e| e.element_data().position_map().y)
            });
        }

        // ── Ground mark animation ────────────────────────────────
        // Advanced after `perform_hourglass_inner` by `ground_mark.tick`,
        // using the deterministic director view. That helper preserves the
        // original on-screen guard, so off-screen marks freeze. The renderer
        // remains read-only; see the wrapper at the start of this file.

        // Selection ring animation lives host-side now —
        // `Game::run_engine_tick` advances `host.selection_mark`
        // once per frame, gated on the same `should_run_hourglass`
        // check as this function, so pause / console still freeze
        // the ring.

        // ── Check selected PCs are still alive ───────────────────
        {
            let mut deselect = Vec::new();
            for &pc_id in &self.players.seats[0].selection {
                if let Some(entity) = self.world.entities.get(pc_id) {
                    let should_deselect = match entity {
                        Entity::Pc(pc) => pc.pc.life_points <= 0 || pc.human.unconscious,
                        _ => false,
                    };
                    if should_deselect {
                        deselect.push(pc_id);
                    }
                }
            }
            for pc_id in deselect {
                // `RHMessenger::ForwardMessage` synchronously routes
                // MSG_UNSELECT_CHARACTER to the engine/game receivers at
                // this exact point. Do not leave the authoritative selection
                // mutation in Rust's next-frame message queue.
                if self.is_sherwood(&assets.profile_manager)
                    && let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id)
                {
                    pc.pc.interface_hidden = true;
                }
                self.unselect_single_pc(pc_id);
                self.update_recording_after_selection_change();
                self.players.action_before_recording_macro = crate::profiles::Action::NoAction;
            }
        }

        // ── Anonymous timers ─────────────────────────────────────
        // `RHEngine::Hourglass` (RHengine.cpp:3794-3810) terminates a timer
        // element only when its `RHFIELD_TIMER` property is *exactly* 1, and
        // otherwise decrements the `int` property. A timer recorded with 0
        // frames — e.g. `RecordTimer( Rand( 25 ) )` rolling a zero — therefore
        // counts down through negative values and NEVER terminates, stalling
        // its sequence level for the rest of the mission. Match that exactly:
        // an `expired if remaining <= 1` test would let the zero case fire a
        // frame later and advance a sequence the Original leaves parked.
        let mut expired: Vec<crate::sequence::SequenceElementRef> = Vec::new();
        self.orders.timer_elements.retain_mut(|timer| {
            if timer.remaining == 1 {
                expired.push(timer.element_ref);
                false
            } else {
                timer.remaining -= 1;
                true
            }
        });
        for r in expired {
            self.orders
                .sequence_manager
                .element_terminated(r.sequence_id, r.element_index);
        }

        // ── Post-timer SendCondolationCard dispatch ──────────────
        // The pre-timer pass after `hourglass_phase_sequences` preserves the
        // original SetState -> SendCondolationCard -> Ready ordering for work
        // that completed before this scan. This second pass is still required:
        // timer expiry above can itself terminate an owned sequence element
        // and queue another card. Its continuation and immediate successors
        // drain below, after this frame's timer iteration has finished.
        self.dispatch_condolations(sim, assets);

        // ── Same-tick re-entrant stimulus dispatch ───────────────
        // The condolation drain calls `Think(EVENT_DONE)` /
        // `Think(EVENT_IMPOSSIBLE)` / etc. synchronously and
        // re-entrantly on the same tick — so e.g. a patrol Turn
        // that gets interrupted when `SetAttentiveMode(true)`
        // launches `ENTER_ATTENTIVE_MODE` during
        // `EventViewStandardProcedure` fires its `EVENT_DONE`
        // *during that same* `EventView` Think, advancing
        // `SUBSTATE_ATTACKING_REACTIONTIME_TURNING` →
        // `REACTIONTIME` before the frame ends.  We can't nest
        // `&mut AiController` borrows mid-think, so
        // `send_condolation_card` queues the stimulus via
        // `fire_self_stimulus` (→ `pending_self_stimuli`).  Drain
        // that queue here — after `dispatch_condolations` has
        // populated it — so the redispatch happens on the same
        // tick as the condolation, keeping
        // `REACTIONTIME_TURNING → REACTIONTIME` timing correct.
        // Without this the substate waits for the full
        // `LaunchTimer(20)` upper bound regardless of which
        // sequence actually completed.
        self.drain_pending_self_stimuli(sim, assets);

        // ── End-of-tick registration-inline drain ───────────────────
        // Anonymous timers run after SequenceManager::Hourglass. Preserve
        // only work Original registration executes on that callback stack:
        // ExecutedImmediately commands and direct RHPRIORITY_WAIT Go calls.
        // Ordinary successors stay queued for the next manager hourglass.
        self.drain_registration_inline_actions_sync(sim, display, assets);
    }

    /// Auto-leave disguise/stealth posture if the entity is in one and
    /// the incoming command requires Upright posture.
    ///
    /// **Superseded.**  The transition logic now lives in
    /// `engine/transitions.rs` and runs at launch time via
    /// `launch_element_for_owner` / the stamped single-order
    /// wrapper.  Posture transitions resolve before the element
    /// becomes `InProgress`, so the dispatch pipeline no longer
    /// needs to peek at posture.
    ///
    /// This helper remains as `#[cfg(test)]` so the legacy edge-case
    /// tests in `engine/tests.rs` that document the partial-port
    /// behaviour still compile.  Those tests cross-check commands the
    /// transitions module also covers; once they're migrated to call
    /// `generate_transition` directly, this function can be deleted.
    #[cfg(test)]
    pub(super) fn auto_leave_disguise_if_needed(
        &mut self,
        owner: EntityId,
        command: Command,
    ) -> bool {
        use crate::stealth;
        use crate::titbit::{ElementHandle, TitbitKind};

        if !stealth::command_requires_upright(command) {
            return false;
        }

        let posture = match self.world.entities.get(owner) {
            Some(e) => e.element_data().posture,
            None => return false,
        };

        // Honor the `CAN_BE_LEANING_OUT` /
        // `CAN_BE_ANONYMOUS_ARCHER` flags that pair with
        // `MUST_BE_UPRIGHT` on a handful of bow commands: the actor
        // keeps its lean-out / anonymous-archer pose rather than
        // unsticking before the shot (e.g. `SHOOT_BOW` from a
        // lean-out window preserves the lean).
        if posture == crate::element::Posture::LeaningOut
            && stealth::command_allows_leaning_out(command)
        {
            return false;
        }
        if posture == crate::element::Posture::AnonymousArcher
            && stealth::command_allows_anonymous_archer(command)
        {
            return false;
        }

        // ENTER_LEISURE permits CAN_BE_LEISURING, letting an
        // already-leisuring NPC re-enter leisure without standing
        // up first.  Skip the auto-leave in that case so the
        // animation pipeline doesn't churn through Upright.
        if command == Command::EnterLeisure && posture == crate::element::Posture::Leisure {
            return false;
        }

        let transition = match stealth::leave_disguise(posture) {
            Some(t) => t,
            None => {
                // Also handle Crouched → Upright for commands that need it.
                if posture == crate::element::Posture::Crouched {
                    stealth::crouch_up()
                } else {
                    return false;
                }
            }
        };

        // Snap posture + action state.  Pre-existing behavior for
        // disguise / crouched transitions is silent (no transition
        // anim queued); the soldier-specific `LeaningOut → Upright`
        // branch additionally queues
        // `TransitionLeaningOutWaitingAlerted` on the actor's
        // order_queue so the lean-out-window soldier plays the
        // visible unstick transition.  Sitting/Leisure are also
        // visible transitions (NPC standing up out of a chair / out
        // of leisure pose), so they queue their animation too.
        let queue_anim = matches!(
            posture,
            crate::element::Posture::LeaningOut
                | crate::element::Posture::Sitting
                | crate::element::Posture::Leisure
        );
        // Look up the sequence element that's currently dispatching
        // this command so the queued transition animation can be
        // tagged with its owner — if the element is later
        // interrupted (injury mid-transition),
        // `send_condolation_card` scrubs the pending order so no
        // ghost animation plays.  The order lives on the sequence
        // element and goes away with it.
        let dispatching = self.find_dispatching_element(owner, command);

        if let Some(entity) = self.world.entities.get_mut(owner) {
            entity.set_posture(transition.result_posture);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = transition.result_action_state;
            }
        }
        if queue_anim {
            // `compute_direction = false` on the transition
            // order — direction is preserved so the soldier
            // finishes facing the same way it was leaning.
            let mut order = crate::order::Order::new(
                transition.animation,
                0.0,
                0.0,
                self.orders.allocate_order_id(),
            );
            order.compute_direction = false;
            if let Some((seq_id, elem_idx)) = dispatching {
                self.orders
                    .sequence_manager
                    .push_order_on(seq_id, elem_idx, order);
            } else {
                // No dispatching element found — spawn a single-
                // order generic sequence so the visible unstick
                // transition still plays.  Without a host element
                // we launch a tiny one just to carry this animation.
                self.launch_single_order_sequence_stamped(
                    &crate::sim_rng::test_context(),
                    &LevelAssets::new(),
                    owner,
                    Command::Generic,
                    order,
                );
            }
        }

        // Set `posture_after_transition` so downstream dispatch
        // (e.g. `NpcAttentionCommandContext`) decides whether to
        // run the command's real transition or snap.
        if let Some((seq_id, elem_idx)) = dispatching
            && let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
        {
            elem.posture_after_transition = transition.result_posture;
            elem.action_state_after_transition = transition.result_action_state;
        }

        // Remove HIDDEN titbit when leaving a hidden posture.
        if posture.is_hidden() {
            self.feedback
                .titbit_manager
                .remove_titbit(TitbitKind::Hidden, ElementHandle(owner.index()));
        }

        tracing::debug!(
            ?owner,
            ?command,
            old_posture = ?posture,
            new_posture = ?transition.result_posture,
            "auto-leave disguise before command"
        );
        true
    }

    /// Find the sequence element currently being dispatched for
    /// `(owner, command)` so auto-leave can update its
    /// `posture_after_transition` / `action_state_after_transition`
    /// fields.
    ///
    /// Only reachable from `auto_leave_disguise_if_needed`, which is
    /// itself `#[cfg(test)]` after the transitions-port migration.
    #[cfg(test)]
    fn find_dispatching_element(
        &self,
        owner: EntityId,
        command: Command,
    ) -> Option<(crate::sequence::SequenceId, usize)> {
        use crate::sequence::SequenceState;
        self.orders
            .sequence_manager
            .live_element_for_actor_matching(owner, |elem| {
                elem.command == command
                    && matches!(elem.state, SequenceState::Todo | SequenceState::InProgress)
            })
    }

    /// Whether `owner` is a beggar civilian that refuses this command.
    ///
    /// Beggars accept only `RECEIVE_PURSE`, `BEGGAR_SHOW_FACE`, and
    /// `WAIT`.  Every other sequence command on a beggar is
    /// rejected — `sequence_manager.element_impossible` fires.
    pub(super) fn beggar_rejects_command(&self, owner: EntityId, cmd: Command) -> bool {
        let is_beggar = self.get_entity(owner).is_some_and(|e| {
            matches!(e, crate::element::Entity::Civilian(c)
                if c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar)
        });
        is_beggar
            && !matches!(
                cmd,
                Command::ReceivePurse | Command::BeggarShowFace | Command::Wait
            )
    }

    pub(super) fn apply_door_pass_transition_done_side_effects(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        use crate::coordinates::MapPoint;
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType as OT;

        let Some((door_index, action, is_pc)) = self.get_entity(entity_id).and_then(|entity| {
            entity.actor_data().and_then(|actor| {
                actor
                    .active_door_pass
                    .as_ref()
                    .map(|dp| (dp.door_index, dp.current_action, entity.is_pc()))
            })
        }) else {
            return;
        };

        let door = required_canonical_door(
            &self.script_domains.interactables.doors,
            door_index,
            "PassDoor transition side effects",
        );
        let (layer_in, layer_out, sector_in, sector_out, point_in, point_mid, point_out) = (
            door.layer_in,
            door.layer_out,
            door.sector_in,
            door.sector_out,
            MapPoint {
                x: door.point_in.x,
                y: door.point_in.y,
            },
            MapPoint {
                x: door.point_mid.x,
                y: door.point_mid.y,
            },
            MapPoint {
                x: door.point_out.x,
                y: door.point_out.y,
            },
        );

        let lift_direction = self
            .grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(sector_in)))
            .and_then(|sector| {
                if sector.lift_type == Some(crate::sector::LiftType::Wall) {
                    Some(sector.lift_direction)
                } else {
                    None
                }
            });

        match action {
            OT::TransitionWaitingUprightClimbingWallUp => {
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(Posture::OnWall);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                }
            }
            OT::TransitionWaitingCrouchedClimbingWallDown => {
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(Posture::OnWall);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                }
                self.set_transition_position_map_and_compute_position_all(
                    assets,
                    entity_id,
                    crate::coordinates::MapPoint {
                        x: point_in.x,
                        y: point_in.y,
                    },
                );
            }
            OT::TransitionWaitingCrouchedClimbingWallDownCrenel => {
                let point_in = crate::coordinates::MapPoint::new(point_in.x, point_in.y);
                // The crenel variant re-latches the old position across the
                // teleport, so the wall-height jump to the door's entry point
                // is not reported as this frame's movement. Only the map half
                // of the latch sees the teleported point: the 3D position is
                // still the pre-teleport one when the latch happens and is
                // re-derived from the map afterwards.
                let pre_teleport_position = self
                    .get_entity(entity_id)
                    .map(|entity| entity.position_iface().get_position());
                self.finalize_special_move_position_using_projection_sector(
                    assets,
                    entity_id,
                    super::special_motion::SpecialMovePosition::Map(point_in),
                    layer_in,
                    u16::from(sector_in),
                    point_in,
                    "crenel climb-down transition",
                );
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    let pi = entity.position_iface_mut();
                    pi.set_old_map_position(point_in);
                    if let Some(position) = pre_teleport_position {
                        pi.set_old_position(position);
                    }
                    entity.set_posture(Posture::OnWall);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                    let elem = entity.element_data_mut();
                    if let Some(dir) = lift_direction {
                        super::animation::direction_provenance_snapshot(
                            &elem.sprite.position_iface,
                            entity_id,
                            self.control.frame_counter,
                            "writer:crenel_completion_instant:before",
                        );
                        elem.set_direction_instantly(dir);
                        super::animation::direction_provenance_snapshot(
                            &elem.sprite.position_iface,
                            entity_id,
                            self.control.frame_counter,
                            "writer:crenel_completion_instant:after",
                        );
                    }
                    // The teleported position is re-aimed at the map goal the
                    // actor was already standing on; the direction is the one
                    // just latched from the lift, so it is not recomputed.
                    elem.sprite.position_iface.compute_increment_all(false);
                }
            }
            OT::TransitionClimbingWallUpWaitingCrouched => {
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(if is_pc {
                        Posture::Crouched
                    } else {
                        Posture::Upright
                    });
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Waiting;
                    }
                }
                self.set_transition_position_map_and_compute_position_all(
                    assets,
                    entity_id,
                    crate::coordinates::MapPoint {
                        x: point_mid.x,
                        y: point_mid.y,
                    },
                );
            }
            OT::TransitionClimbingWallUpWaitingCrouchedCrenel => {
                let point_out_probe = crate::coordinates::MapPoint::new(point_out.x, point_out.y);
                let point_mid_map = crate::coordinates::MapPoint::new(point_mid.x, point_mid.y);
                self.finalize_special_move_position_using_projection_sector(
                    assets,
                    entity_id,
                    super::special_motion::SpecialMovePosition::Map(point_mid_map),
                    layer_out,
                    u16::from(sector_out),
                    point_out_probe,
                    "crenel climb-up transition",
                );
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(Posture::Flying);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                    {
                        let pi = &mut entity.element_data_mut().sprite.position_iface;
                        let point_out = crate::coordinates::MapPoint {
                            x: point_out.x,
                            y: point_out.y,
                        };
                        pi.set_old_map_position(point_out);
                        pi.set_map_goal(point_out);
                        pi.compute_increment_all(true);
                    }
                }
            }
            OT::TransitionClimbingWallDownWaitingUpright => {
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(Posture::Upright);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Waiting;
                    }
                }
            }
            OT::TransitionClimbingLadderUpWaitingCrouched
            | OT::TransitionClimbingLadderUpWaitingUprightAlerted => {
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(if is_pc {
                        Posture::Crouched
                    } else {
                        Posture::Upright
                    });
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Waiting;
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn apply_door_pass_transition_start_side_effects(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        use crate::coordinates::MapPoint;
        use crate::order::OrderType as OT;

        let (door_index, action) = self
            .get_entity(entity_id)
            .and_then(|entity| entity.actor_data())
            .and_then(|actor| {
                actor
                    .active_door_pass
                    .as_ref()
                    .map(|pass| (pass.door_index, pass.current_action))
            })
            .unwrap_or_else(|| {
                panic!(
                    "queued PassDoor transition START effect for {entity_id:?} has no active pass"
                )
            });
        assert!(
            matches!(
                action,
                OT::TransitionClimbingLadderUpWaitingCrouched
                    | OT::TransitionClimbingLadderUpWaitingUprightAlerted
            ),
            "queued PassDoor ladder START effect for {entity_id:?} has action {action:?}"
        );

        let door = required_canonical_door(
            &self.script_domains.interactables.doors,
            door_index,
            "PassDoor transition START",
        );
        let midpoint = MapPoint::new(door.point_mid.x, door.point_mid.y);
        // These two ladder-exit transition Execute arms align the actor to
        // the gate midpoint on raw RHMOTION_START.  This is a positional
        // alignment only: the later PassingDoor order remains responsible
        // for changing sector/layer membership.
        self.set_transition_position_map_and_compute_position_all(assets, entity_id, midpoint);
    }

    fn set_transition_position_map_and_compute_position_all(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        point: crate::coordinates::MapPoint,
    ) {
        self.finalize_special_move_position(
            assets,
            entity_id,
            super::special_motion::SpecialMovePosition::Map(point),
            None,
            None,
            None,
            "door transition",
        );
    }

    pub(super) fn apply_door_pass_transition_completion_side_effects(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        action: crate::order::OrderType,
    ) {
        use crate::coordinates::MapPoint;
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType as OT;

        // A restored Original save may already contain the complete
        // translated PassDoor order chain without Rust's parallel
        // ActiveDoorPass mirror.  This crenel exit needs no door geometry:
        // its terminal Execute arm only publishes the PC state before
        // DoNextOrder selects PASSING_DOOR.
        if action == OT::TransitionClimbingWallUpWaitingCrouchedCrenel
            && self.get_entity(entity_id).is_some_and(|entity| {
                entity.is_pc()
                    && entity
                        .actor_data()
                        .is_some_and(|actor| actor.active_door_pass.is_none())
            })
        {
            let entity = self
                .world
                .entities
                .get_mut(entity_id)
                .expect("crenel transition completion owner disappeared");
            entity.set_posture(Posture::Crouched);
            entity
                .actor_data_mut()
                .expect("crenel transition completion owner is not an actor")
                .action_state = ActionState::Waiting;
            return;
        }

        let Some((door_index, is_pc)) = self.get_entity(entity_id).and_then(|entity| {
            entity.actor_data().and_then(|actor| {
                actor
                    .active_door_pass
                    .as_ref()
                    .map(|dp| (dp.door_index, entity.is_pc()))
            })
        }) else {
            return;
        };

        let Some((snap_point, posture, action_state)) = (|| {
            let door = required_canonical_door(
                &self.script_domains.interactables.doors,
                door_index,
                "PassDoor transition completion",
            );
            let snap = match action {
                OT::TransitionWaitingUprightClimbingWallUp => Some(MapPoint {
                    x: door.point_mid.x,
                    y: door.point_mid.y,
                }),
                OT::TransitionWaitingCrouchedClimbingLadderDown
                | OT::TransitionWaitingUprightClimbingLadderDownAlerted => Some(MapPoint {
                    x: door.point_in.x,
                    y: door.point_in.y,
                }),
                OT::TransitionClimbingWallDownWaitingUpright
                | OT::TransitionClimbingLadderDownWaitingUpright
                | OT::TransitionClimbingLadderDownWaitingUprightAlerted
                | OT::TransitionClimbingWallUpWaitingCrouchedCrenel => None,
                _ => return None,
            };
            let (posture, action_state) = match action {
                OT::TransitionWaitingUprightClimbingWallUp => {
                    (Posture::OnWall, ActionState::Moving)
                }
                OT::TransitionWaitingCrouchedClimbingLadderDown
                | OT::TransitionWaitingUprightClimbingLadderDownAlerted => {
                    (Posture::OnLadder, ActionState::Moving)
                }
                OT::TransitionClimbingWallDownWaitingUpright => {
                    (Posture::Upright, ActionState::Waiting)
                }
                OT::TransitionClimbingLadderDownWaitingUpright
                | OT::TransitionClimbingLadderDownWaitingUprightAlerted => {
                    (Posture::Upright, ActionState::Waiting)
                }
                OT::TransitionClimbingWallUpWaitingCrouchedCrenel => {
                    let posture = if is_pc {
                        Posture::Crouched
                    } else {
                        Posture::Upright
                    };
                    (posture, ActionState::Waiting)
                }
                _ => return None,
            };
            Some((snap, posture, action_state))
        })() else {
            return;
        };
        tracing::trace!(
            ?entity_id,
            ?action,
            ?snap_point,
            ?posture,
            "door transition completion side effects"
        );
        if let Some(snap_point) = snap_point {
            self.set_transition_position_map_and_compute_position_all(
                assets,
                entity_id,
                crate::coordinates::MapPoint {
                    x: snap_point.x,
                    y: snap_point.y,
                },
            );
        }

        let Some(entity) = self.world.entities.get_mut(entity_id) else {
            return;
        };
        let elem = entity.element_data_mut();
        elem.update_grid_cell();
        entity.set_posture(posture);
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = action_state;
        }
    }

    fn apply_helper_driven_shoulder_dismount(
        &mut self,
        dismount: super::animation::ShoulderHelperDismount,
    ) {
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType;
        use crate::sprite::MotionState;

        let helper_direction = self
            .get_entity(dismount.helper_id)
            .unwrap_or_else(|| {
                panic!(
                    "shoulder-dismount helper {:?} vanished during Execute",
                    dismount.helper_id
                )
            })
            .element_data()
            .direction();
        let carried_direction = (helper_direction + 8) & 15;

        if dismount.initialising {
            // FreezeExecution interrupts the rider's selected sequence. Its
            // cached installed order is the Rust mirror of Original's
            // detached mpOrder and must disappear at the same owner boundary.
            self.actor_freeze_execution(dismount.carried_id);
            if let Some(carried) = self.get_entity_mut(dismount.carried_id)
                && let Some(actor) = carried.actor_data_mut()
            {
                actor.installed_order = None;
            }
        }

        let Some(carried) = self.get_entity_mut(dismount.carried_id) else {
            // Original permits mpCarried to become null while the transition
            // runs and simply finishes the helper animation in that case.
            return;
        };
        carried
            .element_data_mut()
            .set_direction_goal(carried_direction);
        let carried_sprite_direction = u16::try_from(carried.element_data().direction())
            .expect("PC shoulder rider has a negative direction");
        let sprite = &mut carried.element_data_mut().sprite;
        sprite.force_sprite_row(
            OrderType::ClimbingDownFromShoulders,
            carried_sprite_direction,
        );
        sprite.synchronize_anim(dismount.helper_frame, dismount.helper_frame_count);
        sprite.display_order_ref = Some(dismount.helper_id);
        sprite.behind_display_order_ref = false;

        if dismount.motion == MotionState::Done {
            carried.set_posture(Posture::Upright);
            carried
                .actor_data_mut()
                .expect("PC has actor data")
                .action_state = ActionState::Waiting;
        }
        if dismount.motion != MotionState::Terminated {
            return;
        }

        let helper_position = self
            .get_entity(dismount.helper_id)
            .expect("shoulder-dismount helper vanished before termination")
            .element_data()
            .position_map();
        let helper_current_point = self
            .get_entity(dismount.helper_id)
            .expect("shoulder-dismount helper vanished before landing search")
            .cxx_current_point_map()
            .unwrap_or_else(|| {
                panic!(
                    "shoulder-dismount helper {:?} has no current action point",
                    dismount.helper_id
                )
            });
        let helper_layer = self
            .get_entity(dismount.helper_id)
            .expect("shoulder-dismount helper vanished before termination")
            .element_data()
            .layer();
        let landing_position = {
            let carried_box = self
                .get_entity(dismount.carried_id)
                .expect("shoulder rider vanished before landing search")
                .position_iface()
                .get_move_box()
                .to_owned();
            if carried_box.is_somewhere() {
                // Original translates the upright rider box from the
                // helper's live animation hotspot (`GetCurrentPointMap`),
                // while using the helper's map origin as the directional
                // reference for the three-argument authorization search.
                let mut box_at_helper = carried_box.translated(helper_current_point);
                if self.world.fast_grid.find_authorized_position_toward(
                    &mut box_at_helper,
                    helper_position,
                    helper_layer,
                ) {
                    box_at_helper.center()
                } else {
                    helper_position
                }
            } else {
                helper_position
            }
        };

        if let Some(carried) = self.get_entity_mut(dismount.carried_id) {
            carried
                .element_data_mut()
                .set_position_map_delayed(landing_position);
            carried.set_posture(Posture::Upright);
            // RHElementActorHuman::SetCarrier(NULL) restores the old
            // carrier's heading as the released rider's direction goal
            // before clearing the back-reference
            // (RHelementactorhuman.cpp:5990-6017).
            carried
                .element_data_mut()
                .set_direction_goal(helper_direction);
            if let Some(human) = carried.human_data_mut() {
                human.carrier = None;
            }
            if let Some(actor) = carried.actor_data_mut() {
                actor.execution_frozen = false;
                actor.action_state = ActionState::Waiting;
            }
            let sprite = &mut carried.element_data_mut().sprite;
            sprite.display_order_ref = None;
            sprite.behind_display_order_ref = false;
        }
        if let Some(helper) = self.get_entity_mut(dismount.helper_id)
            && let Some(pc) = helper.pc_data_mut()
        {
            pc.carried = None;
            pc.set_live_carried_posture(Posture::Lying);
        }
        // Original invokes mpCarried->Wait(), not helper->Wait(), before
        // releasing the final carrier/carried references.
        self.actor_wait(dismount.carried_id);
    }

    /// Post-animation hook that drains outcomes collected by
    /// [`EngineInner::tick_actor_animation_for`] for non-`EventDone`
    /// completion variants.
    ///
    /// - `seq_terminate`: terminate the associated sequence element
    ///   (Turn / any plain `SequenceElement` booking).
    /// - `unlock_door_done`: clear all live door lock/authorization flags at
    ///   the lockpick action point. The later termination edge advances the
    ///   sequence through the ordinary `seq_advance` path.
    /// - `resume_door_pass`: re-enter `advance_door_pass` for the actor
    ///   so the next step in the door-pass chain (PassingDoor trigger,
    ///   next Walk step, or Done) can fire.
    pub(super) fn process_anim_completion_outcomes(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        outcomes: super::animation::AnimCompletionOutcomes,
        assets: &LevelAssets,
    ) {
        let super::animation::AnimCompletionOutcomes {
            non_interruptable_lifts,
            seq_advance,
            seq_terminate,
            seq_impossible,
            wasp_next_cycle,
            unlock_door_done,
            resume_door_pass,
            select_hulk,
            next_jump_step,
            play_anim_frozen,
            corpse_drop_done,
            shoulder_carried_waits,
            shoulder_helper_dismounts,
            execute_sides,
        } = outcomes;
        let super::animation::ExecuteSideOutcomes {
            rejected_dead_idle_posture_requests,
            waiting_upright,
            waiting_alerted,
            drop_ale_done,
            deactivate_entities,
            pickups,
            drink_done,
            wasp_sting_remark,
            special_remark,
            weak_stunned_start,
            pickpockets,
            pc_target_activations,
            cry_for_help_under_net,
            smalltalk_strikes,
            killed_at_bottom,
            waking_up_done,
            hidden_titbit_removals,
            beggar_coin_flags,
            beggar_wait_handoffs,
            stature_change_end,
            pc_bow_equip_action,
            pc_bow_unequip_action,
            pc_helping_climb_action,
        } = execute_sides;

        // The drain order below reproduces the completion-callback order the
        // engine has always used; it is part of the parity contract. Do not
        // reorder these calls.
        self.drain_non_interruptable_lifts(non_interruptable_lifts);
        self.drain_corpse_drop_done(assets, corpse_drop_done);
        for carried_id in shoulder_carried_waits {
            self.actor_wait(carried_id);
        }
        for dismount in shoulder_helper_dismounts {
            self.apply_helper_driven_shoulder_dismount(dismount);
        }
        self.drain_seq_advance(seq_advance);
        self.drain_wasp_next_cycle(wasp_next_cycle);
        self.drain_seq_terminate(seq_terminate);
        self.drain_play_anim_frozen(play_anim_frozen);
        self.drain_seq_impossible(seq_impossible);
        self.drain_unlock_door_done(unlock_door_done);
        self.drain_next_jump_step(assets, next_jump_step);
        self.drain_select_hulk(select_hulk);
        self.drain_resume_door_pass(sim, assets, resume_door_pass);
        for entity_id in rejected_dead_idle_posture_requests {
            self.process_rejected_nonlying_posture_request_for(entity_id);
        }
        self.drain_waiting_upright(waiting_upright);
        self.drain_waiting_alerted(sim, assets, waiting_alerted);
        // Soldier `Execute` cross-entity side effects, collected by the
        // animation tick as it walks each `active_ai_anim` booking. Each
        // drain fires a cross-entity effect (bottle hide, coin pickup,
        // remarks, blood-alcohol bump).
        self.drain_drop_ale_done(assets, drop_ale_done);
        self.drain_pc_bow_equip_action(assets, pc_bow_equip_action);
        self.drain_pc_bow_unequip_action(assets, pc_bow_unequip_action);
        self.drain_pc_helping_climb_action(assets, pc_helping_climb_action);
        self.drain_stature_change_end(stature_change_end);
        self.drain_weak_stunned_start(sim, assets, weak_stunned_start);
        self.drain_hidden_titbit_removals(hidden_titbit_removals);
        self.drain_beggar_wait_handoffs(sim, assets, beggar_wait_handoffs);
        self.drain_beggar_coin_flags(beggar_coin_flags);
        self.drain_smalltalk_strikes(sim, assets, smalltalk_strikes);
        self.drain_killed_at_bottom(killed_at_bottom);
        self.drain_deactivate_entities(deactivate_entities);
        self.drain_pc_target_activations(pc_target_activations);
        self.drain_waking_up_done(sim, assets, waking_up_done);
        self.drain_pickups(sim, assets, pickups);
        self.drain_drink_done(assets, drink_done);
        self.drain_pickpockets(pickpockets);
        self.drain_wasp_sting_remark(sim, assets, wasp_sting_remark);
        self.drain_special_remark(sim, assets, special_remark);
        self.drain_cry_for_help_under_net(sim, assets, cry_for_help_under_net);
    }

    fn advance_mission_clock(&mut self) {
        self.control.frame_counter += 1;
        if self.control.frame_counter.is_multiple_of(FRAMES_PER_SECOND)
            && let Some(campaign) = Some(&mut self.mission_domain.campaign)
        {
            campaign.add_value(crate::campaign::CampaignValue::MissionLength, 1);
        }
    }
}

/// Insert randomised midpoint detours into a pathfinder-returned
/// waypoint list (drunken soldier post-process path).
///
/// Walks the waypoint list in passes (one pass per
/// `blood_alcohol / increment` increments) and for every segment
/// tries up to 3 random deviation vectors; the first reachable one
/// gets inserted as a new intermediate waypoint.  Running soldiers
/// use a lower increment + factor (they don't wobble as much per
/// step) than walking soldiers.
///
/// The RNG is drained deterministically from the explicit caller context, so
/// replays reproduce the same deviation sequence. Original provenance:
/// `RHElementActorSoldier::PostProcessPath` in
/// `original-code/RHelementactorsoldier.cpp:1688-1771` uses two draws for
/// each of up to three candidate deviations per segment.
#[inline]
fn drunken_deviation_direction(direction: i16) -> [f32; 2] {
    // SBGeoVector2D::SetSector0to15(direction, ASPECT_RATIO) compresses the
    // table direction's Y component back into isometric map space.
    crate::position_interface::sector_to_vector_iso(direction)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_drunken_path_deviation(
    sim: &crate::sim_rng::SimulationContext,

    mut waypoints: Vec<crate::coordinates::MapPoint>,
    origin: crate::coordinates::MapPoint,
    blood_alcohol: u8,
    is_running: bool,
    layer: u16,
    move_box: &crate::coordinates::MoveBox,
    half_diagonal: crate::coordinates::MoveBoxHalfDiagonal,
    grid: &crate::fast_find_grid::FastFindGrid,
) -> Vec<crate::coordinates::MapPoint> {
    const DRUNKEN_DEVIATION_FACTOR: f32 = 0.03;

    // Max of (30, blood_alcohol) — the minimum ensures even mildly
    // tipsy soldiers still show some wobble.
    let clamped_ba = blood_alcohol.max(30) as f32;
    let (factor, increment) = if is_running {
        (0.003 * clamped_ba, 60u8)
    } else {
        (0.01 * clamped_ba, 30u8)
    };

    let mut iterator = 0u8;
    while iterator < blood_alcohol {
        let mut new_path: Vec<crate::coordinates::MapPoint> =
            Vec::with_capacity(waypoints.len() * 2);
        let mut prev = origin;
        for next in &waypoints {
            let straight = crate::coordinates::MapVec::new(next.x - prev.x, next.y - prev.y);
            let max_norm = straight.x.abs().max(straight.y.abs());
            // Midpoint of the current segment.
            let midpoint = crate::coordinates::MapPoint::new(
                prev.x + 0.5 * straight.x,
                prev.y + 0.5 * straight.y,
            );
            let mut inserted: Option<crate::coordinates::MapPoint> = None;
            for _try in 0..3 {
                // `rand() & 15` — pick a random 16-sector direction
                // and scale by another 0..15 random magnitude.
                let dir_sector =
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DrunkenPathDeviation, 0..16)
                        as i16;
                let magnitude =
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DrunkenPathDeviation, 0..16)
                        as f32;
                let [dx, dy] = drunken_deviation_direction(dir_sector);
                let scale = magnitude * max_norm * DRUNKEN_DEVIATION_FACTOR * factor;
                let candidate = crate::coordinates::MapPoint::new(
                    midpoint.x + dx * scale,
                    midpoint.y + dy * scale,
                );
                if grid.is_straight_movement_authorized(prev, candidate, layer, move_box)
                    && grid.is_reachable_thick(candidate, *next, layer, half_diagonal)
                {
                    inserted = Some(candidate);
                    break;
                }
            }
            if let Some(ip) = inserted {
                new_path.push(ip);
            }
            new_path.push(*next);
            prev = *next;
        }
        waypoints = new_path;
        iterator = iterator.saturating_add(increment);
    }

    waypoints
}

/// Original soldier post-processing runs after actor path post-processing has
/// already inserted startup/end transitions. Walk only the remaining upright
/// movement orders and insert deviated copies immediately before them, leaving
/// transition geometry untouched.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_drunken_order_deviation(
    sim: &crate::sim_rng::SimulationContext,
    element: &mut crate::sequence::SequenceElement,
    origin: crate::coordinates::MapPoint,
    blood_alcohol: u8,
    is_running: bool,
    layer: u16,
    move_box: &crate::coordinates::MoveBox,
    half_diagonal: crate::coordinates::MoveBoxHalfDiagonal,
    grid: &crate::fast_find_grid::FastFindGrid,
    next_order_id: &mut u32,
) {
    const DRUNKEN_DEVIATION_FACTOR: f32 = 0.03;

    let clamped_ba = blood_alcohol.max(30) as f32;
    let (factor, increment) = if is_running {
        (0.003 * clamped_ba, 60usize)
    } else {
        (0.01 * clamped_ba, 30usize)
    };
    let passes = usize::from(blood_alcohol).div_ceil(increment);

    insert_drunken_orders_with(element, origin, passes, next_order_id, |first, second| {
        let straight = crate::coordinates::MapVec::new(second.x - first.x, second.y - first.y);
        let max_norm = straight.x.abs().max(straight.y.abs());
        let midpoint = crate::coordinates::MapPoint::new(
            first.x + 0.5 * straight.x,
            first.y + 0.5 * straight.y,
        );
        for _try in 0..3 {
            let dir_sector =
                crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DrunkenPathDeviation, 0..16)
                    as i16;
            let magnitude =
                crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DrunkenPathDeviation, 0..16)
                    as f32;
            let [dx, dy] = drunken_deviation_direction(dir_sector);
            let scale = magnitude * max_norm * DRUNKEN_DEVIATION_FACTOR * factor;
            let candidate =
                crate::coordinates::MapPoint::new(midpoint.x + dx * scale, midpoint.y + dy * scale);
            if grid.is_straight_movement_authorized(first, candidate, layer, move_box)
                && grid.is_reachable_thick(candidate, second, layer, half_diagonal)
            {
                return Some(candidate);
            }
        }
        None
    });
}

fn insert_drunken_orders_with(
    element: &mut crate::sequence::SequenceElement,
    origin: crate::coordinates::MapPoint,
    passes: usize,
    next_order_id: &mut u32,
    mut candidate_for_segment: impl FnMut(
        crate::coordinates::MapPoint,
        crate::coordinates::MapPoint,
    ) -> Option<crate::coordinates::MapPoint>,
) {
    for _ in 0..passes {
        let mut first = origin;
        let mut order_index = 0usize;
        while order_index < element.orders.len() {
            let order = &element.orders[order_index];
            if !matches!(
                order.order_type,
                crate::order::OrderType::WalkingUpright | crate::order::OrderType::RunningUpright
            ) {
                order_index += 1;
                continue;
            }

            let second = crate::coordinates::MapPoint::new(order.target_x, order.target_y);
            if let Some(candidate) = candidate_for_segment(first, second) {
                // C++ constructs `new RHOrder(*pOrder)`: all movement
                // metadata is copied, while the inserted order receives a
                // fresh identity and its midpoint destination.
                let mut inserted = order.clone();
                inserted.reseed_id(crate::order::alloc_order_id(next_order_id));
                inserted.target_x = candidate.x;
                inserted.target_y = candidate.y;
                element.insert_order(order_index, inserted);
                order_index += 1;
            }
            first = second;
            order_index += 1;
        }
    }
}

// ─── Titbit update query ─────────────────────────────────────────

#[cfg(test)]
mod drunken_path_deviation_tests {
    use super::{drunken_deviation_direction, insert_drunken_orders_with};

    #[test]
    fn deviation_direction_uses_original_isometric_aspect_ratio() {
        let direction = 2;
        let (raw_x, raw_y) = crate::element_kinds::direction_vector_16(direction);
        let [x, y] = drunken_deviation_direction(direction);

        assert_eq!(x, raw_x);
        assert_eq!(y, raw_y * crate::position_interface::ASPECT_RATIO);
        assert_ne!(y, raw_y, "the bare compass vector overextends map-space Y");
    }

    #[test]
    fn drunken_midpoint_follows_startup_transition_without_reheading_it() {
        let mut element = crate::sequence::SequenceElement::new_movement(
            1,
            crate::element::Command::MoveOk,
            None,
            crate::order::OrderType::WalkingUpright,
        );
        element.push_order(crate::order::Order::new(
            crate::order::OrderType::TransitionWaitingUprightWalkingUpright,
            0.0,
            -4.0,
            std::num::NonZeroU32::new(10).unwrap(),
        ));
        element.push_order(crate::order::Order::new(
            crate::order::OrderType::WalkingUpright,
            0.0,
            -40.0,
            std::num::NonZeroU32::new(11).unwrap(),
        ));
        let mut next_order_id = 20;

        insert_drunken_orders_with(
            &mut element,
            crate::coordinates::MapPoint::ZERO,
            1,
            &mut next_order_id,
            |first, second| {
                Some(crate::coordinates::MapPoint::new(
                    (first.x + second.x) * 0.5 + 3.0,
                    (first.y + second.y) * 0.5,
                ))
            },
        );

        assert_eq!(element.orders.len(), 3);
        assert_eq!(
            element.orders[0].order_type,
            crate::order::OrderType::TransitionWaitingUprightWalkingUpright
        );
        assert_eq!(
            (element.orders[0].target_x, element.orders[0].target_y),
            (0.0, -4.0),
            "actor transition geometry was fixed before soldier drunken post-processing"
        );
        assert_eq!(
            (element.orders[1].target_x, element.orders[1].target_y),
            (3.0, -20.0)
        );
        assert_eq!(
            (element.orders[2].target_x, element.orders[2].target_y),
            (0.0, -40.0)
        );
        assert_ne!(element.orders[1].order_id, element.orders[2].order_id);
    }
}

/// Real implementation of [`crate::titbit::TitbitUpdateQuery`] that
/// queries live entity state.  Replaces the old `StubQuery` that kept
/// all titbits alive unconditionally.
struct EntityTitbitQuery<'a> {
    sim: &'a crate::sim_rng::SimulationContext,
    entities: &'a crate::entities::Entities,
    sequence_manager: &'a crate::sequence::SequenceManager,
    follow_element: Option<EntityId>,
}

impl crate::titbit::TitbitUpdateQuery for EntityTitbitQuery<'_> {
    /// True when the entity should keep its weak-stunned titbit.
    ///
    /// - Soldiers in `WonderingAppleSauceInTheVisor` always keep stars.
    /// - Otherwise, stars stay only while the current animation is
    ///   `BeingWeakSword` or `BeingStunnedSword`.
    fn is_weak_or_stunned(&self, element: crate::titbit::ElementHandle) -> bool {
        use crate::ai::Substate;
        use crate::order::OrderType;

        let Some(entity_id) = self.entities.id_at_legacy_slot(element.0) else {
            return false;
        };
        let Some(entity) = self.entities.get(entity_id) else {
            return false;
        };

        // Soldiers in apple-sauce substate keep stars unconditionally.
        if let Entity::Soldier(s) = entity
            && s.npc.ai_substate() == Substate::WonderingAppleSauceInTheVisor
        {
            return true;
        }

        // Otherwise, check if the current animation is weak/stunned sword.
        // Orders live on the owning `SequenceElement.orders` now —
        // look up via the actor's current in-progress element.
        matches!(
            self.sequence_manager
                .current_order_for_actor(entity_id)
                .map(|(_, _, o)| o.order_type),
            Some(OrderType::BeingWeakSword | OrderType::BeingStunnedSword)
        )
    }

    fn is_unconscious_and_alive(&self, element: crate::titbit::ElementHandle) -> bool {
        let Some(entity_id) = self.entities.id_at_legacy_slot(element.0) else {
            return false;
        };
        let Some(entity) = self.entities.get(entity_id) else {
            return false;
        };
        match entity {
            Entity::Pc(pc) => pc.human.unconscious && pc.pc.life_points > 0,
            Entity::Soldier(s) => s.human.unconscious && s.npc.life_points > 0,
            Entity::Civilian(c) => c.human.unconscious && c.npc.life_points > 0,
            _ => false,
        }
    }

    fn is_follow_element(&self, element: crate::titbit::ElementHandle) -> bool {
        // The entity the camera is currently locked onto (via
        // `SelectFollowElement` / `LockCameraOn`).
        self.follow_element
            .is_some_and(|id| id.index() == element.0)
    }

    fn is_hidden_posture(&self, element: crate::titbit::ElementHandle) -> bool {
        use crate::element::Posture;
        let Some(entity_id) = self.entities.id_at_legacy_slot(element.0) else {
            return false;
        };
        let Some(entity) = self.entities.get(entity_id) else {
            return false;
        };
        matches!(
            entity.element_data().posture,
            Posture::Spy | Posture::Tree | Posture::AnonymousArcher
        )
    }

    fn random_u32(&self) -> u32 {
        crate::sim_rng::u32(self.sim, crate::sim_rng::RngSite::TitbitUpdate, ..)
    }
}

#[cfg(test)]
mod bow_command_body_parity_tests {
    use super::*;
    use crate::element::{
        ActionState, ActorData, ActorPc, ActorSoldier, ElementData, ElementKind, Entity, HumanData,
        NpcData, PcData, Posture, SoldierData,
    };
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceId, SequenceState};

    fn make_aiming_pc(action_state: ActionState) -> Entity {
        Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData {
                action_state,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    fn launch_bow_command_and_tick(command: Command, action_state: ActionState) -> EngineInner {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let pc_id = engine.add_entity(make_aiming_pc(action_state));
        engine.launch_element(SequenceElement::new(1, command, Some(pc_id)));

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        engine
    }

    fn command_order_types(engine: &EngineInner) -> Vec<OrderType> {
        engine
            .orders
            .sequence_manager
            .get_element(SequenceId(1), 0)
            .unwrap()
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect()
    }

    fn make_bow_soldier(posture: Posture, action_state: ActionState) -> Entity {
        Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                posture,
                ..ElementData::default()
            },
            actor: ActorData {
                action_state,
                ..ActorData::default()
            },
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        })
    }

    fn install_test_lift_sector(
        engine: &mut EngineInner,
        owner: EntityId,
        sector_number: crate::sector::SectorNumber,
    ) {
        engine
            .world
            .entities
            .get_mut(owner)
            .expect("test lift owner exists")
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(0));
        let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
        level.sector_number_map.insert(sector_number, 0);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::LIFT,
            layer: 0,
            sector_number,
            door_index: None,
            lift_type: Some(crate::sector::LiftType::Ladder),
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
    }

    #[test]
    fn bow_lean_out_commands_keep_transition_order_live() {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let soldier_id = engine.add_entity(make_bow_soldier(
            Posture::Upright,
            ActionState::AimingWithBow,
        ));
        let seq_id = engine.launch_element(SequenceElement::new(
            1,
            Command::LowerBowLeanOut,
            Some(soldier_id),
        ));

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

        let elem = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(
            elem.state,
            SequenceState::InProgress,
            "C++ LOWER_BOW_LEAN_OUT keeps its translated transition order live"
        );
        assert_eq!(
            elem.current_order().map(|order| order.order_type),
            Some(OrderType::TransitionLoweringBowLeaningOut)
        );
    }

    #[test]
    fn equip_bow_terminates_when_actor_is_already_aiming() {
        let engine = launch_bow_command_and_tick(Command::EquipBow, ActionState::AimingWithBow);
        let elem = engine
            .orders
            .sequence_manager
            .get_element(SequenceId(1), 0)
            .unwrap();

        assert_eq!(elem.state, SequenceState::Terminated);
        assert!(
            elem.orders.is_empty(),
            "redundant EquipBow must not queue equip/load orders"
        );
    }

    #[test]
    fn pre_timer_condolation_starts_successor_timer_before_the_scan() {
        use crate::sequence::{Field, FieldValue, Sequence};

        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let pc_id = engine.add_entity(make_aiming_pc(ActionState::AimingWithBow));
        let mut sequence = Sequence::new();
        sequence.append_element(SequenceElement::new(1, Command::EquipBow, Some(pc_id)));
        let mut timer = SequenceElement::new_generic(2, Command::Timer, None);
        timer.set_property(Field::Timer, FieldValue::Integer(2));
        sequence.append_element(timer);
        engine.orders.sequence_manager.launch_sequence(sequence);

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

        assert_eq!(engine.orders.timer_elements.len(), 1);
        assert_eq!(
            engine.orders.timer_elements[0].remaining, 1,
            "the immediate Timer successor must launch before the same frame's timer scan"
        );
    }

    #[test]
    fn timer_expiry_condolation_starts_successor_after_the_scan() {
        use crate::sequence::{Field, FieldValue, Sequence};

        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let pc_id = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let mut sequence = Sequence::new();
        let mut expiring = SequenceElement::new_generic(1, Command::Timer, Some(pc_id));
        expiring.set_property(Field::Timer, FieldValue::Integer(1));
        sequence.append_element(expiring);
        let mut successor = SequenceElement::new_generic(2, Command::Timer, None);
        successor.set_property(Field::Timer, FieldValue::Integer(2));
        sequence.append_element(successor);
        engine.orders.sequence_manager.launch_sequence(sequence);

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

        assert_eq!(engine.orders.timer_elements.len(), 1);
        assert_eq!(
            engine.orders.timer_elements[0].remaining, 2,
            "a successor launched by timer expiry belongs to the final condolation drain and must not re-enter the timer scan in progress"
        );
    }

    #[test]
    fn equip_bow_down_terminates_when_actor_is_already_aiming_up() {
        let engine =
            launch_bow_command_and_tick(Command::EquipBowDown, ActionState::AimingWithBowUp);
        let elem = engine
            .orders
            .sequence_manager
            .get_element(SequenceId(1), 0)
            .unwrap();

        assert_eq!(elem.state, SequenceState::Terminated);
        assert!(
            elem.orders.is_empty(),
            "redundant EquipBowDown must not queue equip/load/lower orders"
        );
    }

    #[test]
    fn raise_bow_from_waiting_queues_equip_load_then_raise() {
        let engine = launch_bow_command_and_tick(Command::RaiseBow, ActionState::Waiting);

        assert_eq!(
            command_order_types(&engine),
            vec![
                OrderType::TransitionEquipBow,
                OrderType::TransitionLoadingBow,
                OrderType::TransitionRaisingBow,
            ],
            "C++ TestBowAimUp expects RaiseBow from waiting to equip, load, then raise"
        );
    }

    #[test]
    fn unequip_bow_from_aiming_up_queues_lower_unload_then_unequip() {
        let engine = launch_bow_command_and_tick(Command::UnequipBow, ActionState::AimingWithBowUp);

        assert_eq!(
            command_order_types(&engine),
            vec![
                OrderType::TransitionLoweringBow,
                OrderType::TransitionUnloadBow,
                OrderType::TransitionUnequipBow,
            ],
            "C++ TestBowAimUp expects UnequipBow from bow-up to lower, unload, then unequip"
        );
    }

    #[test]
    fn turn_context_sets_goal_without_snapping_and_books_turning() {
        use crate::sequence::{Field, FieldValue};

        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let mut turn = SequenceElement::new_generic(1, Command::Turn, Some(owner));
        turn.set_property(Field::Direction, FieldValue::Integer(5));
        let seq_id = engine.orders.sequence_manager.launch_element(turn);

        let barrier = TurnCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
        }
        .dispatch(owner, Command::Turn, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        let entity = engine.world.entities.get(owner).unwrap();
        assert_eq!(entity.element_data().direction(), 0);
        assert_eq!(
            u8::from(
                entity
                    .element_data()
                    .sprite
                    .position_iface
                    .get_direction_goal()
            ),
            5,
            "Turn must set the progressive direction goal, not snap direction"
        );
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::Turning)
        );
        assert!(
            !element.current_order().unwrap().compute_direction,
            "Turn translation already resolved the direction goal and must not recompute it from the order's dummy point"
        );
    }

    #[test]
    fn wait_timer_context_arms_actor_and_books_upright_idle() {
        use crate::sequence::{Field, FieldValue};

        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let mut wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
        wait.set_property(Field::Timer, FieldValue::Integer(7));
        let seq_id = engine.orders.sequence_manager.launch_element(wait);

        let barrier = WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitTimer, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        assert_eq!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .wait_time,
            7
        );
        assert_eq!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .seek_refresh_wait,
            7,
            "WAIT_TIMER writes Original's shared mulWaitTime, so every Rust storage mirror must retain the same value across interruption"
        );
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::WaitingUprightBored)
        );
        assert!(!element.current_order().unwrap().compute_direction);

        // A timer may interrupt a seek while the actor-owned post-seek
        // pointers remain dormant. Once the timer itself is interrupted and
        // the actor falls back to Wait, the parity view must still expose the
        // last value written to Original's one shared mulWaitTime scalar.
        {
            let actor = engine
                .world
                .entities
                .get_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.seek_target = Some(owner);
            actor.post_seek_sequence = Some(crate::sequence::Sequence::new().into_post_seek());
        }
        engine.orders.sequence_manager.element_interrupted(
            seq_id,
            0,
            crate::sequence::CascadeFlags::NEXT_LEVEL,
        );
        let mut idle = SequenceElement::new(1, Command::Wait, Some(owner));
        idle.priority = crate::sequence::SequencePriority::Wait;
        let idle_sequence = engine.orders.sequence_manager.launch_element(idle);
        engine
            .orders
            .sequence_manager
            .element_in_progress(idle_sequence, 0);
        assert_eq!(engine.actor_legacy_wait_time(owner), 7);

        // Savegame_linux3/Profile_003/Savegame_065 replay-003 frame
        // 16245: a long jump starts while these post-seek pointers remain
        // retained. Original's airborne Execute arm overwrites mulWaitTime
        // with the flight duration, so that live owner must take precedence
        // over the dormant seek-refresh copy.
        {
            use crate::engine::jump::{ActiveJump, CurrentStepState, JumpStep};
            use crate::sequence::SequenceId;
            use std::collections::VecDeque;
            use std::num::NonZeroU32;

            let actor = engine
                .world
                .entities
                .get_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.wait_time = 4;
            actor.seek_refresh_wait = 0;
            actor.active_jump = Some(ActiveJump {
                steps: VecDeque::new(),
                current: Some(CurrentStepState {
                    start_x: 0.0,
                    start_y: 0.0,
                    start_z: 0.0,
                    total_frames: 5,
                    frames_elapsed: 1,
                    order_id: NonZeroU32::new(1).unwrap(),
                    airborne_increment: None,
                    step: JumpStep {
                        anim: OrderType::JumpingLong,
                        target_3d: None,
                        airborne: true,
                        max_frames: None,
                    },
                }),
                sequence_id: SequenceId(1),
                element_index: 0,
                dest_sector: None,
                dest_layer: 0,
                source_direction_goal: 0,
                dest_projection_point: crate::coordinates::MapPoint::default(),
            });
        }
        assert_eq!(engine.actor_legacy_wait_time(owner), 4);
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_jump
            .as_mut()
            .unwrap()
            .current
            .as_mut()
            .unwrap()
            .step
            .airborne = false;
        assert_eq!(engine.actor_legacy_wait_time(owner), 0);
    }

    #[test]
    fn ladder_fall_wait_owns_legacy_scalar_over_dormant_seek() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Moving));
        let actor = engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();

        // A swordstrike post-seek may remain attached while a non-interruptible
        // ladder/wall fall runs. Original's FallingLadderWall Execute arm owns
        // the single mulWaitTime scalar for the flight countdown in this state.
        actor.seek_target = Some(owner);
        actor.post_seek_sequence = Some(crate::sequence::Sequence::new().into_post_seek());
        actor.seek_refresh_wait = 0;
        actor.wait_time = 2;
        actor.active_flight = Some(crate::element::ActiveFlight {
            frames_remaining: 2,
            ladder_fall: true,
            ..crate::element::ActiveFlight::default()
        });

        assert_eq!(engine.actor_legacy_wait_time(owner), 2);
    }

    #[test]
    fn frozen_all_wait_timer_still_completes_in_owner_slot() {
        use crate::sequence::{Field, FieldValue};

        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let mut wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
        wait.set_property(Field::Timer, FieldValue::Integer(0));
        let seq_id = engine.orders.sequence_manager.launch_element(wait);
        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitTimer, seq_id, 0);
        let _ = engine
            .orders
            .sequence_manager
            .take_pending_synchronous_actions();
        engine.set_actors_frozen(true);

        engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .expect("wait timer remains inspectable")
                .state,
            SequenceState::Terminated
        );
    }

    #[test]
    fn wait_timer_wraps_beggar_execute_and_generic_execute_once_each() {
        fn run_once(order_type: OrderType, wait_time: u32) -> (u32, SequenceState) {
            let mut engine = EngineInner::new();
            let assets = LevelAssets::new();
            let mut owner_entity = make_aiming_pc(ActionState::Waiting);
            let mut conversion =
                vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
            conversion[order_type as usize] = 0;
            owner_entity.element_data_mut().sprite = crate::sprite::Sprite::new(
                std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                    action_id: order_type as u16,
                    action_done: 0,
                    frame_ids: vec![1],
                    delays: vec![10],
                    distances: vec![0],
                    offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                    sound_ids: vec![0],
                    ..Default::default()
                }]),
                std::sync::Arc::new(conversion),
            );
            owner_entity.actor_data_mut().unwrap().wait_time = wait_time;
            owner_entity.actor_data_mut().unwrap().seek_refresh_wait = wait_time;
            let owner = engine.add_entity(owner_entity);

            let mut wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
            wait.priority = crate::sequence::SequencePriority::Normal;
            let seq_id = engine.orders.sequence_manager.launch_element(wait);
            engine
                .orders
                .sequence_manager
                .element_in_progress(seq_id, 0);
            let order_id = engine.orders.allocate_order_id();
            engine.orders.sequence_manager.push_order_on(
                seq_id,
                0,
                crate::order::Order::new(order_type, 0.0, 0.0, order_id),
            );

            engine
                .tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);
            let remaining = engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .wait_time;
            let state = engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .expect("WAIT_TIMER remains inspectable")
                .state;
            (remaining, state)
        }

        // Savegame_linux2/Profile_002/Savegame_032 replay-008 frame
        // 17705 enters with WAIT_TIMER=23 and SIMULATING_BEGGAR selected.
        // Original's base Actor::Hourglass decrements it after the PC
        // override returns; the specialized Rust arm used to skip that base
        // modifier and retain 23.
        assert_eq!(run_once(OrderType::SimulatingBeggar, 23).0, 22);
        assert_eq!(
            run_once(OrderType::WaitingUpright, 23).0,
            22,
            "the generic Execute path must retain its single decrement"
        );
        assert_eq!(
            run_once(OrderType::SimulatingBeggar, 0).1,
            SequenceState::Terminated,
            "the specialized Execute result must carry WAIT_TIMER termination into base completion"
        );
    }

    #[test]
    fn lazy_wait_publishes_start_before_preexisting_owner_instruction() {
        use crate::sequence::SequenceAction;

        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let mut owner_entity = make_aiming_pc(ActionState::Moving);
        let mut conversion =
            vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
        conversion[OrderType::TransitionWalkingUprightWaitingUpright as usize] = 0;
        conversion[OrderType::WalkingUpright as usize] = 1;
        conversion[OrderType::WaitingUpright as usize] = 2;
        let script = |action: OrderType| crate::sprite_script::SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        owner_entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![
                script(OrderType::TransitionWalkingUprightWaitingUpright),
                script(OrderType::WalkingUpright),
                script(OrderType::WaitingUpright),
            ]),
            std::sync::Arc::new(conversion),
        );
        owner_entity.element_data_mut().sprite.current_row = 1;
        owner_entity.element_data_mut().sprite.last_action = OrderType::WalkingUpright;
        let owner = engine.add_entity(owner_entity);
        let parry_sequence =
            engine.launch_element(SequenceElement::new(1, Command::ParrySword, Some(owner)));
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(parry_sequence, 0)
                .expect("ParrySword remains queued for manager dispatch")
                .priority,
            crate::sequence::SequencePriority::NotYetSet,
            "LaunchSequenceElement must leave ordinary work unresolved until manager Instruct"
        );

        engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

        let sprite = &engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .element_data()
            .sprite;
        assert_eq!(
            sprite.last_action,
            OrderType::TransitionWalkingUprightWaitingUpright,
            "synchronous synthetic Wait must publish its transition START before later owner work"
        );
        assert_eq!(sprite.current_row, 0);
        assert_eq!(sprite.current_frame, 0);
        assert_eq!(sprite.frame_count, u16::MAX);
        assert!(
            engine
                .orders
                .sequence_manager
                .current_order_for_actor(owner)
                .is_some_and(|(_, _, order)| {
                    order.order_type == OrderType::TransitionWalkingUprightWaitingUpright
                }),
            "the transient Wait remains selected until deferred owner work is processed"
        );
        let pending = engine.orders.sequence_manager.hourglass();
        assert_eq!(pending.len(), 1);
        let pending_ids = pending
            .iter()
            .map(|action| match action {
                SequenceAction::InstructOwner {
                    owner: action_owner,
                    sequence_id,
                    element_index: 0,
                } if *action_owner == owner => *sequence_id,
                other => panic!("unexpected pending action after actor slot: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(pending_ids[0], parry_sequence);
    }

    #[test]
    fn owner_local_stop_movement_new_id_preserves_execute_start() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Moving));
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingUpright,
        );
        movement.priority = crate::sequence::SequencePriority::Normal;
        let sequence_id = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
        let entry_order_id = engine.orders.allocate_order_id();
        engine.orders.sequence_manager.push_order_on(
            sequence_id,
            0,
            crate::order::Order::new(OrderType::WalkingUpright, 20.0, 0.0, entry_order_id),
        );

        engine.tick_actor_animation_action_change_slots_with_hooks(
            &crate::sim_rng::test_context(),
            &assets,
            |_, _| {},
            |_, _| {},
            |engine, execute_owner, selected_movement, _, _, _, _| {
                assert_eq!(execute_owner, owner);
                assert!(selected_movement.is_some());
                engine
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap()
                    .element_data_mut()
                    .sprite
                    .last_motion_state = Some(crate::sprite::MotionState::Start);
                // A LINE_SCRIPT EnterZone callback can invoke StopActor here,
                // after Execute has produced START but before Actor::Hourglass
                // performs its completion projection.
                engine.stop_owner(owner, crate::sequence::SequencePriority::Script);
            },
            |_, _, _| {},
        );

        let actor = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap();
        assert_eq!(
            actor.continuation.motion_state,
            crate::sprite::MotionState::Start
        );
        let (_, _, rewritten) = engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .expect("stopped walking order remains selected as its transition");
        assert_eq!(
            rewritten.order_type,
            OrderType::TransitionWalkingUprightWaitingUpright
        );
        assert_ne!(rewritten.order_id, entry_order_id);
    }

    #[test]
    fn fresh_waypoint_start_advancing_to_older_stop_transition_is_in_progress() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::MovingFast));
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningUpright,
        );
        movement.priority = crate::sequence::SequencePriority::Normal;
        let sequence_id = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
        // PostProcessPath allocates its final transition before inserting
        // path waypoints ahead of it, so the waypoint has the newer ID.
        let transition_order_id = engine.orders.allocate_order_id();
        let waypoint_order_id = engine.orders.allocate_order_id();
        engine.orders.sequence_manager.push_order_on(
            sequence_id,
            0,
            crate::order::Order::new(OrderType::RunningUpright, 20.0, 0.0, waypoint_order_id),
        );
        engine.orders.sequence_manager.push_order_on(
            sequence_id,
            0,
            crate::order::Order::new(
                OrderType::TransitionRunningUprightWaitingUpright,
                20.0,
                0.0,
                transition_order_id,
            ),
        );

        engine.tick_actor_animation_action_change_slots_with_hooks(
            &crate::sim_rng::test_context(),
            &assets,
            |_, _| {},
            |_, _| {},
            |engine, execute_owner, selected_movement, _, _, _, _| {
                assert_eq!(execute_owner, owner);
                assert!(selected_movement.is_some());
                engine
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap()
                    .element_data_mut()
                    .sprite
                    .last_motion_state = Some(crate::sprite::MotionState::Start);
                engine.do_next_order(sequence_id, 0);
            },
            |_, _, _| {},
        );

        let actor = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap();
        assert_eq!(
            actor.continuation.motion_state,
            crate::sprite::MotionState::InProgress
        );
        let (_, _, successor) = engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .expect("pre-existing stop transition must remain selected");
        assert_eq!(successor.order_id, transition_order_id);
        assert_eq!(
            successor.order_type,
            OrderType::TransitionRunningUprightWaitingUpright
        );
    }

    #[test]
    fn npc_state_context_preserves_menace_order_and_reaches_splice_barrier() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::StartMenace, Some(owner)));

        let barrier = NpcStateCommandContext {
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
        }
        .dispatch(Command::StartMenace, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element
                .orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![
                OrderType::TransitionRaisingSword,
                OrderType::TransitionWaitingSwordMenacing,
            ]
        );
        assert!(element.orders.iter().all(|order| !order.compute_direction));
    }

    #[test]
    fn npc_attention_context_uses_alerted_look_and_reaches_splice_barrier() {
        let mut engine = EngineInner::new();
        let mut soldier_entity = make_bow_soldier(Posture::Upright, ActionState::Waiting);
        let Entity::Soldier(soldier) = &mut soldier_entity else {
            unreachable!();
        };
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        let owner = engine.add_entity(soldier_entity);
        engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::enemy_ai_mut)
            .expect("test soldier has enemy AI")
            .attentive = true;
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::LookLeft, Some(owner)));

        let barrier = NpcAttentionCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
        }
        .dispatch(owner, Command::LookLeft, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::LookingLeftAlerted)
        );
        assert!(!element.current_order().unwrap().compute_direction);
    }

    #[test]
    fn stealth_context_crouches_and_preserves_terminated_order() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::CrouchDown, Some(owner)));

        let barrier = StealthCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            titbit_manager: &mut engine.feedback.titbit_manager,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::CrouchDown, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        // Translation only appends the crouch transition order; the posture
        // and action-state snap happen when the transition animation reaches
        // DONE, so the element stays selected/in-progress and the actor is
        // untouched during dispatch.
        let entity = engine.world.entities.get(owner).unwrap();
        assert_eq!(entity.element_data().posture, Posture::Upright);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::Waiting
        );
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::TransitionCrouchingDown),
            "the crouch body only queues its transition order at translation time"
        );
    }

    #[test]
    #[should_panic(expected = "WAIT_TIMER owner")]
    fn wait_timer_context_rejects_missing_timer_contextually() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
        let seq_id = engine.orders.sequence_manager.launch_element(wait);

        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitTimer, seq_id, 0);
    }

    #[test]
    #[should_panic(expected = "Wait translation owner")]
    fn wait_context_rejects_stale_owner_contextually() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let wait = SequenceElement::new(1, Command::Wait, Some(owner));
        let seq_id = engine.orders.sequence_manager.launch_element(wait);
        engine.remove_entity(owner);

        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::Wait, seq_id, 0);
    }

    #[test]
    fn stealth_termination_splices_timer_successor_before_same_tick_scan() {
        use crate::sequence::{Field, FieldValue, Sequence};

        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        // Crouch bodies now stay live until their transition animation
        // completes, so use LeaveSpy — the stealth context still snaps the
        // posture and terminates it synchronously inside its dispatch slot.
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .element_data_mut()
            .posture = Posture::Spy;
        let mut sequence = Sequence::new();
        // Production launches LeaveSpy with the auto-leave helper, which
        // never reaches priority arbitration; preset the priority the way
        // an already-arbitrated element carries it.
        let mut leave = SequenceElement::new(1, Command::LeaveSpy, Some(owner));
        leave.priority = crate::sequence::SequencePriority::Normal;
        sequence.append_element(leave);
        let mut timer = SequenceElement::new_generic(2, Command::Timer, None);
        timer.set_property(Field::Timer, FieldValue::Integer(2));
        sequence.append_element(timer);
        engine.orders.sequence_manager.launch_sequence(sequence);

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

        assert_eq!(engine.orders.timer_elements.len(), 1);
        assert_eq!(
            engine.orders.timer_elements[0].remaining, 1,
            "the stealth context must reach the synchronous splice before the timer scan"
        );
    }

    #[test]
    fn direct_ability_context_starts_whistle_and_reaches_splice_barrier() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::WhistleCmd, Some(owner)));

        let barrier = DirectAbilityCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WhistleCmd, true, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::Whistling)
        );
        assert_eq!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .whistle_wait_time,
            25
        );
    }

    #[test]
    fn direct_ability_context_preserves_eat_no_ammo_skip_barrier() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::EatCmd, Some(owner)));

        let barrier = DirectAbilityCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::EatCmd, false, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Skip);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::Terminated);
        assert!(element.orders.is_empty());
    }

    #[test]
    fn direct_ability_context_preserves_missing_throw_target_skip_barrier() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::ThrowApple, Some(owner)));

        let barrier = DirectAbilityCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::ThrowApple, true, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Skip);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .state,
            SequenceState::Impossible
        );
    }

    #[test]
    fn position_assertion_context_interrupts_at_tolerance_boundary() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let mut assertion = SequenceElement::new_movement(
            1,
            Command::AssertPosition,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            destination,
            tolerance,
            ..
        } = &mut assertion.data
        {
            *destination = crate::coordinates::MapPoint::new(5.0, 0.0);
            *tolerance = 0.0;
        }
        let seq_id = engine.orders.sequence_manager.launch_element(assertion);

        let barrier = PositionAssertionContext {
            entities: &engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .dispatch(owner, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Skip);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted,
            "RHelementactor.cpp uses >= tolerance + 5 for the max-norm mismatch"
        );
    }

    #[test]
    fn position_assertion_context_accepts_nan_distance_like_original() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        engine
            .world
            .entities
            .get_mut(owner)
            .expect("test assertion owner")
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(f32::NAN, f32::NAN));
        let mut assertion = SequenceElement::new_movement(
            1,
            Command::AssertPosition,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            destination,
            tolerance,
            ..
        } = &mut assertion.data
        {
            *destination = crate::coordinates::MapPoint::new(362.0, 1535.0);
            *tolerance = 10.0;
        }
        let seq_id = engine.orders.sequence_manager.launch_element(assertion);

        let barrier = PositionAssertionContext {
            entities: &engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .dispatch(owner, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Skip);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .state,
            SequenceState::Terminated,
            "Original's `qNaN >= tolerance + 5` mismatch test is false"
        );
    }

    #[test]
    fn lift_wait_context_keeps_blocked_lift_in_progress_and_reaches_splice() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let sector_number = crate::sector::SectorNumber::new(42);
        install_test_lift_sector(&mut engine, owner, sector_number);
        engine.world.fast_grid_mut().lift_state_mut(0).wait_time = 2;
        let door = crate::gate::Door {
            door_type: crate::gate::DoorType::LiftHigh,
            sector_in: sector_number,
            ..crate::gate::Door::default()
        };
        let mut wait = SequenceElement::new_movement(
            1,
            Command::WaitFreeLift,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            gate_id, sector, ..
        } = &mut wait.data
        {
            *gate_id = Some(crate::gate::DoorIndex(0));
            *sector = crate::position_interface::SectorHandle::new(42);
        }
        let seq_id = engine.orders.sequence_manager.launch_element(wait);

        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitFreeLift, seq_id, 0);

        let authorized = LiftWaitCommandContext {
            entities: &mut engine.world.entities,
            fast_grid: std::sync::Arc::make_mut(&mut engine.world.fast_grid),
            doors: std::slice::from_ref(&door),
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .authorize_and_reserve(owner, seq_id, 0);

        assert!(!authorized);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .state,
            SequenceState::InProgress
        );
        assert_eq!(engine.world.fast_grid_mut().lift_state_mut(0).wait_time, 1);
        assert!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_lift
                .is_none()
        );
    }

    #[test]
    #[should_panic(expected = "must be LiftHigh or LiftLow")]
    fn lift_wait_context_rejects_crenel_lift_type_contextually() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let sector_number = crate::sector::SectorNumber::new(42);
        install_test_lift_sector(&mut engine, owner, sector_number);
        let door = crate::gate::Door {
            door_type: crate::gate::DoorType::LiftHighCrenel,
            sector_in: sector_number,
            ..crate::gate::Door::default()
        };
        let mut wait = SequenceElement::new_movement(
            1,
            Command::WaitFreeLift,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            gate_id, sector, ..
        } = &mut wait.data
        {
            *gate_id = Some(crate::gate::DoorIndex(0));
            *sector = crate::position_interface::SectorHandle::new(42);
        }
        let seq_id = engine.orders.sequence_manager.launch_element(wait);
        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitFreeLift, seq_id, 0);

        LiftWaitCommandContext {
            entities: &mut engine.world.entities,
            fast_grid: std::sync::Arc::make_mut(&mut engine.world.fast_grid),
            doors: std::slice::from_ref(&door),
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .authorize_and_reserve(owner, seq_id, 0);
    }

    #[test]
    fn lift_wait_context_reserves_direction_before_terminating() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let sector_number = crate::sector::SectorNumber::new(42);
        install_test_lift_sector(&mut engine, owner, sector_number);
        let door = crate::gate::Door {
            door_type: crate::gate::DoorType::LiftHigh,
            sector_in: sector_number,
            ..crate::gate::Door::default()
        };
        let mut wait = SequenceElement::new_movement(
            1,
            Command::WaitFreeLift,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            gate_id, sector, ..
        } = &mut wait.data
        {
            *gate_id = Some(crate::gate::DoorIndex(0));
            *sector = crate::position_interface::SectorHandle::new(42);
        }
        let seq_id = engine.orders.sequence_manager.launch_element(wait);

        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitFreeLift, seq_id, 0);

        let authorized = LiftWaitCommandContext {
            entities: &mut engine.world.entities,
            fast_grid: std::sync::Arc::make_mut(&mut engine.world.fast_grid),
            doors: std::slice::from_ref(&door),
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .authorize_and_reserve(owner, seq_id, 0);

        assert!(authorized);
        engine.do_next_order(seq_id, 0);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .state,
            SequenceState::Terminated
        );
        let lift = engine.world.fast_grid_mut().lift_state_mut(0);
        assert_eq!(lift.occupants, 1);
        assert!(lift.occupied_downwards);
        assert_eq!(lift.wait_time, 100);
        let active_lift = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_lift
            .expect("authorized actor records its active lift");
        assert_eq!(active_lift.sector_number, 42);
        assert!(!active_lift.upwards);
    }

    #[test]
    fn lift_wait_reservation_is_consumed_by_production_leave_callback() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let sector_number = crate::sector::SectorNumber::new(42);
        install_test_lift_sector(&mut engine, owner, sector_number);
        {
            let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
            let outside = crate::sector::SectorNumber::new(0);
            let outside_index = level.sectors.len();
            level.sector_number_map.insert(outside, outside_index);
            level.sectors.push(crate::fast_find_grid::GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
                layer: 0,
                sector_number: outside,
                door_index: None,
                lift_type: None,
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            });
        }
        let door = crate::gate::Door {
            door_type: crate::gate::DoorType::LiftHigh,
            sector_in: sector_number,
            sector_out: crate::sector::SectorNumber::new(0),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(0),
            sector_out_index: crate::fast_find_grid::SectorIndex::new(1),
            ..crate::gate::Door::default()
        };
        engine.script_domains.interactables.doors.push(door.clone());
        let mut wait = SequenceElement::new_movement(
            1,
            Command::WaitFreeLift,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            gate_id, sector, ..
        } = &mut wait.data
        {
            *gate_id = Some(crate::gate::DoorIndex(0));
            *sector = crate::position_interface::SectorHandle::new(42);
        }
        let seq_id = engine.orders.sequence_manager.launch_element(wait);
        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitFreeLift, seq_id, 0);

        assert!(
            LiftWaitCommandContext {
                entities: &mut engine.world.entities,
                fast_grid: std::sync::Arc::make_mut(&mut engine.world.fast_grid),
                doors: std::slice::from_ref(&door),
                sequence_manager: &mut engine.orders.sequence_manager,
            }
            .authorize_and_reserve(owner, seq_id, 0)
        );
        assert_eq!(engine.world.fast_grid_mut().lift_state_mut(0).occupants, 1);

        engine.execute_pass_door(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            crate::gate::DoorIndex(0),
            true,
            0,
        );
        assert_eq!(engine.world.fast_grid_mut().lift_state_mut(0).occupants, 1);
        engine.execute_pass_door(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            crate::gate::DoorIndex(0),
            false,
            0,
        );

        let lift = engine.world.fast_grid_mut().lift_state_mut(0);
        assert_eq!(lift.occupants, 0);
        assert!(!lift.occupied_downwards);
        assert!(!lift.occupied_upwards);
        assert_eq!(lift.wait_time, 0);
        assert!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_lift
                .is_none()
        );
    }

    #[test]
    fn frozen_all_lift_wait_rechecks_and_promotes_successor_in_authorizing_slot() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let sector_number = crate::sector::SectorNumber::new(42);
        install_test_lift_sector(&mut engine, owner, sector_number);
        engine.world.fast_grid_mut().lift_state_mut(0).wait_time = 2;
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                door_type: crate::gate::DoorType::LiftHigh,
                sector_in: sector_number,
                ..crate::gate::Door::default()
            });
        let mut wait = SequenceElement::new_movement(
            1,
            Command::WaitFreeLift,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            gate_id, sector, ..
        } = &mut wait.data
        {
            *gate_id = Some(crate::gate::DoorIndex(0));
            *sector = crate::position_interface::SectorHandle::new(42);
        }
        let seq_id = engine.orders.sequence_manager.launch_element(wait);
        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitFreeLift, seq_id, 0);
        engine.set_actors_frozen(true);
        let _ = engine
            .orders
            .sequence_manager
            .take_pending_synchronous_actions();
        let sim = crate::sim_rng::test_context();

        engine.tick_actor_animation_action_change_slots(&sim, &assets);
        assert_eq!(engine.world.fast_grid_mut().lift_state_mut(0).wait_time, 1);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .expect("blocked lift wait remains installed")
                .state,
            SequenceState::InProgress
        );

        engine.tick_actor_animation_action_change_slots(&sim, &assets);
        assert_eq!(engine.world.fast_grid_mut().lift_state_mut(0).wait_time, 0);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .expect("zeroed lift wait remains installed")
                .state,
            SequenceState::InProgress,
            "authorization returns false on the frame that decrements the cooldown to zero"
        );

        engine.tick_actor_animation_action_change_slots(&sim, &assets);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .expect("authorized lift wait remains inspectable")
                .state,
            SequenceState::Terminated
        );
        let lift = engine.world.fast_grid_mut().lift_state_mut(0);
        assert_eq!(lift.occupants, 1);
        assert!(lift.occupied_downwards);
        // The fallback idle Wait is no longer installed inside the
        // terminating owner slot: the null-order guard books it at the start
        // of the owner's next actor frame.
        engine.tick_actor_animation_action_change_slots(&sim, &assets);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner)
                .and_then(|(sequence, element)| engine
                    .orders
                    .sequence_manager
                    .get_element(sequence, element))
                .map(|element| element.command),
            Some(Command::Wait),
            "DoNext/Wait translation must finish by the owner's next actor frame"
        );
    }
}

#[cfg(test)]
mod soldier_take_drink_parity_tests {
    use super::*;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ActorData, ActorSoldier, ElementBonus, ElementData, ElementKind, ElementProjectile,
        HumanData, NpcData, ObjectData, ObjectType, Posture, ProjectileData, SoldierData,
    };
    use crate::sequence::SequenceElement;

    fn make_soldier_at(x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D { x, y, z: 0.0 });
        element.set_position_map(crate::coordinates::MapPoint { x, y });
        element.set_direction_instantly(0);
        Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        })
    }

    fn make_pc_at(x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D { x, y, z: 0.0 });
        element.set_position_map(crate::coordinates::MapPoint { x, y });
        Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        })
    }

    fn make_projectile_object_at(object_type: ObjectType, x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D { x, y, z: 0.0 });
        element.set_position_map(crate::coordinates::MapPoint { x, y });
        Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                object_type,
                ..ObjectData::default()
            },
            projectile: ProjectileData::default(),
        })
    }

    fn make_bonus_object_at(object_type: ObjectType, x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: if object_type == ObjectType::Ale {
                ElementKind::ObjectOther
            } else {
                ElementKind::ObjectBonus
            },
            active: true,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D { x, y, z: 0.0 });
        element.set_position_map(crate::coordinates::MapPoint { x, y });
        Entity::Bonus(ElementBonus {
            element,
            object: ObjectData {
                object_type,
                ..ObjectData::default()
            },
        })
    }

    fn launch_interaction_and_tick(
        command: Command,
        actor: Entity,
        antagonist: Entity,
    ) -> (EngineInner, EntityId) {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let actor_id = engine.add_entity(actor);
        let antagonist_id = engine.add_entity(antagonist);
        engine.launch_element(SequenceElement::new_interaction(
            1,
            command,
            Some(actor_id),
            Some(antagonist_id),
        ));

        let mut dev = DevState::default();
        let mut display = HostDisplayState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        assert_eq!(
            engine
                .get_entity(actor_id)
                .expect("interaction actor present")
                .element_data()
                .direction(),
            0,
            "the sequence-manager dispatch follows the entity loop, so its new order cannot turn the actor on the launch frame"
        );
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        (engine, actor_id)
    }

    #[test]
    fn soldier_taking_sets_goal_and_turns_toward_antagonist() {
        let (engine, actor_id) = launch_interaction_and_tick(
            Command::Take,
            make_soldier_at(0.0, 0.0),
            make_projectile_object_at(ObjectType::Purse, 10.0, 0.0),
        );

        let actor = engine.get_entity(actor_id).unwrap();
        assert_eq!(actor.element_data().direction(), 1);
    }

    #[test]
    fn soldier_drinking_ale_turns_toward_existing_goal() {
        let mut soldier = make_soldier_at(0.0, 0.0);
        soldier.element_data_mut().set_direction_goal(1);
        let (engine, actor_id) = launch_interaction_and_tick(
            Command::DrinkAle,
            soldier,
            make_bonus_object_at(ObjectType::Ale, 100.0, 0.0),
        );

        let actor = engine.get_entity(actor_id).unwrap();
        assert_eq!(actor.element_data().direction(), 1);
    }

    #[test]
    fn crouched_pc_take_uses_stamped_crouched_animation() {
        let mut pc = make_pc_at(0.0, 0.0);
        pc.element_data_mut().posture = Posture::Crouched;
        let (engine, actor_id) = launch_interaction_and_tick(
            Command::Take,
            pc,
            make_bonus_object_at(ObjectType::BonusPurse, 10.0, 0.0),
        );

        assert_eq!(
            engine
                .get_entity(actor_id)
                .expect("crouched PC remains present")
                .actor_data()
                .expect("crouched PC retains actor data")
                .installed_order
                .as_ref()
                .map(|order| order.order_type),
            Some(OrderType::TakingCrouched),
            "PC Translate(Take) must use the interaction element's Crouched post-transition stamp"
        );
    }

    #[test]
    fn nearby_pc_does_not_pick_up_bonus_without_take_command() {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        engine.add_entity(make_pc_at(100.0, 100.0));
        let bonus_id =
            engine.add_entity(make_bonus_object_at(ObjectType::BonusPurse, 100.0, 100.0));

        let mut dev = DevState::default();
        let mut display = HostDisplayState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

        let bonus = engine.get_entity(bonus_id).unwrap();
        assert!(bonus.element_data().active);
        assert!(!bonus.object_data().unwrap().taken);
    }
}

#[cfg(test)]
mod drop_ammo_merge_tests {
    use super::*;
    use crate::campaign::{Campaign, PcDescription};
    use crate::element::{ActorPc, ElementData, ElementKind, EntityId, Posture};
    use crate::profiles::{Action, CharacterProfileIdx};
    use crate::sequence::{Field, FieldValue, SequenceElement};

    fn count_bonuses(engine: &EngineInner, action: Action) -> Vec<(EntityId, u16)> {
        engine
            .world
            .entities
            .bonuses()
            .filter_map(|(entity_id, bonus)| {
                if bonus.element.active && bonus.object.associated_action == action {
                    Some((entity_id.into(), bonus.object.quantity))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Build an engine with one PC at the origin, a campaign with one
    /// PcDescription whose status starts with `bow_ammo` arrows, and a
    /// move-box that lets `find_authorized_position_toward` return a
    /// valid drop position on the empty FastFindGrid.
    fn build_engine_with_pc(bow_ammo: u16) -> (EngineInner, EntityId, LevelAssets) {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let pm = std::sync::Arc::make_mut(&mut assets.profile_manager);
        pm.characters.push(crate::profiles::CharacterProfile {
            index: 0,
            filename: "TEST_PC".into(),
            profile_name: "TEST".into(),
            ..Default::default()
        });
        let mut ale_conversion =
            vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
        ale_conversion[crate::order::OrderType::ObjectLying as usize] = 0;
        assets.accessory_sprite_prototypes.insert(
            crate::element::ObjectType::Ale,
            crate::sprite::Sprite::new(
                std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                    action_id: crate::order::OrderType::ObjectLying as u16,
                    frame_ids: vec![1],
                    delays: vec![0],
                    distances: vec![0],
                    offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                    sound_ids: vec![0],
                    ..Default::default()
                }]),
                std::sync::Arc::new(ale_conversion),
            ),
        );

        let mut campaign = Campaign::default();
        let mut desc = PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(0)),
            ..Default::default()
        };
        desc.status.set_ammo(Action::Bow, bow_ammo);
        campaign.characters.push(desc);
        engine.mission_domain.campaign = campaign;

        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        let mut pc_conversion =
            vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
        pc_conversion[crate::order::OrderType::DroppingAle as usize] = 0;
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_id: crate::order::OrderType::DroppingAle as u16,
                action_done: 1,
                hotspot: crate::coordinates::SpriteLocalPoint::new(8.0, 4.0),
                frame_ids: vec![1, 2, 3],
                delays: vec![0, 0, 0],
                distances: vec![0, 0, 0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
                sound_ids: vec![0, 0, 0],
                ..Default::default()
            }]),
            std::sync::Arc::new(pc_conversion),
        );
        // Sprite::new owns a fresh PositionInterface, so place the fixture
        // after installing its authored DropAle script. Otherwise the shared
        // DropAmmo tests silently run from the sprite default at (0, 0).
        element.set_position_map(crate::coordinates::MapPoint { x: 100.0, y: 100.0 });
        element.set_direction_instantly(0);
        // Seed a non-empty move box so try_get_drop_position's
        // is_somewhere check passes.  The exact dims don't matter on
        // an empty grid.
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-5.0, -5.0),
                crate::coordinates::MapVec::new(5.0, 5.0),
            ));

        let pc_id = engine.add_entity(crate::element::Entity::Pc(ActorPc {
            element,
            actor: Default::default(),
            human: Default::default(),
            pc: crate::element::PcData {
                profile_index: CharacterProfileIdx(0),
                campaign_description_index: Some(0),
                ..Default::default()
            },
        }));

        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        (engine, pc_id, assets)
    }

    fn drop_ammo_and_tick(
        engine: &mut EngineInner,
        pc_id: EntityId,
        amount: u32,
        assets: &LevelAssets,
    ) {
        let mut elem =
            SequenceElement::new_generic(1, crate::element::Command::DropAmmo, Some(pc_id));
        elem.set_property(Field::ActionId, FieldValue::Integer(Action::Bow as u32));
        elem.set_property(Field::Amount, FieldValue::Integer(amount));
        engine.launch_element(elem);

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        engine.perform_hourglass(&mut display, &mut InputState::default(), assets, &mut dev);
    }

    #[test]
    fn drop_ale_spawns_object_other_and_survives_its_next_live_owner_slot() {
        let (mut engine, pc_id, assets) = build_engine_with_pc(0);
        let expected_action_point = engine
            .get_entity(pc_id)
            .unwrap()
            .cxx_current_point_map()
            .unwrap();
        engine.mission_domain.campaign.characters[0]
            .status
            .set_ammo(Action::Ale, 1);
        engine.launch_element(SequenceElement::new(
            1,
            crate::element::Command::DropAle,
            Some(pc_id),
        ));

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        // Translation installs the authored drop order; the bottle itself is
        // created only when that animation reaches its DONE action point.
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        let (_, _, order) = engine
            .orders
            .sequence_manager
            .current_order_for_actor(pc_id)
            .expect("DropAle must install its animation order");
        assert_eq!(order.order_type, crate::order::OrderType::DroppingAle);
        assert_eq!(
            engine.mission_domain.campaign.characters[0]
                .status
                .get_ammo(Action::Ale),
            1,
            "translation must not consume ale before the action point"
        );

        // Drive the real owner envelope across the sprite's authored DONE
        // frame. This is the lifecycle that the schema-14 Save028 replay
        // exercises; directly injecting ExecuteSideOutcomes would miss a
        // dropped callback between generic Execute and Actor::Hourglass.
        for _ in 0..4 {
            engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        }
        assert_eq!(
            engine.mission_domain.campaign.characters[0]
                .status
                .get_ammo(Action::Ale),
            0,
            "DropAle DONE must consume one ale at the action point"
        );
        assert_eq!(
            engine
                .feedback
                .sound_sim
                .pending_exclamations
                .iter()
                .map(|pending| (pending.actor_id, pending.exclamation_id))
                .collect::<Vec<_>>(),
            vec![(pc_id.index(), crate::engine::melee::HERO_OUT_OF_AMMO)],
            "the last ale must synchronously queue HERO_OUT_OF_AMMO"
        );
        let ale_id = engine
            .world
            .entities
            .occupied()
            .find_map(|(id, entity)| {
                (entity
                    .object_data()
                    .is_some_and(|object| object.object_type == crate::element::ObjectType::Ale))
                .then_some(id)
            })
            .expect("DropAle DONE must append its RHElementAle-equivalent");
        let ale = engine.get_entity(ale_id).unwrap();
        assert_eq!(ale.kind(), ElementKind::ObjectOther);
        assert_eq!(ale.element_data().position_map(), expected_action_point);
        assert!(!ale.element_data().blipped);
        assert_eq!(ale.sprite().frame_count, 0);
        assert_eq!(
            ale.object_data().unwrap().animation,
            crate::element::Animation::ObjectLying
        );
        assert_eq!(
            ale.original_hourglass_class(),
            crate::element::OriginalHourglassClass::Ale
        );

        // The next frame resolves the appended slot through the real live
        // owner coordinator. A stale ObjectBonus label would panic here.
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        assert!(engine.get_entity(ale_id).is_some_and(Entity::is_active));
    }

    #[test]
    fn three_drops_at_same_position_merge_into_one_pile() {
        let (mut engine, pc_id, assets) = build_engine_with_pc(/* bow_ammo */ 10);

        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);
        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);
        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

        let bonuses = count_bonuses(&engine, Action::Bow);
        assert_eq!(
            bonuses.len(),
            1,
            "three same-position drops should leave one merged pile, got {bonuses:?}"
        );
        assert_eq!(bonuses[0].1, 3, "merged quantity");

        // last_dropped_ammo should point at the surviving pile.
        let pc = engine.get_entity(pc_id).unwrap();
        let pc_data = match pc {
            crate::element::Entity::Pc(p) => &p.pc,
            _ => unreachable!(),
        };
        assert_eq!(pc_data.last_dropped_ammo, Some(bonuses[0].0));
        assert_eq!(pc_data.last_ammo_dropping_position.x, 100.0);
    }

    #[test]
    fn drop_over_pile_cap_spawns_fresh_and_bumps_facing() {
        let (mut engine, pc_id, assets) = build_engine_with_pc(20);

        // Fill a pile to the cap (5).
        for _ in 0..5 {
            drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);
        }
        let bonuses = count_bonuses(&engine, Action::Bow);
        assert_eq!(bonuses.len(), 1, "five drops merge into one pile");
        assert_eq!(bonuses[0].1, 5, "pile capped at 5");

        let dir_before = engine.get_entity(pc_id).unwrap().element_data().direction();

        // Sixth drop overflows the cap → new pile, facing rotates +1.
        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

        let bonuses = count_bonuses(&engine, Action::Bow);
        assert_eq!(
            bonuses.len(),
            2,
            "cap-overflow drop should spawn a fresh pile, got {bonuses:?}"
        );
        // The fresh pile is the one with quantity 1.
        let fresh_qty = bonuses.iter().find(|(_, q)| *q == 1).map(|(_, q)| *q);
        assert_eq!(fresh_qty, Some(1));

        let dir_after = engine.get_entity(pc_id).unwrap().element_data().direction();
        assert_eq!(
            dir_after,
            (dir_before + 1).rem_euclid(16),
            "PC facing should rotate +1 sector on cap overflow"
        );
    }

    #[test]
    fn moving_between_drops_breaks_merge() {
        let (mut engine, pc_id, assets) = build_engine_with_pc(10);

        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

        // Teleport the PC sideways before the second drop — same as
        // walking off the original tile.
        if let Some(entity) = engine.world.entities.get_mut(pc_id) {
            entity
                .element_data_mut()
                .set_position_map(crate::coordinates::MapPoint { x: 200.0, y: 200.0 });
        }

        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

        let bonuses = count_bonuses(&engine, Action::Bow);
        assert_eq!(
            bonuses.len(),
            2,
            "moving between drops invalidates the merge gate, got {bonuses:?}"
        );
    }
}
