//! Main per-frame update tick (`perform_hourglass`).

mod deferred_outcomes;
mod frame_systems;
mod mission;
mod paths;

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
    super::diagnostics::config().drop_boundary_matches(frame, owner)
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
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Flying);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
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
        let owner = engine.add_test_entity(airborne_pc());
        engine.apply_door_pass_transition_completion_side_effects(
            &LevelAssets::new(),
            owner,
            OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
        );

        let pc = engine.get_entity(owner).unwrap();
        assert_eq!(pc.element_data().posture(), Posture::Crouched);
        assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Waiting);
        assert!(pc.actor_data().unwrap().active_door_pass.is_none());
    }

    #[test]
    fn restored_ladder_down_exits_complete_without_active_door_pass() {
        for action in [
            OrderType::TransitionClimbingLadderDownWaitingUpright,
            OrderType::TransitionClimbingLadderDownWaitingUprightAlerted,
        ] {
            let mut engine = EngineInner::new();
            let owner = engine.add_test_entity(airborne_pc());
            engine
                .get_entity_mut(owner)
                .unwrap()
                .set_posture(Posture::OnLadder);

            engine.apply_door_pass_transition_completion_side_effects(
                &LevelAssets::new(),
                owner,
                action,
            );

            let actor = engine.get_entity(owner).unwrap();
            assert_eq!(
                actor.element_data().posture(),
                Posture::Upright,
                "{action:?}"
            );
            assert_eq!(
                actor.actor_data().unwrap().action_state,
                ActionState::Waiting,
                "{action:?}"
            );
            assert!(actor.actor_data().unwrap().active_door_pass.is_none());
        }
    }

    #[test]
    fn unrelated_transition_still_requires_active_door_pass() {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(airborne_pc());
        engine.apply_door_pass_transition_completion_side_effects(
            &LevelAssets::new(),
            owner,
            OrderType::TransitionClimbingWallDownWaitingUpright,
        );

        let pc = engine.get_entity(owner).unwrap();
        assert_eq!(pc.element_data().posture(), Posture::Flying);
        assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Moving);
    }
}

#[cfg(test)]
mod frozen_actor_entry_condolation_tests {
    use super::*;
    use crate::ai::{AiEntityHandle, AiLockFlags, Stimulus, StimulusType};
    use crate::element::{
        ActionState, ActorData, ActorSoldier, ElementData, ElementKind, HumanData, NpcData,
        SoldierData,
    };
    use crate::order::{Order, OrderType};
    use crate::sequence::{CascadeFlags, SequenceElement};

    #[test]
    fn selected_terminal_card_precedes_frozen_actors_derived_tail() {
        let mut engine = EngineInner::new();
        let npc = NpcData {
            ai: crate::element::AiActorData {
                ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                ..Default::default()
            },
            ..Default::default()
        };
        let owner = engine.add_test_entity(Entity::Soldier(ActorSoldier {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData {
                action_state: ActionState::WaitingSword,
                execution_frozen: true,
                ..ActorData::default()
            },
            human: HumanData::default(),
            npc,
            soldier: SoldierData {
                cached_camp: crate::element_kinds::Camp::Lacklandists,
                ..SoldierData::default()
            },
        }));
        let ai = engine
            .get_entity_mut(owner)
            .and_then(Entity::enemy_ai_mut)
            .expect("test owner has Enemy AI");
        ai.base.locks_flag_field = AiLockFlags::FREEZE;
        ai.hth_weapon_id = 1;

        let mut strike = SequenceElement::new_interaction(
            1,
            Command::SwordstrikeSmalltalkRight,
            Some(owner),
            None,
        );
        strike
            .orders
            .push_back(Order::test_new(OrderType::StrikingRightSmalltalk, 0.0, 0.0));
        let sequence = engine.orders.sequence_manager.launch_element(strike);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .orders
            .sequence_manager
            .element_interrupted(sequence, 0, CascadeFlags::NEXT_LEVEL);

        let mut assets = LevelAssets::new();
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..Default::default()
        });
        profiles.hth_weapons.push(Default::default());
        engine.tick_actor_animation_action_change_slots_with_after_slot(
            &crate::sim_rng::test_context(),
            &assets,
            |engine, actor| {
                if actor == owner {
                    engine
                        .get_entity_mut(owner)
                        .and_then(Entity::enemy_ai_mut)
                        .expect("test owner retains Enemy AI")
                        .base
                        .stimulus_queue
                        .push(Stimulus::with_human(StimulusType::EventOutOfView, 7));
                }
            },
        );

        let queue = &engine
            .get_entity(owner)
            .and_then(Entity::enemy_ai)
            .expect("test owner retains Enemy AI")
            .base
            .stimulus_queue;
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0].stimulus_type, StimulusType::EventDone);
        assert_eq!(queue[1].stimulus_type, StimulusType::EventOutOfView);
        assert_eq!(
            queue[1].info,
            crate::ai::StimulusInfo::Human(AiEntityHandle::new(7))
        );
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
            npc: Default::default(),
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
        engine.tick_actor_animation_action_change_slots_with_hooks(
            &sim_context,
            &assets,
            |engine, owner| {
                visited.borrow_mut().push(owner);
                engine.tick_mobile_child_owner_boundary(&sim_context, &assets, owner);
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
        let first = engine.add_test_entity(mobile_fx(0, MapPoint::new(0.0, 0.0)));
        let second = engine.add_test_entity(mobile_fx(1, MapPoint::new(10.0, 0.0)));
        let third = engine.add_test_entity(mobile_fx(2, MapPoint::new(20.0, 0.0)));
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
        // Death-place selection pushes it toward the click side (+Y), producing a
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
        let mut element = {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorSoldier;
            initial_element.active = true;
            initial_element.sprite = dying_sprite();
            initial_element
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
        let owner = engine.add_test_entity(Entity::Soldier(ActorSoldier {
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
            "death-position search must cross both synthetic boundaries"
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
        let mut element = {
            let mut initial_element = ElementData::from_initial_posture(Posture::Tied);
            initial_element.kind = ElementKind::ActorSoldier;
            initial_element.active = true;
            initial_element
        };
        element.set_position_map(MapPoint::new(130.0, 130.0));
        element.sprite.position_iface.set_map_increment(stale);
        // The outgoing movement condolence writes the zero idle goal and
        // invalidates the cached increment before corpse placement commits.
        element.sprite.position_iface.set_map_goal(MapPoint::ZERO);
        element.set_position_map_delayed(destination);
        let owner = engine.add_test_entity(Entity::Soldier(ActorSoldier {
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

/// Actor category contributing an action-execution rule.
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

/// Whether a derived owner arm publishes the sprite's raw motion through the
/// specialized Execute latch.
///
/// Original-game actor execution advances the sword-waiting sprite but
/// deliberately returns an in-progress result after swordfight evaluation.
/// Its sprite's looping terminal edge must therefore remain private to the
/// arm instead of completing the actor's lazy Wait element.
fn specialized_execute_uses_sprite_motion(family: ExecuteOwnerFamily) -> bool {
    !matches!(
        family,
        ExecuteOwnerFamily::GenericAnimation | ExecuteOwnerFamily::WaitingSword
    )
}

/// Whether the entry-latched human action branch reaches the synchronous
/// `WAITING_SWORD` swordfight work.
///
/// Original keys this work to the selected order after the common execution-
/// freeze and validity exits; it is not conditional on a later sprite helper
/// returning a completion record.
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

#[cfg(test)]
#[test]
fn waiting_sword_does_not_publish_its_sprite_terminal_edge() {
    assert!(!specialized_execute_uses_sprite_motion(
        ExecuteOwnerFamily::WaitingSword
    ));
    assert!(specialized_execute_uses_sprite_motion(
        ExecuteOwnerFamily::Movement
    ));
}

/// Return the canonical order installed by command translation for the active
/// ability.  Execute-owner selection must match that order, even when an
/// ability later asks the sprite to perform a different animation.  In
/// In particular, the original game installs the healing animation for every Heal command
/// and substitutes the eating animation only during player execution for self-heal.
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
            // Smalltalk strikes have bespoke human action-execution semantics. The
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
        pub(super) const ACTOR_EXECUTE_CATALOG: &[(ExecuteOverride, crate::order::OrderType, ExecuteOwnerFamily)] = &[
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
/// an explicit exception: player-character execution performs the sprite
/// action and side effects, then always returns an in-progress result.
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
        // Actor seek handling consumes non-terminal sprite results while an
        // entity target remains live. The surrounding movement Execute arm
        // observes IN_PROGRESS even though sprite motion recorded a
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

fn motion_latch_debug_config() -> Option<&'static super::diagnostics::ExactOwnerFrame> {
    super::diagnostics::config().motion_latch.as_ref()
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
        // The original actor update latches the execution result before
        // line-crossing checks. A synchronous line callback may interrupt and
        // replace the selected sequence, but the later motion-state switch
        // still reads that already-held nonterminal result; interruption is
        // not execution-owned order advancement.
        && !selected_element_interrupted
        && (selected_element_retired || !selected_entry_order_still_current)
}

/// Stopping movement rewrites its first order in
/// place and assigns a new identity. That identity change is not order advancement: the
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
        // Movement stopping assigns a new ID to the existing order. Runtime order IDs
        // are monotonic, whereas a translated stop-transition successor was
        // allocated before path waypoints that may later be inserted ahead of
        // it. This separates an in-place reseed from order advancement exposing an
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
            "the original game latches terminal arrival before the line callback interrupts the movement"
        );
        assert_eq!(
            apply_post_completion_execute_override(
                MotionState::InProgress,
                Some(MotionState::Terminated),
                true,
                true,
            ),
            MotionState::InProgress,
            "order advancement's installed successor must still project the actor motion back to InProgress"
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
    CorpseUpdates = 6,
    FrameSounds = 7,
    BuildEntityViews = 8,
    BuildWorldView = 9,
    RefreshDetection = 10,
}

const ENTITY_SYSTEM_DETAIL_COUNT: usize = 11;

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
/// simulation-tick deferred-effects phase.
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
        // Keep the existing placeholder/restoration boundary around script callbacks.
        let display = std::mem::take(&mut self.feedback.cutscene_camera.display);
        let effects = self.perform_post_initialize_authoritative(assets);
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

        // The game records parity immediately after the simulation update, then its
        // render pass calls each element's Refresh. Reproduce the resulting
        // arrow and frame-sound mutations here, before the next engine frame.
        // A restored mission starts with no pending pass because its serialized
        // sprites already crossed the preceding Refresh boundary.
        self.apply_pending_presentation_refresh(sim);

        // Fade-to-black presents its ramp in a tight loop without
        // advancing simulation. Drain the corresponding presentation
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

        // Director work runs after the preceding simulation tick and can
        // complete a CameraGoto/ZoomLevel sequence element there.  Original
        // termination followed by readiness and launch executes immediate
        // successors before the next actor update. Close that between-
        // frame callback stack now: this preserves the post-update state
        // boundary while ensuring LockUser/SendMessage/Timer successors run
        // before any actor receives the next movement tick.
        self.drain_pending_immediate_actions_sync(sim, assets);

        let code = self.perform_hourglass_inner(sim, display, assets, simulation_body_allowed);
        self.refresh_achievement_progress(assets);
        self.advance_auto_quick_action_queues(sim, display, assets);
        self.refresh_fog_of_war(assets, false);
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
        // and re-rolls only when that playback finishes,
        // so keep a deterministic sim-side finish deadline rather than
        // consuming gameplay RNG immediately when playback starts.
        let num_sources = self.feedback.sound_sim.sources.num_sources();
        for i in 0..num_sources {
            let Some(src) = self.feedback.sound_sim.sources.get_mut(i) else {
                continue;
            };
            if !src.is_effectively_active()
                || src.source_kind != crate::sound_source::SoundSourceKind::Delayed
            {
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
                let duration = assets
                    .audio
                    .source_durations
                    .get(&src.id)
                    .copied()
                    .unwrap_or(0);
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

    /// Apply the sprite mutations performed by the preceding original-game
    /// refresh, after its parity snapshot and before the next tick.
    pub(crate) fn apply_pending_presentation_refresh(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
    ) {
        if !std::mem::take(&mut self.control.arrow_refresh_pending) {
            return;
        }

        self.refresh_arrows_for_presentation(sim);
        {
            let _detail = entity_system_detail_guard(EntitySystemDetail::FrameSounds);
            self.dispatch_frame_sounds();
        }
    }

    /// Run the arrow portion of the game refresh immediately.
    ///
    /// Besides the ordinary post-snapshot refresh, Original can re-enter
    /// the refresh while constructing an in-game modal.
    /// Dialogue commands do that synchronously, so a newly-created arrow can
    /// publish its orientation before the same frame's parity snapshot.
    pub(crate) fn refresh_arrows_for_presentation(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
    ) {
        // Refresh walks the full display-sort result. Its FX-polyline merge
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
    /// The original game loop calls this after the first refresh and sound
    /// update, not from inside the engine tick. The host therefore invokes this
    /// explicit stage after its first refresh/sound boundary.  Rollback
    /// replay invokes the same stage after replaying frame zero so the
    /// resulting pre-frame-one simulation state remains deterministic.
    fn perform_post_initialize_authoritative(
        &mut self,
        assets: &LevelAssets,
    ) -> Option<super::SideEffects> {
        if !self.control.sim_config.script_enabled
            || self.script_domains.mission_ui.game_post_initialized
        {
            return None;
        }

        // The game latch advances even without a mission callback. Preserve
        // the no-VM boundary: no refresh, RNG lease, or effects are consumed.
        if self.scripts.mission.is_none() {
            self.script_domains.mission_ui.game_post_initialized = true;
            // Completion is authoritative even with no callback effects:
            // the host records Some as the replay's post-initialize stage bit.
            return Some(super::SideEffects {
                code: GameCode::LevelInProgress,
                ..Default::default()
            });
        }

        // PostInitialize can call randomising natives, so keep it on the same
        // engine-owned deterministic stream while moving only the scheduling
        // boundary.
        let sim = self.control.simulation_context();
        let sim = &sim;

        // This explicit host stage is defined to run after the first native
        // Refresh. Cross the same pending presentation boundary before
        // PostInitialize can consume RNG or inspect sprite state.
        self.apply_pending_presentation_refresh(sim);

        self.run_post_initialize_if_needed(sim, assets);
        self.drain_pending_immediate_actions_sync(sim, assets);

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
        let camera = self.feedback.cutscene_camera.display.clone();
        let effects = self.perform_post_initialize_authoritative(assets);
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

        let was_swordfighting =
            time_hourglass_phase(HourglassPhase::Entities, || self.hourglass_phase_entities());

        let manager_fifo_before_entity_phase =
            self.orders.sequence_manager.deferred_elements_to_go();
        let terminal_movement_order_pops =
            time_hourglass_phase(HourglassPhase::EntitySystems, || {
                self.hourglass_phase_entity_systems(sim, display, assets)
            });

        time_hourglass_phase(HourglassPhase::Npcs, || self.hourglass_phase_npcs());

        time_hourglass_phase(HourglassPhase::GameplaySystems, || {
            self.hourglass_phase_gameplay_systems(sim, display, assets)
        });

        time_hourglass_phase(HourglassPhase::Sequences, || {
            self.hourglass_phase_sequences_authoritative(
                sim,
                assets,
                &manager_fifo_before_entity_phase,
                &terminal_movement_order_pops,
            )
        });

        // Terminating a sequence element calls the owner's completion callback,
        // then readies the sequence synchronously
        // in the original game. `Ready` immediately
        // starts the next command level, so an
        // immediate timer successor must be installed before the engine reaches
        // its anonymous-timer scan. This implementation defers the
        // borrow-reentrant card itself, but this barrier must stay on the
        // sequence-manager side of that scan.
        self.dispatch_condolations(sim, assets);

        // Sequence-manager processing runs before the anonymous-timer
        // scan. If a deferred command terminates and advances its sequence
        // to an immediate Timer, the original game executes it re-entrantly, adds
        // it to the timer-element list, and decrements it later in this same
        // tick. Drain that immediate continuation here so Rust preserves the
        // same launch-frame decrement. Waiting until DeferredEffectsEnd's
        // final drain makes every such timer one frame late.
        self.drain_pending_immediate_actions_sync(sim, assets);

        time_hourglass_phase(HourglassPhase::DeferredEffectsEnd, || {
            self.hourglass_phase_deferred_effects_end(sim, assets, was_swordfighting)
        });

        GameCode::LevelInProgress
    }

    /// Prove that a live host speech completion came from the sealed timing
    /// catalog admitted before ranked engine construction. The concrete audio
    /// sample is presentation-only; the duration is the only selected value
    /// that enters simulation state. Explicit variants therefore bind one
    /// exact ordered catalog entry; random playback uses the group's longest
    /// English duration, independent of the local audio variant.
    fn validate_ranked_speech_resolution(
        assets: &LevelAssets,
        pending: &crate::sound::PendingExclamation,
        resolution: &crate::sound::ResolvedExclamation,
    ) -> Result<(), String> {
        let identifier = (pending.profile_id & 0xFFFF_0000) | u32::from(pending.exclamation_id);
        let group = assets
            .audio
            .speech_timing_catalog
            .groups
            .get(&identifier)
            .ok_or_else(|| {
                format!("ranked sound resolution {identifier:#010x} has no sealed timing group")
            })?;
        if group.variants.is_empty() {
            return Err(format!(
                "ranked sound timing group {identifier:#010x} has no authored variants"
            ));
        }

        let duration_matches = match pending.variant {
            -1 => {
                group
                    .variants
                    .iter()
                    .filter_map(|variant| variant.duration_frames)
                    .max()
                    == Some(resolution.duration_frames)
            }
            explicit if explicit >= 0 => {
                let variant_index = usize::try_from(explicit).map_err(|_| {
                    format!(
                        "ranked sound variant {explicit} for {identifier:#010x} is not representable"
                    )
                })?;
                let variant = group.variants.get(variant_index).ok_or_else(|| {
                    format!(
                        "ranked sound variant {variant_index} is outside the {} authored variants for {identifier:#010x}",
                        group.variants.len()
                    )
                })?;
                let expected = variant.duration_frames.ok_or_else(|| {
                    format!(
                        "ranked sound variant {variant_index} for {identifier:#010x} has no authoritative duration"
                    )
                })?;
                expected == resolution.duration_frames
            }
            invalid => {
                return Err(format!(
                    "ranked sound request {identifier:#010x} has invalid variant {invalid}"
                ));
            }
        };
        if !duration_matches {
            return Err(format!(
                "ranked sound resolution {identifier:#010x} supplied unauthoritative duration {}",
                resolution.duration_frames
            ));
        }
        Ok(())
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
        if !super::diagnostics::config().path_barrier {
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
        if !super::diagnostics::config().path_barrier {
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

    /// Execute a mobile element at its first masked-effect child's
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
                        "mobile child {child_id} at its update slot references missing master index {mobile_index}"
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
                .navigation
                .hiking_paths
                .get(usize::from(path_index))
                .unwrap_or_else(|| panic!("mobile {mobile_index} lost hiking path {path_index}"));
            if let Some(motion) = self.world.mobile_elements[mobile_index].begin_hourglass_motion()
            {
                let movement_animation_speed =
                    self.world.mobile_elements[mobile_index].animation_speed();
                // The original game translates every masked child before
                // line-crossing checks and before the goal/waypoint arm. Its
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
                            "mobile {mobile_index} waypoint update at child {child_id} failed: {error}"
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
                panic!("mobile {mobile_index} child {child_id} vanished before masked FX update")
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
                "static update owner {owner:?} disappeared immediately after live legacy-slot resolution"
            )
        });
        match entity {
            Entity::Fx(fx) if fx.fx.mobile_index.is_some() => (),
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
                            .unwrap_or_else(|| panic!("FX {owner:?} references missing patch {patch_idx} at its live update slot"));
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
                    .unwrap_or_else(|| panic!("FX {owner:?} vanished before sprite update"))
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
                // Sprite handling samples the engine FreezeAll state after the
                // synchronous Scroll VM returns, not at update entry.
                if !self.actors_frozen()
                    && let Some(entity) = self.world.entities.get_mut(owner)
                {
                    let Entity::Scroll(scroll) = entity else {
                        panic!(
                            "scroll {owner:?} changed concrete type before entry-active sprite update"
                        )
                    };
                    // The original game tests activity only once on entry. A due VM
                    // callback may deactivate this surviving scroll, but its
                    // sprite still advances before this update returns.
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
                // The ale update returns false once inactive, but
                // The engine removes the element with its default
                // deactivation-only mode. The pointer stays in the element collection
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
                    "Entity::Bonus {owner:?} has unsupported original-game concrete-kind mapping for {:?}",
                    bonus.object.object_type
                ),
            },
            _ => {}
        }
    }

    /// Run the bounded base-actor update in live original-game element
    /// order: generic animation/Execute, synchronous combat-injury Think,
    /// completion/priority effects, then `ActionChange`.
    ///
    /// Element serialization sorts elements by creation order before writing a
    /// save, and the loaded compact array
    /// retains that order. Rust entity IDs keep the initialized mission's
    /// stable sparse slots, so their numeric order is not the loaded
    /// element order. Walk the authoritative original-game creation
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

                        // The actor update consumes one queued base
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
                            // The actor update does not reach the
                            // execution-freeze return until after it has
                            // observed an orderless selected element and run
                            // order advancement. A terminal transition recorded
                            // in an earlier-created attacker's slot therefore
                            // sends its selected-element condolence before the
                            // Human/NPC tail (and its detection refresh), even
                            // though this actor cannot execute an order now.
                            self.dispatch_selected_condolations_for_actor_entry(
                                sim, entity_id, assets,
                            );
                            self.drain_script_synchronous_actions(
                                sim,
                                assets,
                                &mut Vec::new(),
                            )
                            .unwrap_or_else(|error| {
                                panic!(
                                    "frozen actor {entity_id:?} order-advancement boundary failed to drain synchronous sequence work: {error:?}"
                                )
                            });
                            // The actor update refreshes the order after applying
                            // the delayed position. With no selected order it
                            // clears the pointer, then an execution freeze returns
                            // before lazy Wait and the second movement snapshot.
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
                                "frozen actor {entity_id:?} leaked synchronous sequence work after its specialized update tail: {leaked_slot_work:?}"
                            );
                            self.orders
                                .sequence_manager
                                .restore_pending_synchronous_actions(preexisting_sequence_work);
                            break 'actor_hourglass;
                        }

                        // The engine tick updates every element regardless of
                        // whether it is active. The actor update
                        // then installs Wait whenever its order is empty. Active
                        // controls world presence/rendering, not sequence time.
                        self.ensure_wait_element(entity_id);
                        // The original game's wait-to-sequence-launch path then
                        // Sequence launch through element dispatch to instruction is
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

                        // The actor update starts a move after lazy Wait
                        // installation and immediately before it samples the
                        // current order ID and enters Execute. The delayed-position
                        // branches begin an earlier movement step for their crossing
                        // segment, then reach this second snapshot as well. Keep
                        // PositionInterface's old-position latch frame-local;
                        // movement and combat helpers query movement status later in
                        // this same owner slot.
                        self.world
                            .entities
                            .get_mut(entity_id)
                            .expect("actor disappeared before new movement update")
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
                        // The actor update refreshes the order from the selected
                        // element immediately before Execute. Preserve that
                        // pointer publication independently of manager selection:
                        // later order advancement or instruction updates the explicit
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
                            .expect("actor disappeared before installing its update order")
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
                            actor.select_execute_order(order_id);
                        }
                        self.debug_drop_owner_boundary(
                            "execute_latch_published",
                            entity_id,
                            selected_order,
                        );
                        // Player action execution handles the carrying-corpse exit for an
                        // ENTER_SWORDFIGHT before the default validity arm: on
                        // the transition's first Execute it drops immediately
                        // and returns TERMINATED. Translation still has to register the
                        // transition, so key this to the entry-latched order
                        // rather than consuming it during transition generation.
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
                        // after the actor update has established the new-order flag for
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
                            // Player-character execution owns this initialization,
                            // not the DROP_CORPSE command builder. Posture
                            // transitions inserted for another PC command enter
                            // the same animation arm without an ActiveAbility.
                            // The original game aligns the carried actor after validity and before
                            // starting the action.
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
                        // actor's next update rather than entering generic
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
                                        specialized_execute_uses_sprite_motion(*family)
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
                            // The actor update applies WAIT_TIMER / WAIT_FREE_LIFT
                            // after complete action execution. Specialized
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
                            // assigned to the actor's motion state before the update
                            // applies completion and order-advancement handling.
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
                            && selected_owner_family
                                .is_some_and(specialized_execute_uses_sprite_motion)
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
                            // the actor update latches it.
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
                            // Player-character execution owns eventual strike /
                            // execution remarks. Their 50% RNG draw and speech
                            // side effects occur synchronously before the next
                            // element's update slot.
                            self.tick_pc_combat_anim_speech_for_owner(sim, assets, entity_id);
                        }
                        // The original game clears the sequence-started flag immediately
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

                        // Human-actor execution performs this work inside
                        // the sword-waiting arm, after action processing and before
                        // returning its motion result to the actor update. Keep
                        // launches and cross-actor mutations live so later slots
                        // observe them and earlier slots do not.
                        // This is part of human action execution's selected
                        // WAITING_SWORD arm, not an animation-completion
                        // callback.  In particular, the arm still runs when
                        // the generic sprite helper has no completion record
                        // for this slot. Key it to the actor update's
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

                        // Human-actor execution decrements the parry hold
                        // counter and queues a parry stop before this actor yields
                        // its legacy slot. Preserve that ordering relative to
                        // sword hits performed by later-created actors.
                        if let Some(result) = execute_result.as_mut() {
                            self.tick_parry_counter_for_execute(entity_id, result);
                        }

                        // The original actor update modifies the just-produced
                        // Execute result for WAIT_TIMER / WAIT_FREE_LIFT before
                        // completion or order advancement. Sampling the current element
                        // here is intentional: WaitingSword callbacks above may
                        // have synchronously replaced it.
                        if let Some(result) = execute_result.as_mut() {
                            self.apply_actor_post_execute_wait_modifier(entity_id, result);
                        }
                        // The base actor update calls line-crossing detection
                        // after the complete execution chain and its wait
                        // modifier, but before interpreting the motion result.
                        // Movement owners and Rolling close this boundary in
                        // their specialized executors; generic animation
                        // (including death-place selection and flight) reaches it here.
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
                            // The actor update stores every execution
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

                        // The original-game soldier update runs AI before returning
                        // Terminated to the base actor update. Only after that
                        // synchronous decision tick finishes may order advancement/completion
                        // promote the actor's successor order.
                        self.process_anim_completion_outcomes(sim, outcomes, assets);
                        // Terminating a sequence element calls the
                        // actor's removal notification and then readiness
                        // synchronously inside this update slot. Close only
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
                        // Order advancement changes the retained execution result to
                        // IN_PROGRESS only when Proceed returns a non-null
                        // actor order. Manager residency is not sufficient: queue
                        // exhaustion can terminate the selected element while
                        // leaving a fallback Wait discoverable in the manager,
                        // yet the actor's order remains empty until its next
                        // update entry. `installed_order` is the explicit
                        // mirror updated by order advancement and accepted instruction.
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
                            // Advancing to the next actor order overwrites a TERMINATED result
                            // result with IN_PROGRESS when Proceed exposes another
                            // order. Instruction handling does the same when terminating the
                            // old element synchronously installs a successor.
                            // Specialized owners retire their order internally,
                            // so an entry-identity change is their equivalent of
                            // the base update's TERMINATED branch even when the
                            // last raw sprite edge was START/DONE/IN_PROGRESS.
                            // ABORTED is tied to the sequence element captured
                            // on actor-update entry. Its synchronous
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
                        // Order advancement may synchronously expose a real postponed
                        // successor through state changes and readiness. If it does not,
                        // The original game leaves the actor order empty for the rest of this
                        // actor update. The fallback Wait is created
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
                        // Do not derive the actor order from the manager at the tail. The
                        // exact identity was published at update entry and is
                        // subsequently changed only by order advancement, selected
                        // element cleanup, or a synchronous accepted instruction.
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

                        // Human posture changes update intersecting-corpse state
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

            // Original-game element removal compacts the element collection immediately, so
            // incrementing the loop index skips the element shifted into the
            // removed position. Retaining before incrementing reproduces that
            // behavior. Registration appends newly created elements; their
            // monotonically increasing creation identities let us discover
            // only the new tail without confusing stable Rust slots for
            // Original array positions.
            original_slots.retain(|&id| self.world.entities.get(id).is_some());
            if self.world.next_original_creation_order != observed_creation_counter {
                assert!(
                    self.world.next_original_creation_order > observed_creation_counter,
                    "original-game creation counter moved backwards during update"
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

        // The original game's execution dispatch chain is closed here: generic sprite
        // arms use tick_actor_animation_for; selected movement, melee, bow,
        // ability, beggar, and WaitingSword work use their live owner arms;
        // the human/PC/NPC derived tail hook runs before the slot advances.
    }

    /// Fuse the supported Actor → Human → PC/NPC update phases into one
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
    ) -> Vec<super::movement::TerminalMovementOrderPop> {
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
        );
    }

    fn tick_actor_owner_envelopes_with_owner_hook(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        mut owner_hook: impl FnMut(&mut Self, EntityId),
    ) -> Vec<super::movement::TerminalMovementOrderPop> {
        let mut terminal_movement_order_pops = Vec::new();
        let mut prepared = {
            let _detail = entity_system_detail_guard(EntitySystemDetail::PrepareNpc);
            self.prepare_npc_owner_pass()
        };
        self.tick_actor_animation_action_change_slots_with_hooks(
            sim,
            assets,
            |engine, owner| {
                let _detail = entity_system_detail_guard(EntitySystemDetail::StaticOwners);
                use crate::element::OriginalHourglassClass as Class;

                // Original-derived nonactor nesting: the mobile master/child
                // boundary runs before the independent static owner, followed
                // by projectile/net dispatch.
                let class = engine
                    .get_entity(owner)
                    .unwrap_or_else(|| {
                        panic!(
                            "update owner {owner:?} disappeared immediately after live legacy-slot resolution"
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
                // Human's literal sword-movement arm rejects an unforced
                // movement with no opponents before opponent-facing or
                // seeking. In particular, a stale moved-target seek must
                // launch QuitSwordfight instead of refreshing itself first.
                if let Some(selection) = movement
                    && engine.abort_orphaned_sword_movement(sim, assets, owner, selection)
                {
                    return ExplicitExecuteMotion::default();
                }
                // Seeking's "wait for the post seek sequence to be
                // launched" arm runs ahead of every other seek step: Execute
                // returns TERMINATED before any motion, countdown ageing, or
                // seek refresh, and the actor update then advances the order.
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
                // Seek refresh is part of this exact actor's seeking
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
                    // Opponent-facing / danger-facing run inside the execution
                    // arm *before* seeking, so their facing write and
                    // turning still happen on the frame seeking's
                    // moved-target seek-refresh branch preempts the motion.
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
                // Seeking's completion-time refresh branches return
                // in-progress motion explicitly,
                // so the actor update runs none of its DONE / TERMINATED /
                // ABORTED tail for that slot.
                let movement_motion =
                    engine.tick_entity_movement_owner(sim, assets, owner, movement);
                terminal_movement_order_pops.extend(movement_motion.terminal_order_pops);
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
                        // The player override wraps human action execution. Therefore its
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
                    // The original game's listening-animation update ignores
                    // the sprite's DONE/TERMINATED states and remains in
                    // progress until the wait timer reaches zero. The detection
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
                            "actor owner {} disappeared before its specialized update tail",
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
        terminal_movement_order_pops
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

    /// Dispatch the exact original-game per-frame update chain for a live
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
                            // The projectile update starts a move
                            // before testing the flying flag. Active stopped arrows
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
            // Element removal is called with its default
            // deactivation-only mode from the element's hourglass loop. The
            // projectile remains in the element array as an inactive
            // tombstone so outstanding references and creation order stay
            // valid; physical removal is reserved for teardown/load paths.
            entity.element_data_mut().active = false;
        }
    }

    /// Apply the two sequence-command motion modifiers owned by
    /// the actor update after one execution call.
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

        // Execution is selected from the current sequence element before entering the
        // actor's specialized response. A WaitingSword callback may stop
        // that element before control returns to the actor update, but the
        // original game retains its reference while this update stack unwinds.
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

        if live_command == Some(crate::element::Command::WaitFreeLift)
            && let Some((seq_id, elem_idx)) = live_element
        {
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

    /// Resolve the retained base-Actor motion after derived Execute callbacks
    /// and wait modifiers. Original-game termination advances to the next order through the
    /// owner's live selected sequence element; ABORTED alone uses the sequence
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
                // Player-character execution creates a dropped ale bottle at
                // the DROPPING_ALE action point. Stage this on the retained
                // actor-update result rather than inside the generic
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
            // retained actor-order allocation, but no later priority decision can
            // observe that detached order.
            tracing::trace!(
                ?owner,
                ?entry_seq_id,
                entry_elem_idx,
                %order_id,
                "Done entry element was synchronously collected before actor-update write-back"
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
                "Done entry order was synchronously removed before actor-update write-back"
            );
            return;
        };
        // The original actor update marks the order done immediately after
        // Execute returns. Later callbacks in this same owner slot and
        // The sequence-manager tick can therefore terminate a blocker
        // instead of postponing behind an animation which already reached its
        // action point.
        order.done = true;
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
            Some(e) => e.element_data().posture(),
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
        // the gate midpoint on the initial motion tick. This is a positional
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
        // ActiveDoorPass mirror. These exit transitions need no door
        // geometry: their terminal Execute arms only publish actor state
        // before order advancement selects door passing.
        let restored_completion = self.get_entity(entity_id).and_then(|entity| {
            let actor = entity.actor_data()?;
            if actor.active_door_pass.is_some() {
                return None;
            }
            match action {
                OT::TransitionClimbingWallUpWaitingCrouchedCrenel if entity.is_pc() => {
                    Some((Posture::Crouched, ActionState::Waiting))
                }
                OT::TransitionClimbingLadderDownWaitingUpright
                | OT::TransitionClimbingLadderDownWaitingUprightAlerted => {
                    Some((Posture::Upright, ActionState::Waiting))
                }
                _ => None,
            }
        });
        if let Some((posture, action_state)) = restored_completion {
            let entity = self
                .world
                .entities
                .get_mut(entity_id)
                .expect("restored transition completion owner disappeared");
            entity.set_posture(posture);
            entity
                .actor_data_mut()
                .expect("restored transition completion owner is not an actor")
                .action_state = action_state;
            return;
        }

        let Some((door_index, is_pc)) = self.get_entity(entity_id).and_then(|entity| {
            entity.actor_data().and_then(|actor| {
                actor
                    .active_door_pass
                    .as_ref()
                    .map(|dp| dp.door_index)
                    .or_else(|| {
                        // Original-game v48 saves serialize the translated door-passage
                        // order chain and PositionInterface's live door, but
                        // have no Rust-only ActiveDoorPass mirror.  Until the
                        // first PassingDoor action consumes that pointer it is
                        // the authoritative door for transition completion.
                        entity.position_iface().get_door()
                    })
                    .map(|door_index| (door_index, entity.is_pc()))
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
            // detached actor order and must disappear at the same owner boundary.
            self.actor_freeze_execution(dismount.carried_id);
            if let Some(carried) = self.get_entity_mut(dismount.carried_id)
                && let Some(actor) = carried.actor_data_mut()
            {
                actor.installed_order = None;
            }
        }

        let Some(carried) = self.get_entity_mut(dismount.carried_id) else {
            // The original game permits the carried-actor reference to become empty while the transition
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
            .current_gameplay_point_map()
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
                // helper's live map-space animation hotspot,
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
            // Clearing a human actor's carrier restores the old
            // carrier's heading as the released rider's direction goal
            // before clearing the back-reference
            // as part of the drop.
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
        // The original game makes the carried actor wait, not the helper, before
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
            taking_net_ticks,
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
        self.drain_resume_door_pass(assets, resume_door_pass);
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
        self.drain_smalltalk_strikes(assets, smalltalk_strikes);
        self.drain_killed_at_bottom(killed_at_bottom);
        self.drain_deactivate_entities(deactivate_entities);
        self.drain_pc_target_activations(pc_target_activations);
        self.drain_waking_up_done(sim, assets, waking_up_done);
        self.drain_taking_net_ticks(sim, assets, taking_net_ticks);
        self.drain_pickups(sim, assets, pickups);
        self.drain_drink_done(assets, drink_done);
        self.drain_pickpockets(pickpockets);
        self.drain_wasp_sting_remark(sim, assets, wasp_sting_remark);
        self.drain_special_remark(sim, assets, special_remark);
        self.drain_cry_for_help_under_net(sim, assets, cry_for_help_under_net);
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
/// replays reproduce the same deviation sequence. Required behavior:
/// The original game's soldier path post-processing uses two draws for
/// each of up to three candidate deviations per segment.
#[inline]
fn drunken_deviation_direction(direction: i16) -> [f32; 2] {
    // Converting a sector direction with the aspect ratio compresses the
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
                // The original game copies the complete order: all movement
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
            entity.element_data().posture(),
            Posture::Spy | Posture::Cloaked | Posture::Tree | Posture::AnonymousArcher
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
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
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
        let pc_id = engine.add_test_entity(make_aiming_pc(action_state));
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
            element: {
                let mut initial_element = ElementData::from_initial_posture(posture);
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element.active = true;
                initial_element
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
        let soldier_id = engine.add_test_entity(make_bow_soldier(
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
            "lower-bow lean-out keeps its translated transition order live"
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
        let pc_id = engine.add_test_entity(make_aiming_pc(ActionState::AimingWithBow));
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
        let pc_id = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
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
            "bow aiming raises from waiting by equipping, loading, then raising"
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
            "bow aiming exits bow-up by lowering, unloading, then unequipping"
        );
    }

    #[test]
    fn turn_context_sets_goal_without_snapping_and_books_turning() {
        use crate::sequence::{Field, FieldValue};

        let mut engine = EngineInner::new();
        let owner =
            engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
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
        // last value written to the original game's one shared wait-timer scalar.
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
        // retained. The original game's airborne execution branch overwrites the wait timer
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::Moving));
        let actor = engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();

        // A swordstrike post-seek may remain attached while a non-interruptible
        // ladder/wall fall runs. The original game's ladder/wall fall execution owns
        // the single wait-timer scalar for the flight countdown in this state.
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
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
            let owner = engine.add_test_entity(owner_entity);

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
        // The original game's base actor update decrements it after the player
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
        let owner = engine.add_test_entity(owner_entity);
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
            "sequence-element launch must leave ordinary work unresolved until manager instruction"
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::Moving));
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
                // after execution has produced START but before the actor update
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::MovingFast));
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
        // Path postprocessing allocates its final transition before inserting
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
        let owner =
            engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
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
        let owner = engine.add_test_entity(soldier_entity);
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
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
        assert_eq!(entity.element_data().posture(), Posture::Upright);
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
        // Crouch bodies now stay live until their transition animation
        // completes, so use LeaveSpy — the stealth context still snaps the
        // posture and terminates it synchronously inside its dispatch slot.
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .element_data_mut()
            .publish_order_posture(Posture::Spy);
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
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
        let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
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
        let owner =
            engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
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
            "movement uses >= tolerance + 5 for the max-norm mismatch"
        );
    }

    #[test]
    fn position_assertion_context_accepts_nan_distance_like_original() {
        let mut engine = EngineInner::new();
        let owner =
            engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
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
        let owner =
            engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
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
            *gate_id = Some(crate::gate::DoorIndex::new(0).expect("valid door index"));
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
        let owner =
            engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
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
            *gate_id = Some(crate::gate::DoorIndex::new(0).expect("valid door index"));
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
        let owner =
            engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
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
            *gate_id = Some(crate::gate::DoorIndex::new(0).expect("valid door index"));
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
        let owner =
            engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
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
            *gate_id = Some(crate::gate::DoorIndex::new(0).expect("valid door index"));
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
            crate::gate::DoorIndex::new(0).expect("valid door index"),
            true,
            0,
        );
        assert_eq!(engine.world.fast_grid_mut().lift_state_mut(0).occupants, 1);
        engine.execute_pass_door(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            crate::gate::DoorIndex::new(0).expect("valid door index"),
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
        let owner =
            engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
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
            *gate_id = Some(crate::gate::DoorIndex::new(0).expect("valid door index"));
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
            "next-action/wait translation must finish by the owner's next actor frame"
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
        let mut element = {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorSoldier;
            initial_element.active = true;
            initial_element
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
        let mut element = {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorPc;
            initial_element.active = true;
            initial_element
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
        let mut element = {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectProjectile;
            initial_element.active = true;
            initial_element
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
        let mut element = {
            let mut initial_element = ElementData::default();
            initial_element.kind = if object_type == ObjectType::Ale {
                ElementKind::ObjectOther
            } else {
                ElementKind::ObjectBonus
            };
            initial_element.active = true;
            initial_element
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
        let actor_id = engine.add_test_entity(actor);
        let antagonist_id = engine.add_test_entity(antagonist);
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
        pc.element_data_mut()
            .publish_order_posture(Posture::Crouched);
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
        engine.add_test_entity(make_pc_at(100.0, 100.0));
        let bonus_id =
            engine.add_test_entity(make_bonus_object_at(ObjectType::BonusPurse, 100.0, 100.0));

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

        let mut element = {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorPc;
            initial_element.active = true;
            initial_element
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

        let pc_id = engine.add_test_entity(crate::element::Entity::Pc(ActorPc {
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
            .current_gameplay_point_map()
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
        // dropped callback between generic execution and the actor update.
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
            .expect("completed ale dropping must append its ale element");
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
