use super::*;
use crate::engine::tick::capture_projectile_derived_tails;

#[test]
fn scrolling_table_generation() {
    let bg = BackgroundTransform::default();
    assert_eq!(bg.x_scrolling_values[0], 0.0);
    // First non-zero entry should be DEFAULT_SCROLLING_START (6.0)
    assert_eq!(bg.x_scrolling_values[1], 6.0);
    // Values should be monotonically non-decreasing
    for i in 1..SCROLLING_TABLE_SIZE - 1 {
        assert!(bg.x_scrolling_values[i] <= bg.x_scrolling_values[i + 1]);
    }
    // Last values should be capped at or above DEFAULT_SCROLLING_LIMIT
    assert!(bg.x_scrolling_values[SCROLLING_TABLE_SIZE - 1] >= DEFAULT_SCROLLING_LIMIT);
}

#[test]
fn zoom_state_machine() {
    let display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    engine.feedback.cutscene_camera.display.display_op = DisplayOpCode::NoBackgroundMove;

    assert!(engine.is_zoom_possible(&display));
    assert!(engine.is_zoom_up_possible());
    assert!(engine.is_zoom_down_possible());
    assert!(!engine.is_zooming(&display));

    // Trigger zoom up
    assert!(engine.change_state_with_camera_display(0, EngineStateRequest::ZoomingUp));
    assert!(engine.is_zooming(&display));
    assert!(!engine.is_zoom_possible(&display));
    assert_eq!(
        engine.feedback.cutscene_camera.display.display_op,
        DisplayOpCode::InitZoom
    );
}

#[test]
fn camera_clip_view() {
    let mut camera = CameraState {
        level_size: MapSize::new(2000.0, 1500.0),
        zoom_factor: 1.0,
        view_position: crate::coordinates::MapPoint::new(-100.0, -50.0),
        ..Default::default()
    };
    let clipped = camera.clip_view();
    assert!(clipped);
    assert_eq!(camera.view_position.x, 0.0);
    assert_eq!(camera.view_position.y, 0.0);
}

#[test]
fn hourglass_returns_in_progress() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelInProgress);
    assert_eq!(engine.control.frame_counter, 1);
}

#[test]
fn hourglass_phase_trace_locks_entity_npc_path_sequence_and_deferred_order() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();

    begin_hourglass_phase_capture();
    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    let phases = end_hourglass_phase_capture();

    assert_eq!(result, GameCode::LevelInProgress);
    assert_eq!(
        phases,
        vec![
            HourglassPhase::DeferredEffectsStart,
            HourglassPhase::MissionAndMessages,
            HourglassPhase::NpcOrders,
            HourglassPhase::Paths,
            HourglassPhase::Entities,
            HourglassPhase::EntitySystems,
            HourglassPhase::Npcs,
            HourglassPhase::GameplaySystems,
            HourglassPhase::Sequences,
            HourglassPhase::DeferredEffectsEnd,
        ]
    );
}

#[test]
fn hourglass_phase_trace_records_only_phases_reached_before_mission_exit() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.state.quit_won = true;

    begin_hourglass_phase_capture();
    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    let phases = end_hourglass_phase_capture();

    assert_eq!(result, GameCode::LevelSucceeded);
    assert_eq!(
        phases,
        vec![
            HourglassPhase::DeferredEffectsStart,
            HourglassPhase::MissionAndMessages,
        ]
    );
}

#[test]
fn hourglass_phase_trace_stops_after_the_locked_mission_gate() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.set_engine_locked(true);
    engine
        .orders
        .pending_hades_kills
        .push(EntityId::new(99, crate::element::EntityIdKind::Soldier));

    begin_hourglass_phase_capture();
    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    let phases = end_hourglass_phase_capture();

    assert_eq!(result, GameCode::LevelInProgress);
    assert_eq!(
        engine.control.frame_counter, 1,
        "the lock gate follows clock advance"
    );
    assert!(
        engine.orders.pending_hades_kills.is_empty(),
        "deferred order work must drain before the locked mission gate"
    );
    assert_eq!(
        phases,
        vec![
            HourglassPhase::DeferredEffectsStart,
            HourglassPhase::MissionAndMessages,
        ]
    );
}

#[test]
fn blocking_fade_frame_runs_before_rng_clock_and_phase_dispatch() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.set_fade_freeze_frames_remaining(1);
    let rng_seed = engine.rng_seed();

    begin_hourglass_phase_capture();
    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    let phases = end_hourglass_phase_capture();

    assert_eq!(result, GameCode::LevelInProgress);
    assert_eq!(engine.control.frame_counter, 0);
    assert_eq!(engine.rng_seed(), rng_seed);
    assert!(phases.is_empty());
    assert_eq!(engine.fade_freeze_frames_remaining(), 0);
}

#[test]
fn pending_sequence_animation_starts_after_entity_hourglass_boundary() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));
    bind_test_action_point(
        &mut engine,
        soldier_id,
        OrderType::TransitionWaitingUprightSitting,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    // Bypass EngineInner's synchronous launch wrapper to model an element
    // already waiting in RHSequenceManager's FIFO at frame start.
    let mut element = SequenceElement::new(1, Command::SitDown, Some(soldier_id));
    element.posture_after_transition = Posture::Upright;
    let sequence_id = engine.orders.sequence_manager.launch_element(element);

    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence_id, 0)
        .expect("pending element should have dispatched");
    assert_eq!(element.state, SequenceState::InProgress);
    let order_id = element
        .current_order()
        .expect("SitDown should translate to an animation")
        .order_id
        .get();
    assert_eq!(
        engine
            .get_entity(soldier_id)
            .expect("soldier present")
            .element_data()
            .sprite
            .last_processed_order_id,
        u32::MAX,
        "an order dispatched by RHSequenceManager after the entity loop must not animate in that same frame"
    );

    engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert_eq!(
        engine
            .get_entity(soldier_id)
            .expect("soldier present")
            .element_data()
            .sprite
            .last_processed_order_id,
        order_id,
        "the dispatched animation must start on the following entity frame"
    );
}

fn immortal_pc_hit_by_creation_ordered_arrow(pc_before_arrow: bool) -> i16 {
    use crate::bow_shot::{SpawnArrowParams, spawn_arrow};
    use crate::coordinates::{WorldPoint3D, WorldVec3D};
    use crate::element::Posture;
    use crate::entity_id::PcId;

    let mut engine = EngineInner::new();

    let mut shooter = make_test_soldier(Posture::Upright);
    shooter
        .element_data_mut()
        .set_position_map(MapPoint { x: 0.0, y: 0.0 });
    let Entity::Soldier(shooter_soldier) = &mut shooter else {
        unreachable!();
    };
    shooter_soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
    shooter_soldier.npc.life_points = 100;
    let shooter_id = engine.add_entity(shooter);

    let victim_id = EntityId::Pc(PcId(if pc_before_arrow { 1 } else { 2 }));
    let make_arrow = || {
        spawn_arrow(SpawnArrowParams {
            shooter: shooter_id,
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 25.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: victim_id,
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory: vec![crate::element::TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 25.0,
                },
                time: 2,
            }],
            damage: 10,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        })
    };
    let mut victim = make_test_pc(Posture::Upright);
    victim
        .element_data_mut()
        .set_position_map(MapPoint { x: 50.0, y: 0.0 });
    let Entity::Pc(victim_pc) = &mut victim else {
        unreachable!();
    };
    victim_pc.pc.life_points = 74;
    victim_pc.pc.immortal = true;

    if pc_before_arrow {
        assert_eq!(engine.add_entity(victim), victim_id);
        engine.add_entity(make_arrow());
    } else {
        engine.add_entity(make_arrow());
        assert_eq!(engine.add_entity(victim), victim_id);
    }

    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let Some(Entity::Pc(victim)) = engine.get_entity(victim_id) else {
        panic!("victim PC missing after projectile frame");
    };
    victim.pc.life_points
}

#[test]
fn pc_auto_heal_and_projectile_damage_follow_cross_entity_creation_order() {
    // RHElementActorPC::Hourglass snaps 74 HP to 75. The projectile then
    // subtracts 10 when the PC's creation order is earlier: 74 -> 75 -> 65.
    assert_eq!(immortal_pc_hit_by_creation_ordered_arrow(true), 65);

    // Reversing only the element-array order reverses the observable state:
    // projectile damage 74 -> 64, then the PC hourglass snaps 64 -> 75.
    assert_eq!(immortal_pc_hit_by_creation_ordered_arrow(false), 75);
}

#[test]
fn earlier_projectile_runs_before_later_bow_release_and_spawned_arrow_runs_again() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::bow_shot::{SpawnArrowParams, spawn_arrow};
    use crate::coordinates::{WorldPoint3D, WorldVec3D};
    use crate::element::{ActionState, Command, Posture, TrajectoryPoint};
    use crate::entity_id::{PcId, ProjectileId, SoldierId};
    use crate::movement::ActiveShot;
    use crate::order::{Order, OrderType};
    use crate::profiles::{
        BowProfile, BowShootMode, CharacterProfile, ProfileManager, SoldierProfile,
    };
    use crate::sequence::SequenceElement;
    use crate::weapons::ShootMode;

    let mut engine = EngineInner::new();
    let mut target = make_test_soldier(Posture::Upright);
    target
        .element_data_mut()
        .set_position_map(MapPoint::new(1000.0, 0.0));
    target.element_data_mut().set_position(WorldPoint3D {
        x: 1000.0,
        y: 0.0,
        z: 0.0,
    });
    let Entity::Soldier(target_data) = &mut target else {
        unreachable!();
    };
    target_data.soldier.soldier_profile_index = crate::profiles::SoldierProfileIdx(0);
    target_data.npc.life_points = 100;
    let target_id = engine.add_entity(target);
    assert_eq!(target_id, EntityId::Soldier(SoldierId(0)));

    let shooter_id = EntityId::Pc(PcId(2));
    let existing_arrow = spawn_arrow(SpawnArrowParams {
        shooter: shooter_id,
        bow_point: WorldPoint3D {
            x: 2000.0,
            y: 0.0,
            z: 25.0,
        },
        trajectory_origin: MapPoint::new(2000.0, 0.0),
        target: target_id,
        target_pos: MapPoint::new(1000.0, 0.0),
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 1000.0,
                y: 0.0,
                z: 25.0,
            },
            time: 2,
        }],
        damage: 10,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: -1.0,
            y: 0.0,
            z: 0.0,
        },
    });
    let existing_arrow_id = engine.add_entity(existing_arrow);
    assert_eq!(existing_arrow_id, EntityId::Projectile(ProjectileId(1)));

    let mut shooter = make_test_pc(Posture::Upright);
    shooter
        .element_data_mut()
        .set_position_map(MapPoint::new(0.0, 0.0));
    shooter.element_data_mut().set_position(WorldPoint3D {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    assert_eq!(engine.add_entity(shooter), shooter_id);
    bind_test_bow_release_action(&mut engine, shooter_id);

    let mut shot_element =
        SequenceElement::new_interaction(1, Command::ShootBow, Some(shooter_id), Some(target_id));
    let order = Order::test_new(OrderType::ShootingWithBow, 0.0, 0.0);
    let order_id = order.order_id;
    shot_element.orders.push_back(order);
    let shot_sequence = engine.orders.sequence_manager.launch_element(shot_element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(shot_sequence, 0);
    {
        let shooter = engine
            .get_entity_mut(shooter_id)
            .expect("bow shooter present");
        let actor = shooter.actor_data_mut().expect("bow shooter actor data");
        actor.action_state = ActionState::AimingWithBow;
        actor.active_shot = ActiveShot {
            sequence_id: Some(shot_sequence),
            element_index: 0,
            target: Some(target_id),
            order_id: Some(order_id),
            released: false,
            shoot_mode: Some(ShootMode::Normal),
        };
    }

    let mut profiles = ProfileManager::new();
    profiles.characters.push(CharacterProfile {
        shooting_weapon_id: 1,
        shooting: 100,
        ..CharacterProfile::default()
    });
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        ..SoldierProfile::default()
    });
    profiles.hth_weapons.push(Default::default());
    profiles.bows.push(BowProfile {
        normal_shoot: BowShootMode {
            range: 2000,
            damage: 10,
            ..BowShootMode::default()
        },
        ..BowProfile::default()
    });
    let mut assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };
    complete_test_runtime_fixture(&mut engine, &mut assets);
    bind_test_bow_release_action(&mut engine, shooter_id);
    let shoot_direction = crate::position_interface::vector_to_sector_0_to_15_iso(1000.0, 0.0);
    engine
        .get_entity_mut(shooter_id)
        .expect("bow shooter present after fixture")
        .element_data_mut()
        .set_direction_instantly(shoot_direction);
    let motion = engine
        .get_entity_mut(shooter_id)
        .expect("bow shooter present after fixture")
        .element_data_mut()
        .sprite
        .perform_action(
            sim,
            Some(order_id),
            OrderType::ShootingWithBow,
            shoot_direction as u16,
            crate::sprite::FrameProgression::Default,
            false,
        );
    assert_eq!(motion, crate::sprite::MotionState::Start);
    let motion = engine
        .get_entity_mut(shooter_id)
        .expect("bow shooter present after first animation pulse")
        .element_data_mut()
        .sprite
        .perform_action(
            sim,
            Some(order_id),
            OrderType::ShootingWithBow,
            shoot_direction as u16,
            crate::sprite::FrameProgression::Default,
            false,
        );
    assert_eq!(motion, crate::sprite::MotionState::InProgress);

    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    let (_, visited) = engine.with_simulation_context(|engine, sim| {
        capture_ordered_gameplay_entities(|| {
            engine.tick_actor_owner_envelopes(sim, &assets, &positions)
        })
    });

    let shot_after = engine
        .get_entity(shooter_id)
        .expect("bow shooter remains")
        .actor_data()
        .expect("bow shooter actor data")
        .active_shot;
    assert!(
        shot_after.released,
        "prepared shooting action did not reach its release pulse: {shot_after:?}"
    );

    let spawned_arrow_id = EntityId::Projectile(ProjectileId(3));
    assert_eq!(
        visited,
        vec![target_id, existing_arrow_id, shooter_id, spawned_arrow_id],
        "the existing impact must run before bow release, and the appended arrow must be reached by the live-size loop"
    );
    let spawned_arrow = match engine.get_entity(spawned_arrow_id) {
        Some(Entity::Projectile(projectile)) => projectile,
        _ => panic!("bow release did not leave the spawned arrow alive"),
    };
    assert!(
        spawned_arrow.projectile.launch_segment_start.is_none(),
        "the spawned arrow's explicit primer must be consumed before its second, registered Hourglass"
    );
}

#[test]
fn all_six_primed_throwables_receive_exactly_one_appended_live_slot_advance() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{Entity, Posture};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_test_pc(Posture::Upright));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let start = WorldPoint3D::new(0.0, 0.0, 20.0);
    let end = WorldPoint3D::new(200.0, 0.0, 0.0);
    let mut spawned = Vec::new();
    let mut appended = false;

    engine.tick_actor_animation_action_change_slots_with_hooks(
        &sim,
        &assets,
        |engine, id| {
            if matches!(
                engine.get_entity(id),
                Some(Entity::Projectile(_) | Entity::Net(_))
            ) {
                engine.tick_projectile_or_net_hourglass(&sim, &assets, id);
            }
        },
        |engine, owner| {
            if owner != actor || appended {
                return;
            }
            appended = true;
            for entity in [
                crate::bow_shot::spawn_net(actor, start, end, 0, None),
                crate::bow_shot::spawn_wasp_nest(actor, start, end, 0, None),
                crate::bow_shot::spawn_purse(actor, start, end, 0, None),
                crate::bow_shot::spawn_apple(actor, start, end, None, None, 0, None),
                crate::bow_shot::spawn_stone(actor, start, end, None, None, 0, None),
                crate::bow_shot::spawn_coin(
                    None,
                    start,
                    end,
                    0,
                    0,
                    None,
                    crate::bow_shot::APEX_BEGGAR_COIN,
                    None,
                ),
            ] {
                let id = engine.add_entity(entity);
                let frame_count = match engine.get_entity(id).unwrap() {
                    Entity::Projectile(projectile) => projectile.projectile.frame_count,
                    Entity::Net(net) => net.projectile.frame_count,
                    _ => unreachable!(),
                };
                assert_eq!(
                    frame_count, 1,
                    "{id:?} must enter EntitySlots after its primer"
                );
                spawned.push(id);
            }
        },
        |_, _, _, _, _, _, _| {},
        |_, _| {},
    );

    assert_eq!(spawned.len(), 6);
    for id in spawned {
        let frame_count = match engine.get_entity(id).unwrap() {
            Entity::Projectile(projectile) => projectile.projectile.frame_count,
            Entity::Net(net) => net.projectile.frame_count,
            _ => unreachable!(),
        };
        assert_eq!(
            frame_count, 2,
            "{id:?} must receive one appended live-slot advance, neither zero nor two"
        );
    }
}

#[test]
#[should_panic(expected = "no Original concrete-class mapping for ObjectType::None")]
fn inactive_unsupported_projectile_mapping_panics_before_owner_slot_removal() {
    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Projectile(crate::element::ElementProjectile {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ObjectProjectile,
            active: false,
            ..Default::default()
        },
        object: crate::element::ObjectData {
            object_type: crate::element::ObjectType::None,
            ..Default::default()
        },
        projectile: Default::default(),
    }));
    engine.perform_hourglass(
        &mut HostDisplayState::default(),
        &LevelAssets::new(),
        &mut DevState::default(),
    );
}

#[test]
#[should_panic(expected = "Entity::Net has invalid ObjectType::None")]
fn inactive_unsupported_net_mapping_panics_before_owner_slot_removal() {
    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Net(crate::element::ElementNet {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ObjectNet,
            active: false,
            ..Default::default()
        },
        object: crate::element::ObjectData {
            object_type: crate::element::ObjectType::None,
            ..Default::default()
        },
        projectile: Default::default(),
        net: Default::default(),
    }));
    engine.perform_hourglass(
        &mut HostDisplayState::default(),
        &LevelAssets::new(),
        &mut DevState::default(),
    );
}

#[test]
fn inactive_projectile_virtual_results_are_applied_after_derived_tails() {
    use crate::element::{
        Animation, ElementData, ElementKind, ElementNet, ElementProjectile, ObjectData, ObjectType,
    };

    fn projectile(object_type: ObjectType, flying: bool) -> Entity {
        Entity::Projectile(ElementProjectile {
            element: ElementData {
                kind: ElementKind::ObjectProjectile,
                active: false,
                ..Default::default()
            },
            object: ObjectData {
                object_type,
                animation: Animation::ObjectFlying,
                ..Default::default()
            },
            projectile: crate::element::ProjectileData {
                flying,
                ..Default::default()
            },
        })
    }

    let mut engine = EngineInner::new();
    let apple = engine.add_entity(projectile(ObjectType::Apple, false));
    let stone = engine.add_entity(projectile(ObjectType::Stone, false));
    let grounded_purse = engine.add_entity(projectile(ObjectType::Purse, false));
    let flying_purse = engine.add_entity(projectile(ObjectType::Purse, true));
    let grounded_coin = engine.add_entity(projectile(ObjectType::Coin, false));
    let flying_coin = engine.add_entity(projectile(ObjectType::Coin, true));
    let grounded_net = engine.add_entity(Entity::Net(ElementNet {
        element: ElementData {
            kind: ElementKind::ObjectNet,
            active: false,
            ..Default::default()
        },
        object: ObjectData {
            object_type: ObjectType::Net,
            animation: Animation::NetUnfolding,
            ..Default::default()
        },
        projectile: crate::element::ProjectileData {
            flying: false,
            ..Default::default()
        },
        net: Default::default(),
    }));
    let flying_net = engine.add_entity(Entity::Net(ElementNet {
        element: ElementData {
            kind: ElementKind::ObjectNet,
            active: false,
            ..Default::default()
        },
        object: ObjectData {
            object_type: ObjectType::Net,
            animation: Animation::ObjectFlying,
            ..Default::default()
        },
        projectile: crate::element::ProjectileData {
            flying: true,
            ..Default::default()
        },
        net: crate::element::NetData {
            time_till_unfolding: 1,
            ..Default::default()
        },
    }));
    let assets = LevelAssets::new();
    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    let (_, tails) = capture_projectile_derived_tails(|| {
        engine.with_simulation_context(|engine, sim| {
            engine.tick_actor_owner_envelopes(sim, &assets, &positions)
        })
    });

    assert!(engine.get_entity(apple).is_none());
    assert!(engine.get_entity(stone).is_none());
    assert!(engine.get_entity(flying_purse).is_none());
    assert!(engine.get_entity(grounded_purse).is_some());
    assert!(engine.get_entity(grounded_coin).is_some());
    assert!(engine.get_entity(flying_coin).is_some());
    assert!(engine.get_entity(grounded_net).is_some());
    assert!(engine.get_entity(flying_net).is_some());
    assert_eq!(
        tails,
        vec![
            (apple, ObjectType::Apple),
            (stone, ObjectType::Stone),
            (grounded_purse, ObjectType::Purse),
            (flying_purse, ObjectType::Purse),
            (grounded_coin, ObjectType::Coin),
            (flying_coin, ObjectType::Coin),
        ],
        "each inactive derived sprite tail must run before its virtual bool controls removal"
    );
    for id in [grounded_purse, grounded_coin] {
        let Entity::Projectile(projectile) = engine.get_entity(id).unwrap() else {
            unreachable!()
        };
        assert_eq!(projectile.object.animation, Animation::ObjectBursting);
    }
    let Entity::Net(net) = engine.get_entity(grounded_net).unwrap() else {
        unreachable!()
    };
    assert_eq!(net.object.animation, Animation::ObjectLying);
    let Entity::Net(net) = engine.get_entity(flying_net).unwrap() else {
        unreachable!()
    };
    assert_eq!(net.net.time_till_unfolding, 0);
    assert_eq!(net.object.animation, Animation::NetUnfolding);
}

#[test]
fn apple_and_stone_impact_selects_burst_row_then_derived_tail_owns_removal() {
    use crate::element::{
        Animation, ElementData, ElementKind, ElementProjectile, ObjectData, ObjectType,
        TrajectoryPoint,
    };
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    for object_type in [ObjectType::Apple, ObjectType::Stone] {
        let mut engine = EngineInner::new();
        let mut element = ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..Default::default()
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[Animation::ObjectFlying as usize] = 0;
        conversion[Animation::ObjectBursting as usize] = 16;
        let row = |animation: Animation, frames: Vec<u32>, delays: Vec<u16>| SpriteScript {
            action_id: animation as u16,
            action_done: frames.len().saturating_sub(1) as u16,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            distances: vec![0; frames.len()],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; frames.len()],
            sound_ids: vec![0; frames.len()],
            frame_ids: frames,
            delays,
        };
        element.sprite.conversion = std::sync::Arc::new(conversion);
        let mut scripts = Vec::with_capacity(32);
        for _ in 0..16 {
            scripts.push(row(Animation::ObjectFlying, vec![10, 11], vec![10, 10]));
        }
        for _ in 0..16 {
            scripts.push(row(
                Animation::ObjectBursting,
                vec![20, 21, 22],
                vec![0, 0, 0],
            ));
        }
        element.sprite.scripts = std::sync::Arc::new(scripts);
        element.sprite.force_animation(Animation::ObjectFlying, 0);
        element.sprite.force_sprite(0, 1);
        let projectile_id = engine.add_entity(Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                object_type,
                animation: Animation::ObjectFlying,
                ..Default::default()
            },
            projectile: crate::element::ProjectileData {
                flying: true,
                trajectory: vec![TrajectoryPoint {
                    position: crate::coordinates::WorldPoint3D::new(10.0, 0.0, 0.0),
                    time: 1,
                }],
                ..Default::default()
            },
        }));
        let assets = LevelAssets::new();
        let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        let tick = |engine: &mut EngineInner| {
            capture_projectile_derived_tails(|| {
                engine.with_simulation_context(|engine, sim| {
                    engine.tick_actor_owner_envelopes(sim, &assets, &positions)
                })
            })
            .1
        };

        let mut impact_tails = Vec::new();
        for _ in 0..3 {
            impact_tails = tick(&mut engine);
            if matches!(
                engine.get_entity(projectile_id),
                Some(Entity::Projectile(projectile))
                    if projectile.object.animation == Animation::ObjectBursting
            ) {
                break;
            }
        }
        assert_eq!(impact_tails, vec![(projectile_id, object_type)]);
        let Entity::Projectile(projectile) = engine.get_entity(projectile_id).unwrap() else {
            unreachable!()
        };
        assert_eq!(projectile.object.animation, Animation::ObjectBursting);
        assert_ne!(
            projectile.element.direction(),
            0,
            "regression requires a nonzero impact direction"
        );
        assert_eq!(projectile.element.sprite.current_row, 16);
        assert_eq!(
            projectile.element.sprite.current_frame, 1,
            "the zero-delay first burst frame must advance in the impact derived tail"
        );

        for _ in 0..8 {
            if !engine.get_entity(projectile_id).unwrap().is_active() {
                break;
            }
            assert_eq!(tick(&mut engine), vec![(projectile_id, object_type)]);
        }
        assert!(!engine.get_entity(projectile_id).unwrap().is_active());
        assert_eq!(
            tick(&mut engine),
            vec![(projectile_id, object_type)],
            "inactive virtual call must still run the derived landed tail"
        );
        assert!(engine.get_entity(projectile_id).is_none());
    }
}

#[test]
fn latent_active_shot_does_not_block_higher_selected_nonbow_order() {
    use crate::element::{Command, Posture};
    use crate::movement::ActiveShot;
    use crate::order::Order;
    use crate::sequence::SequenceElement;
    use crate::weapons::ShootMode;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let mut selected = SequenceElement::new(1, Command::Wait, Some(owner));
    let order = Order::test_new(OrderType::WaitingUpright, 0.0, 0.0);
    let order_id = order.order_id;
    selected.orders.push_back(order);
    let selected_seq = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(selected_seq, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_shot = ActiveShot {
        sequence_id: Some(selected_seq),
        element_index: 0,
        target: Some(owner),
        order_id: Some(order_id),
        released: false,
        shoot_mode: Some(ShootMode::Normal),
    };

    assert!(engine.selected_bow_order(owner).is_none());
    let (_, _, executed) = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );
    assert!(
        executed.is_some(),
        "latent active_shot must not suppress the exact selected nonbow Execute arm"
    );
}

#[test]
fn inactive_actor_hourglass_installs_and_advances_idle_wait() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    engine.get_entity_mut(owner).unwrap().element_data_mut().active = false;

    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .is_none(),
        "regression requires Actor::Hourglass to synthesize the idle Wait"
    );

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    engine.tick_actor_animation_action_change_slots(&sim, &assets);

    let entity = engine.get_entity(owner).unwrap();
    assert!(!entity.is_active());
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(seq, index)| engine.orders.sequence_manager.get_element(seq, index))
            .map(|element| element.command),
        Some(Command::Wait),
        "inactive Actor::Hourglass must lazily install the same Wait as an active actor"
    );

    let animated = engine.add_entity(make_test_pc(Posture::Upright));
    let script = SpriteScript {
        action_id: OrderType::WaitingUpright as u16,
        action_done: 3,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3, 4],
        delays: vec![2; 4],
        distances: vec![0; 4],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 4],
        sound_ids: vec![0; 4],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::WaitingUpright as usize] = 0;
    let entity = engine.get_entity_mut(animated).unwrap();
    entity.element_data_mut().active = false;
    entity.actor_data_mut().unwrap().action_state = crate::element::ActionState::Waiting;
    entity.element_data_mut().sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );

    let mut selected = SequenceElement::new(1, Command::Wait, Some(animated));
    selected
        .orders
        .push_back(Order::test_new(OrderType::WaitingUpright, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let (_, _, executed) = engine.tick_actor_animation_for(&sim, &assets, animated);
    let entity = engine.get_entity(animated).unwrap();
    assert!(
        executed.is_some(),
        "inactive Actor::Hourglass must execute the selected idle order"
    );
    assert_eq!(entity.sprite().last_action, OrderType::WaitingUpright);
    assert!(
        entity.sprite().frame_count > 0 || entity.sprite().current_frame > 0,
        "inactive Actor::Hourglass must advance the bound idle animation"
    );
}

#[test]
fn bound_bow_transition_advances_through_production_owner_coordinator() {
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveShot;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};
    use crate::weapons::ShootMode;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let script = SpriteScript {
        action_id: OrderType::TransitionEquipBow as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::TransitionEquipBow as usize] = 0;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    let mut element = SequenceElement::new(1, Command::ShootBow, Some(owner));
    let order = Order::test_new(OrderType::TransitionEquipBow, 0.0, 0.0);
    let order_id = order.order_id;
    element.orders.push_back(order);
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let actor = engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.action_state = ActionState::Waiting;
    actor.active_shot = ActiveShot {
        sequence_id: Some(sequence),
        element_index: 0,
        target: Some(owner),
        order_id: Some(order_id),
        released: false,
        shoot_mode: Some(ShootMode::Normal),
    };
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    positions[owner] = Some(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .position_map(),
    );

    engine.tick_actor_owner_envelopes(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        &positions,
    );

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(entity.sprite().last_action, OrderType::TransitionEquipBow);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::AimingWithBow
    );
}

#[test]
fn unbound_bow_transition_still_uses_generic_execute() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let script = SpriteScript {
        action_id: OrderType::TransitionEquipBow as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::TransitionEquipBow as usize] = 0;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
    element
        .orders
        .push_back(Order::test_new(OrderType::TransitionEquipBow, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    positions[owner] = Some(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .position_map(),
    );

    engine.tick_actor_owner_envelopes(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        &positions,
    );

    assert_eq!(
        engine.get_entity(owner).unwrap().sprite().last_action,
        OrderType::TransitionEquipBow
    );
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot
            .is_active()
    );
}

#[test]
fn execution_frozen_wait_retains_selected_identity_without_entering_execute_arm() {
    use crate::element::{Command, Posture};
    use crate::order::Order;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let mut element = SequenceElement::new(1, Command::WaitTimer, Some(owner));
    let order = Order::test_new(OrderType::WaitingUpright, 0.0, 0.0);
    let order_id = order.order_id;
    element.orders.push_back(order);
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execution_frozen = true;

    let (_, outcomes, result) = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );
    let result = result.expect("frozen wait still returns the base Execute identity");
    assert_eq!(result.entry_seq_id, sequence);
    assert_eq!(result.entry_elem_idx, 0);
    assert_eq!(result.order_type, OrderType::WaitingUpright);
    assert_eq!(result.motion, crate::sprite::MotionState::InProgress);
    assert!(outcomes.seq_advance.is_empty());
    assert_ne!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .sprite
            .last_processed_order_id,
        order_id.get(),
        "per-actor execution freeze returns before the selected sprite call"
    );
}

#[test]
fn terminal_bow_owner_defers_its_exposed_generic_successor_until_next_hourglass() {
    use crate::element::{Command, Posture};
    use crate::movement::ActiveShot;
    use crate::order::Order;
    use crate::sequence::SequenceElement;
    use crate::weapons::ShootMode;

    let sim_context = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let mut element = SequenceElement::new(1, Command::ShootBow, Some(owner));
    let bow_order = Order::test_new(OrderType::ShootingWithBow, 0.0, 0.0);
    let bow_order_id = bow_order.order_id;
    element.orders.push_back(bow_order);
    element
        .orders
        .push_back(Order::test_new(OrderType::WaitingUpright, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_shot = ActiveShot {
        sequence_id: Some(sequence),
        element_index: 0,
        target: Some(owner),
        order_id: Some(bow_order_id),
        released: false,
        shoot_mode: Some(ShootMode::Normal),
    };

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let initial_action = engine
        .get_entity(owner)
        .unwrap()
        .element_data()
        .sprite
        .last_action;

    engine.tick_actor_animation_action_change_slots_with_hooks(
        &sim_context,
        &assets,
        |_, _| {},
        |_, _| {},
        |engine, selected_owner, _, _, bow, _, _| {
            assert_eq!(selected_owner, owner);
            assert_eq!(bow, Some((sequence, 0, bow_order_id)));
            engine
                .orders
                .sequence_manager
                .get_element_mut(sequence, 0)
                .unwrap()
                .pop_current_order();
            engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap()
                .active_shot = ActiveShot::default();
        },
        |_, _| {},
    );

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .sprite
            .last_action,
        initial_action,
        "the successor exposed by terminal bow work must not enter generic Execute in the same owner slot"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .unwrap()
            .2
            .order_type,
        OrderType::WaitingUpright
    );

    let (_, _, next_execute) = engine.tick_actor_animation_for(&sim_context, &assets, owner);
    assert_eq!(
        next_execute.unwrap().order_type,
        OrderType::WaitingUpright,
        "the exposed generic successor must become eligible at the next Execute boundary"
    );
}

#[test]
fn execution_frozen_selected_bow_does_not_advance_or_fire() {
    use crate::element::{Command, Posture};
    use crate::movement::ActiveShot;
    use crate::order::Order;
    use crate::sequence::SequenceElement;
    use crate::weapons::ShootMode;

    let mut engine = EngineInner::new();
    let shooter = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_pc(Posture::Upright));
    let mut element =
        SequenceElement::new_interaction(1, Command::ShootBow, Some(shooter), Some(target));
    let mut order = Order::test_new(OrderType::ShootingWithBow, 0.0, 0.0);
    order.antagonist = Some(target);
    let order_id = order.order_id;
    element.orders.push_back(order);
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let actor = engine
        .get_entity_mut(shooter)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.execution_frozen = true;
    actor.active_shot = ActiveShot {
        sequence_id: Some(sequence),
        element_index: 0,
        target: Some(target),
        order_id: Some(order_id),
        released: false,
        shoot_mode: Some(ShootMode::Normal),
    };
    let before = engine
        .get_entity(shooter)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_shot;

    assert!(
        engine
            .tick_bow_shot_for(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                shooter,
                order_id
            )
            .is_empty()
    );
    assert_eq!(
        engine
            .get_entity(shooter)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot,
        before
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(shooter)
            .unwrap()
            .2
            .order_id,
        order_id
    );
    assert_eq!(
        engine
            .get_entity(shooter)
            .unwrap()
            .sprite()
            .last_processed_order_id,
        u32::MAX
    );
}

#[test]
fn ordered_ability_dispatch_does_not_advance_a_later_actor() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_test_pc(Posture::Upright));
    let second = engine.add_entity(make_test_pc(Posture::Upright));
    for actor_id in [first, second] {
        bind_test_action_point(
            &mut engine,
            actor_id,
            OrderType::Eating,
            crate::coordinates::SpriteLocalPoint::ZERO,
            crate::coordinates::SpriteAnchor::ZERO,
        );
        let sequence_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::EatCmd, Some(actor_id)));
        assert_eq!(
            crate::abilities::begin_eat(
                &mut engine.world.entities,
                &mut engine.orders.sequence_manager,
                actor_id,
                sequence_id,
                0,
                &mut engine.orders.next_order_id,
            ),
            crate::abilities::BeginResult::Started
        );
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
    }

    let mut display = HostDisplayState::default();
    let assets = LevelAssets::new();
    engine.tick_ability_for(sim, &mut display, &assets, first);

    assert_ne!(
        engine
            .get_entity(first)
            .expect("first ability actor present")
            .element_data()
            .sprite
            .last_processed_order_id,
        u32::MAX,
        "the actor at the current creation slot must advance"
    );
    assert_eq!(
        engine
            .get_entity(second)
            .expect("later ability actor present")
            .element_data()
            .sprite
            .last_processed_order_id,
        u32::MAX,
        "a later actor's ability cannot advance from an earlier actor's Hourglass"
    );
}

#[test]
fn active_ability_type_mismatch_is_not_selected_or_allowed_to_suppress_generic_execute() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let mut element = SequenceElement::new(1, Command::EatCmd, Some(owner));
    let order = Order::test_new(OrderType::WaitingUpright, 0.0, 0.0);
    let order_id = order.order_id;
    element.orders.push_back(order);
    let seq_id = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seq_id, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_ability = crate::movement::ActiveAbility {
        kind: Some(crate::movement::AbilityKind::Eat),
        sequence_id: Some(seq_id),
        element_index: 0,
        target: None,
        order_id: Some(order_id),
        done_effect_applied: false,
        strangle_initialized: false,
    };

    let mut observed = None;
    engine.tick_actor_animation_action_change_slots_with_hooks(
        &sim,
        &LevelAssets::new(),
        |_, _| {},
        |_, _| {},
        |_, selected_owner, _, _, _, ability, _| observed = Some((selected_owner, ability)),
        |_, _| {},
    );
    assert_eq!(observed, Some((owner, None)));
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .order_id,
        Some(order_id),
        "a stale type mismatch remains latent and does not execute"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .unwrap()
            .2
            .order_type,
        OrderType::WaitingUpright,
        "the generic selected order remains authoritative"
    );
}

#[test]
fn aborted_ability_cleanup_is_exact_and_allows_later_selection() {
    use crate::element::{Command, Posture};
    use crate::movement::{AbilityKind, ActiveAbility};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceId};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let original = ActiveAbility {
        kind: Some(AbilityKind::Listen),
        sequence_id: Some(SequenceId(41)),
        element_index: 2,
        target: None,
        order_id: Some(std::num::NonZeroU32::new(9).unwrap()),
        done_effect_applied: false,
        strangle_initialized: false,
    };
    let actor = engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.active_ability = original.clone();
    actor.listen_phase = crate::element::ListenPhase::ExitTransition;
    actor.listen_wait_time = 7;
    engine.cleanup_aborted_ability(
        owner,
        AbilityKind::Listen,
        SequenceId(99),
        2,
        original.order_id,
    );
    let retained = &engine
        .get_entity(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_ability;
    assert_eq!(
        (
            retained.kind,
            retained.sequence_id,
            retained.element_index,
            retained.order_id
        ),
        (
            original.kind,
            original.sequence_id,
            original.element_index,
            original.order_id
        ),
        "stale abort must not clear another selected identity"
    );

    engine.cleanup_aborted_ability(
        owner,
        AbilityKind::Listen,
        SequenceId(41),
        2,
        original.order_id,
    );
    let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
    assert!(!actor.active_ability.is_active());
    assert_eq!(actor.listen_phase, crate::element::ListenPhase::Inactive);
    assert_eq!(actor.listen_wait_time, 0);

    // This is an owner-selection fixture, not an Eat validity fixture.
    let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
    let order = Order::test_new(OrderType::Eating, 0.0, 0.0);
    let order_id = order.order_id;
    element.orders.push_back(order);
    let seq = engine.orders.sequence_manager.launch_element(element);
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_ability = ActiveAbility {
        kind: Some(AbilityKind::Eat),
        sequence_id: Some(seq),
        element_index: 0,
        target: None,
        order_id: Some(order_id),
        done_effect_applied: false,
        strangle_initialized: false,
    };
    let mut selected = None;
    engine.tick_actor_animation_action_change_slots_with_hooks(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        |_, _| {},
        |_, _| {},
        |_, id, _, _, _, ability, _| selected = Some((id, ability)),
        |_, _| {},
    );
    assert_eq!(selected, Some((owner, Some((seq, 0, order_id)))));
}

#[test]
fn production_receive_purse_reveals_before_advancing_waiting_order_identity() {
    use crate::element::{Command, Posture, ReceivePursePhase};
    use crate::movement::{AbilityKind, ActiveAbility};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let beggar = engine.add_entity(make_test_civilian(Posture::Upright));
    let Entity::Civilian(civilian) = engine.get_entity_mut(beggar).unwrap() else {
        unreachable!()
    };
    civilian.civilian.beggar_scroll_sets = Some(vec![vec![]]);
    let script = SpriteScript {
        action_id: OrderType::WaitingWithPurse as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2],
        delays: vec![0, 0],
        distances: vec![0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
        sound_ids: vec![0; 2],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::WaitingWithPurse as usize] = 0;
    engine
        .get_entity_mut(beggar)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );
    let mut element = SequenceElement::new(1, Command::ReceivePurse, Some(beggar));
    let waiting = Order::test_new(OrderType::WaitingWithPurse, 0.0, 0.0);
    let waiting_id = waiting.order_id;
    element.orders.push_back(waiting);
    element.orders.push_back(Order::test_new(
        OrderType::TransitionWaitingWithPurseWaitingUpright,
        0.0,
        0.0,
    ));
    let seq = engine.orders.sequence_manager.launch_element(element);
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    let actor = engine
        .get_entity_mut(beggar)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.receive_purse_phase = ReceivePursePhase::Waiting;
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::ReceivePurse),
        sequence_id: Some(seq),
        element_index: 0,
        target: None,
        order_id: Some(waiting_id),
        done_effect_applied: false,
        strangle_initialized: false,
    };

    let observed = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed_hook = observed.clone();
    crate::engine::combat::set_receive_purse_reveal_observer(Some(Box::new(
        move |engine, owner| {
            let (_, _, order) = engine
                .orders
                .sequence_manager
                .current_order_for_actor(owner)
                .expect("ReceivePurse reveal retains its current order");
            observed_hook.set(
                order.order_id == waiting_id && order.order_type == OrderType::WaitingWithPurse,
            );
        },
    )));
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(Default::default());
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for _ in 0..10 {
        engine.tick_actor_owner_envelopes_with_test_owner_hook(
            &crate::sim_rng::test_context(),
            &assets,
            &positions,
            |_, _| {},
        );
        if observed.get() {
            break;
        }
    }
    crate::engine::combat::set_receive_purse_reveal_observer(None);
    assert!(observed.get());
    assert_eq!(
        engine
            .get_entity(beggar)
            .unwrap()
            .actor_data()
            .unwrap()
            .receive_purse_phase,
        ReceivePursePhase::Transition
    );
}

#[test]
fn production_selected_beggar_frozen_turns_and_bids_while_execution_frozen_and_fried_skip() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let beggar = engine.add_entity(make_test_pc(Posture::SimulatingBeggar));
    let donor = engine.add_entity(make_test_civilian(Posture::Upright));
    let donor_actor = engine
        .get_entity_mut(donor)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    donor_actor.action_state = ActionState::Moving;
    let donor_data = engine
        .get_entity_mut(donor)
        .unwrap()
        .npc_data_mut()
        .unwrap();
    donor_data.money = 200;
    engine
        .get_entity_mut(donor)
        .unwrap()
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-5.0, -5.0),
            crate::coordinates::MapVec::new(5.0, 5.0),
        ));
    engine
        .get_entity_mut(beggar)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(1);
    let donor_direction = (0..16)
        .find(|direction| {
            engine
                .get_entity_mut(donor)
                .unwrap()
                .element_data_mut()
                .set_direction_instantly(*direction);
            crate::engine::beggar::can_give_money_to_beggar(&engine, donor, beggar)
        })
        .expect("test geometry has an eligible donor direction");
    engine
        .get_entity_mut(donor)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(donor_direction);
    engine
        .get_entity_mut(beggar)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(0);
    let mut element = SequenceElement::new(1, Command::EnterBeggar, Some(beggar));
    let order = Order::test_new(OrderType::SimulatingBeggar, 0.0, 0.0);
    element.orders.push_back(order);
    let seq = engine.orders.sequence_manager.launch_element(element);
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    engine
        .get_entity_mut(beggar)
        .unwrap()
        .position_iface_mut()
        .set_direction(crate::position_interface::Direction::from_raw(1));
    engine.set_actors_frozen(true);
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);

    for (name, execution_frozen, fried) in
        [("execution_frozen", true, false), ("fried", false, true)]
    {
        let mut gated = engine.clone();
        let Entity::Pc(pc) = gated.get_entity_mut(beggar).unwrap() else {
            unreachable!()
        };
        pc.actor.execution_frozen = execution_frozen;
        pc.pc.fried_psykokwack = fried;
        gated.tick_actor_owner_envelopes_with_test_owner_hook(
            &crate::sim_rng::test_context(),
            &assets,
            &positions,
            |_, _| {},
        );
        assert!(
            !gated
                .world
                .entities
                .occupied()
                .any(|(_, entity)| entity.object_data().is_some_and(|o| o.belongs_to_beggar)),
            "{name} must suppress selected beggar dispatch"
        );
    }

    engine.tick_actor_owner_envelopes_with_test_owner_hook(
        &crate::sim_rng::test_context(),
        &assets,
        &positions,
        |_, _| {},
    );
    assert_eq!(
        engine
            .get_entity(beggar)
            .unwrap()
            .element_data()
            .direction(),
        1
    );
    let coin = engine
        .world
        .entities
        .occupied()
        .find_map(|(id, entity)| {
            entity
                .object_data()
                .is_some_and(|object| object.belongs_to_beggar)
                .then_some(id)
        })
        .expect("FrozenAll Turn is followed by Bid and a live appended coin");
    assert!(
        coin.index() > donor.index(),
        "coin must occupy a later live creation slot"
    );
    assert!(
        engine
            .get_entity(donor)
            .unwrap()
            .npc_data()
            .unwrap()
            .has_given_money_to_beggar
    );
}

#[test]
fn moving_strangle_victim_event_stop_precedes_next_owner_live_initialization() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let _null_handle_slot = engine.add_entity(make_test_pc(Posture::Upright));
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_soldier(Posture::Upright));
    let crate::element::Entity::Soldier(victim_soldier) = engine.get_entity_mut(victim).unwrap()
    else {
        unreachable!()
    };
    victim_soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(0.0, 0.0));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(20.0, 0.0));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::Moving;
    engine.dispatch_ai_stimulus(
        victim,
        crate::ai::Stimulus::new(crate::ai::StimulusType::EventTimer),
    );
    let seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_interaction(
            1,
            Command::StrangleCmd,
            Some(attacker),
            Some(victim),
        ));
    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let element = engine
        .orders
        .sequence_manager
        .get_element(seq, 0)
        .expect("strangle interaction identity must survive synchronous EventStop effects");
    assert_eq!(element.state, SequenceState::InProgress);
    let order = element
        .current_order()
        .expect("strangle order must remain selected");
    let active = &engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_ability;
    assert_eq!(active.sequence_id, Some(seq));
    assert_eq!(active.element_index, 0);
    assert_eq!(active.target, Some(victim));
    assert_eq!(active.order_id, Some(order.order_id));

    let victim_ai = engine.get_entity(victim).unwrap().ai_controller().unwrap();
    assert!(
        !victim_ai
            .locks_flag_field
            .contains(crate::ai::AiLockFlags::FREEZE)
    );
    assert_eq!(
        victim_ai
            .ai_log
            .iter()
            .filter(|line| {
                line.line_type == crate::ai::LogLineType::Event
                    && line.info == crate::ai::StimulusType::EventStop as u16
            })
            .count(),
        1,
        "EventStop Think must complete while FREEZE is still absent",
    );
    assert_eq!(victim_ai.current_state, crate::ai::AiState::Seeking);
    assert_eq!(
        victim_ai.current_substate,
        crate::ai::Substate::SeekingGotStopEvent,
        "EventStop's re-entrant state/timer effects must be applied before FREEZE",
    );
    assert!(victim_ai.timer_is_running);
    assert_eq!(
        victim_ai
            .outbox
            .detection
            .stimuli
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![crate::ai::StimulusType::EventTimer],
        "synchronous EventStop and its re-entrant effects must preserve the older FIFO",
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
        8,
        "translation must not change the attacker direction",
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .direction(),
        8,
        "translation must not change the victim direction",
    );
    assert_eq!(
        i16::from(
            engine
                .get_entity(attacker)
                .unwrap()
                .position_iface()
                .get_direction_goal()
        ),
        8,
        "translation must not eagerly set the attacker goal",
    );
    assert_eq!(
        i16::from(
            engine
                .get_entity(victim)
                .unwrap()
                .position_iface()
                .get_direction_goal()
        ),
        8,
        "translation must not eagerly set the victim goal",
    );

    let mut invalid = engine.clone();
    invalid
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(100.0, 100.0));
    invalid.tick_ability_for(&sim, &mut HostDisplayState::default(), &assets, attacker);
    assert_eq!(
        invalid
            .orders
            .sequence_manager
            .get_element(seq, 0)
            .unwrap()
            .state,
        SequenceState::Impossible,
        "first owner Execute must recheck live Strangle validity",
    );
    assert!(
        !invalid
            .get_entity(victim)
            .unwrap()
            .ai_controller()
            .unwrap()
            .locks_flag_field
            .contains(crate::ai::AiLockFlags::FREEZE)
    );
    assert!(
        !invalid
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
    );

    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(0.0, 20.0));
    let live_facing = crate::position_interface::vector_to_sector_0_to_15_iso(0.0, 20.0);
    engine.tick_ability_for(&sim, &mut display, &assets, attacker);

    let victim_ai = engine.get_entity(victim).unwrap().ai_controller().unwrap();
    assert!(
        victim_ai
            .locks_flag_field
            .contains(crate::ai::AiLockFlags::FREEZE)
    );
    assert_eq!(
        i16::from(
            engine
                .get_entity(attacker)
                .unwrap()
                .position_iface()
                .get_direction_goal()
        ),
        live_facing,
        "first owner Execute must compute the attacker goal from live positions",
    );
    assert_eq!(
        i16::from(
            engine
                .get_entity(victim)
                .unwrap()
                .position_iface()
                .get_direction_goal()
        ),
        live_facing,
        "first owner Execute must compute the victim goal from live positions",
    );
}

#[test]
fn non_stranglable_terminal_retaliation_falls_through_to_cleanup_and_victim_starts_same_done_tick()
{
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    fn bind(engine: &mut EngineInner, id: EntityId, action: OrderType) {
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 0],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;
        engine.get_entity_mut(id).unwrap().element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(conversion),
        );
    }

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let _null_handle_slot = engine.add_entity(make_test_pc(Posture::Upright));
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .active = true;
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .active = true;
    bind(&mut engine, attacker, OrderType::Strangling);
    bind(&mut engine, victim, OrderType::BeingStrangled);
    for id in [attacker, victim] {
        engine
            .get_entity_mut(id)
            .unwrap()
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-5.0, -5.0),
                crate::coordinates::MapVec::new(5.0, 5.0),
            ));
    }
    let mut assets = LevelAssets::new();
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(Default::default());
    profiles.soldiers.push(crate::profiles::SoldierProfile {
        strangle: false,
        ..Default::default()
    });
    assets.profile_manager = std::sync::Arc::new(profiles);
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .npc_data_mut()
        .unwrap()
        .eye_status = crate::element::EyeStatus::LookToTheLeft;
    let seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_interaction(
            1,
            Command::StrangleCmd,
            Some(attacker),
            Some(victim),
        ));
    assert_eq!(
        crate::abilities::begin_strangle(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            attacker,
            victim,
            seq,
            0,
            &mut engine.orders.next_order_id
        ),
        crate::abilities::BeginResult::Started
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_ability
        .strangle_initialized = true;
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    let mut display = HostDisplayState::default();

    for _ in 0..10 {
        engine.tick_ability_for(&sim, &mut display, &assets, attacker);
        if engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .done_effect_applied
        {
            break;
        }
    }
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .done_effect_applied
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .sprite
            .last_action,
        OrderType::BeingStrangled,
        "attacker Done must force the victim animation before its same-invocation increment"
    );
    assert!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .sprite
            .current_frame
            > 0,
        "victim virgin increment must occur during initial attacker Done setup"
    );
    engine.dispatch_ai_stimulus(
        victim,
        crate::ai::Stimulus::new(crate::ai::StimulusType::EventTimer),
    );
    engine.dispatch_ai_stimulus(
        victim,
        crate::ai::Stimulus::new(crate::ai::StimulusType::EventFitAgain),
    );

    let (_, condolation_order) =
        crate::engine::soldier_helpers::capture_strangle_condolation_order(|| {
            for _ in 0..10 {
                engine.tick_ability_for(&sim, &mut display, &assets, attacker);
                if !engine
                    .get_entity(attacker)
                    .unwrap()
                    .actor_data()
                    .unwrap()
                    .active_ability
                    .is_active()
                {
                    break;
                }
            }
        });
    assert_eq!(
        condolation_order,
        [
            "TerminalEventGotHit",
            "Wait",
            "Unlock",
            "EventGotHit",
            "LookForward",
        ],
        "both original EventGotHit handler boundaries must complete synchronously in order"
    );
    assert!(
        !engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active(),
        "non-stranglable retaliation must not return before the following Terminated cleanup"
    );
    assert_ne!(
        engine
            .orders
            .sequence_manager
            .get_element(seq, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress
    );
    let victim_waits: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| {
            sequence
                .elements
                .iter()
                .map(move |element| (sequence.id, element))
        })
        .filter(|(_, element)| element.owner == Some(victim) && element.command == Command::Wait)
        .collect();
    assert_eq!(victim_waits.len(), 1);
    assert!(victim_waits[0].0 > seq);
    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(!victim_entity.ai_controller().unwrap().ai_is_locked());
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        crate::element::EyeStatus::LookForward
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .outbox
            .detection
            .stimuli
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![
            crate::ai::StimulusType::EventTimer,
            crate::ai::StimulusType::EventFitAgain,
        ],
        "both synchronous EventGotHit Thinks must preserve the genuinely pre-existing FIFO in exact order"
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|line| {
                line.line_type == crate::ai::LogLineType::Event
                    && line.info == crate::ai::StimulusType::EventGotHit as u16
            })
            .count(),
        2,
        "the direct retaliation and condolation EventGotHit handlers must both execute synchronously"
    );
    let sequence_count = engine.orders.sequence_manager.sequences_iter().count();
    engine.tick_ability_for(&sim, &mut display, &assets, attacker);
    assert_eq!(
        engine.orders.sequence_manager.sequences_iter().count(),
        sequence_count,
        "retaliation side effects must not repeat after terminal cleanup"
    );
}

#[test]
fn strangle_authorized_placement_failure_cleans_exact_owner_before_post_authorization_effects() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let _null_handle_slot = engine.add_entity(make_test_pc(Posture::Upright));
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_soldier(Posture::Upright));
    assert_ne!(
        attacker.index(),
        0,
        "the attacker must have a non-null legacy AI handle so EventGotHit observation is meaningful"
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .active = true;
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .active = true;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let hotspot = crate::coordinates::SpriteLocalPoint::new(7.0, 9.0);
    let script = SpriteScript {
        action_id: OrderType::Strangling as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::Strangling as usize] = 0;
    let attacker_sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .sprite = attacker_sprite;
    {
        let element = engine.get_entity_mut(attacker).unwrap().element_data_mut();
        element.sprite.current_row = 0;
        element.set_position_map(crate::coordinates::MapPoint::new(100.0, 120.0));
        element.set_layer(3);
        element.set_sector(crate::position_interface::SectorHandle::new(2));
    }
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        let element = victim_entity.element_data_mut();
        element.set_layer(8);
        element.set_sector(crate::position_interface::SectorHandle::new(5));
        element.set_direction_instantly(6);
        victim_entity.npc_data_mut().unwrap().eye_status = crate::element::EyeStatus::LookToTheLeft;
    }
    let expected_action_point = {
        let attacker = engine.get_entity(attacker).unwrap();
        let sprite_pos = attacker.cxx_position_sprite();
        crate::coordinates::MapPoint::new(sprite_pos.x + hotspot.x, sprite_pos.y + hotspot.y)
    };
    let victim_frame_before = engine
        .get_entity(victim)
        .unwrap()
        .element_data()
        .sprite
        .current_frame;
    let sequence_count_before = engine.orders.sequence_manager.sequences_iter().count();
    let seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_interaction(
            1,
            Command::StrangleCmd,
            Some(attacker),
            Some(victim),
        ));
    assert_eq!(
        crate::abilities::begin_strangle(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            attacker,
            victim,
            seq,
            0,
            &mut engine.orders.next_order_id,
        ),
        crate::abilities::BeginResult::Started
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_ability
        .strangle_initialized = true;
    let attacker_topology = {
        let element = engine.get_entity(attacker).unwrap().element_data();
        (
            element.layer(),
            element.sector(),
            element.obstacle_index(),
            element.direction(),
        )
    };
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    let mut display = HostDisplayState::default();

    let (_, condolation_order) =
        crate::engine::soldier_helpers::capture_strangle_condolation_order(|| {
            for _ in 0..10 {
                engine.tick_ability_for(&sim, &mut display, &assets, attacker);
                if !engine
                    .get_entity(attacker)
                    .unwrap()
                    .actor_data()
                    .unwrap()
                    .active_ability
                    .is_active()
                {
                    break;
                }
            }
        });
    assert_eq!(
        condolation_order,
        ["Wait", "Unlock", "EventGotHit", "LookForward"]
    );

    assert!(
        !engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq, 0)
            .unwrap()
            .state,
        SequenceState::Impossible
    );
    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.element_data().position_map(),
        expected_action_point
    );
    assert_eq!(victim_entity.element_data().layer(), 3);
    assert_eq!(
        (
            victim_entity.element_data().layer(),
            victim_entity.element_data().sector(),
            victim_entity.element_data().obstacle_index(),
            victim_entity.element_data().direction(),
        ),
        attacker_topology,
        "failed authorization retains the topology copied before the search"
    );
    assert!(!victim_entity.actor_data().unwrap().execution_frozen);
    assert_ne!(
        victim_entity.element_data().sprite.last_action,
        OrderType::BeingStrangled
    );
    assert_eq!(
        victim_entity.element_data().sprite.current_frame,
        victim_frame_before,
        "failed setup must not virgin-increment the victim"
    );
    assert_eq!(
        engine.orders.sequence_manager.sequences_iter().count(),
        sequence_count_before + 2,
        "failed setup synchronously appends only its required victim Wait"
    );
    let victim_waits: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| {
            sequence
                .elements
                .iter()
                .map(move |element| (sequence.id, element))
        })
        .filter(|(_, element)| element.owner == Some(victim) && element.command == Command::Wait)
        .collect();
    assert_eq!(victim_waits.len(), 1);
    assert!(
        victim_waits[0].0 > seq,
        "condolation Wait must be appended synchronously after its Interaction owner"
    );
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        crate::element::EyeStatus::LookForward
    );
    assert!(
        !victim_entity.ai_controller().unwrap().ai_is_locked(),
        "owner-boundary condolation must unlock the failed victim before returning"
    );
    assert!(
        victim_entity
            .ai_controller()
            .expect("soldier fixture requires an AI controller")
            .outbox
            .reentrant
            .owner_work
            .is_empty(),
        "failed authorization must not enqueue emergency speech owner work"
    );
    let victim_ai = victim_entity.ai_controller().unwrap();
    assert!(
        victim_ai.outbox.detection.stimuli.is_empty(),
        "synchronous EventGotHit Think must finish before tick_ability_for returns"
    );
    assert_eq!(
        victim_ai.primary_target,
        attacker.index(),
        "the victim's EventGotHit handler must observe the attacker at the owner boundary"
    );

    let snapshot = (
        victim_entity.element_data().position_map(),
        victim_entity.element_data().sprite.current_frame,
        engine.orders.sequence_manager.sequences_iter().count(),
    );
    engine.tick_ability_for(&sim, &mut display, &assets, attacker);
    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        (
            victim_entity.element_data().position_map(),
            victim_entity.element_data().sprite.current_frame,
            engine.orders.sequence_manager.sequences_iter().count(),
        ),
        snapshot,
        "a later owner tick must not repeat failed setup effects"
    );
}

#[test]
#[should_panic(expected = "requires Interaction data")]
fn strangle_condolation_rejects_non_interaction_owner_data() {
    use crate::element::{Command, Posture};
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::StrangleCmd, Some(owner)));
    engine.orders.sequence_manager.element_impossible(seq, 0);
    engine.dispatch_condolations_for_owner_boundary(&sim, owner, &LevelAssets::new());
}

#[test]
fn terminal_ability_owner_defers_exposed_generic_successor_until_next_hourglass() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    // This fixture isolates owner identity; the real projectile terminal
    // effect is covered by the production coordinator regression below.
    let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
    let ability_order = Order::test_new(OrderType::ThrowingApple, 0.0, 0.0);
    let ability_id = ability_order.order_id;
    element.orders.push_back(ability_order);
    element
        .orders
        .push_back(Order::test_new(OrderType::WaitingUpright, 0.0, 0.0));
    let seq = engine.orders.sequence_manager.launch_element(element);
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_ability = crate::movement::ActiveAbility {
        kind: Some(crate::movement::AbilityKind::ThrowApple),
        sequence_id: Some(seq),
        element_index: 0,
        target: None,
        order_id: Some(ability_id),
        done_effect_applied: true,
        strangle_initialized: false,
    };
    let initial = engine
        .get_entity(owner)
        .unwrap()
        .element_data()
        .sprite
        .last_action;

    engine.tick_actor_animation_action_change_slots_with_hooks(
        &sim,
        &LevelAssets::new(),
        |_, _| {},
        |_, _| {},
        |engine, selected_owner, _, _, _, ability, _| {
            assert_eq!(
                (selected_owner, ability),
                (owner, Some((seq, 0, ability_id)))
            );
            engine
                .orders
                .sequence_manager
                .get_element_mut(seq, 0)
                .unwrap()
                .pop_current_order();
            engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap()
                .active_ability
                .clear();
        },
        |_, _| {},
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .sprite
            .last_action,
        initial
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .unwrap()
            .2
            .order_type,
        OrderType::WaitingUpright
    );
}

#[test]
fn unbound_ability_catalog_order_still_uses_generic_execute() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let script = SpriteScript {
        action_id: OrderType::ThrowingApple as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::ThrowingApple as usize] = 0;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
    element
        .orders
        .push_back(Order::test_new(OrderType::ThrowingApple, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.tick_actor_animation_action_change_slots(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
    );

    assert_eq!(
        engine.get_entity(owner).unwrap().sprite().last_action,
        OrderType::ThrowingApple
    );
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
    );
}

#[test]
fn production_throw_apple_owner_emits_terminal_projectile_effect() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_pc(Posture::Upright));
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(40.0, 0.0));
    bind_test_action_point(
        &mut engine,
        owner,
        OrderType::ThrowingApple,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    let script = SpriteScript {
        action_id: OrderType::ThrowingApple as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::ThrowingApple as usize] = 0;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    let element =
        SequenceElement::new_interaction(1, Command::ThrowApple, Some(owner), Some(target));
    let sequence = engine.orders.sequence_manager.launch_element(element);
    assert_eq!(
        crate::abilities::begin_throw_apple(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            owner,
            target,
            sequence,
            0,
            &mut engine.orders.next_order_id,
        ),
        crate::abilities::BeginResult::Started
    );
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    for _ in 0..10 {
        let mut positions_before_movement =
            crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (entity_id, entity) in engine.world.entities.occupied() {
            positions_before_movement[entity_id] = Some(entity.element_data().position_map());
        }
        let mut display = HostDisplayState::default();
        engine.tick_actor_owner_envelopes_with_display(
            &sim,
            &mut display,
            &assets,
            &positions_before_movement,
        );
    }

    assert_eq!(engine.world.entities.projectiles().count(), 1);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Terminated
    );
}

#[test]
fn ability_done_emits_once_retains_owner_and_only_terminated_releases() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    bind_test_action_point(
        &mut engine,
        owner,
        OrderType::Eating,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    {
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};
        let script = SpriteScript {
            action_id: OrderType::Eating as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 0],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[OrderType::Eating as usize] = 0;
        engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(conversion),
        );
    }
    let element = SequenceElement::new(1, Command::EatCmd, Some(owner));
    let seq = engine.orders.sequence_manager.launch_element(element);
    assert_eq!(
        crate::abilities::begin_eat(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            owner,
            seq,
            0,
            &mut engine.orders.next_order_id
        ),
        crate::abilities::BeginResult::Started
    );
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(seq, 0)
        .unwrap()
        .orders
        .push_back(Order::test_new(OrderType::WaitingUpright, 0.0, 0.0));
    let order_id = engine
        .get_entity(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_ability
        .order_id;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let mut done_count = 0;
    loop {
        let results = crate::abilities::tick_ability(
            &sim,
            &mut engine.world.entities,
            &engine.orders.sequence_manager,
            owner,
            false,
        );
        done_count += results
            .iter()
            .filter(|result| matches!(result, crate::abilities::AbilityTickResult::EatDone { .. }))
            .count();
        if engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .done_effect_applied
        {
            break;
        }
    }
    assert_eq!(done_count, 1);
    let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
    assert_eq!(actor.active_ability.order_id, order_id);
    assert!(actor.active_ability.done_effect_applied);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .unwrap()
            .2
            .order_id,
        order_id.unwrap()
    );

    let mut display = HostDisplayState::default();
    for _ in 0..10 {
        engine.tick_ability_for(&sim, &mut display, &assets, owner);
        if !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
        {
            break;
        }
    }
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .unwrap()
            .2
            .order_type,
        OrderType::WaitingUpright
    );
}

#[test]
fn production_leave_listen_is_postponed_until_enter_chain_naturally_finishes() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let order_types = [
        OrderType::TransitionWaitingUprightListening,
        OrderType::Listening,
        OrderType::TransitionListeningWaitingUpright,
    ];
    let scripts = order_types
        .iter()
        .map(|order_type| SpriteScript {
            action_id: *order_type as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 0],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
        })
        .collect::<Vec<_>>();
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    for (row, order_type) in order_types.into_iter().enumerate() {
        conversion[order_type as usize] = row as u16;
    }
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(scripts),
        std::sync::Arc::new(conversion),
    );
    let mut assets = LevelAssets::new();
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(crate::profiles::CharacterProfile {
        actions: [
            crate::profiles::Action::Listen,
            crate::profiles::Action::NoAction,
            crate::profiles::Action::NoAction,
        ],
        ..Default::default()
    });
    assets.profile_manager = std::sync::Arc::new(profiles);
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let enter_seq =
        engine.launch_element(SequenceElement::new(1, Command::EnterListen, Some(owner)));
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    for _ in 0..20 {
        engine.perform_hourglass(&mut display, &assets, &mut dev);
        if engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .listen_phase
            == crate::element::ListenPhase::CountingDown
        {
            break;
        }
    }
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .listen_phase,
        crate::element::ListenPhase::CountingDown
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(enter_seq, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .order_type,
        OrderType::Listening
    );

    let listening_order_id = engine
        .orders
        .sequence_manager
        .get_element(enter_seq, 0)
        .unwrap()
        .current_order()
        .unwrap()
        .order_id;
    let leave_seq =
        engine.launch_element(SequenceElement::new(1, Command::LeaveListen, Some(owner)));
    let leave = engine
        .orders
        .sequence_manager
        .get_element(leave_seq, 0)
        .unwrap();
    assert_eq!(leave.state, SequenceState::Postponed);
    assert!(leave.orders.is_empty());
    let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
    assert_eq!(actor.active_ability.sequence_id, Some(enter_seq));
    assert_eq!(actor.active_ability.element_index, 0);
    assert_eq!(actor.active_ability.order_id, Some(listening_order_id));
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(enter_seq, 0)
            .unwrap()
            .state,
        SequenceState::InProgress,
        "LeaveListen must not replace the non-interruptable EnterListen owner"
    );

    let mut saw_enter_exit = false;
    for _ in 0..80 {
        engine.perform_hourglass(&mut display, &assets, &mut dev);
        saw_enter_exit |= engine
            .orders
            .sequence_manager
            .get_element(enter_seq, 0)
            .and_then(|element| element.current_order())
            .is_some_and(|order| order.order_type == OrderType::TransitionListeningWaitingUpright);
        if !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
        {
            break;
        }
    }
    assert!(
        saw_enter_exit,
        "EnterListen must own its existing exit order"
    );
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(enter_seq, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .listen_phase,
        crate::element::ListenPhase::Inactive
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        crate::element::ActionState::Waiting
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .is_registered_to_go(leave_seq, 0),
        "released LeaveListen must be registered for production re-dispatch"
    );
    for _ in 0..20 {
        engine.perform_hourglass(&mut display, &assets, &mut dev);
        if engine
            .orders
            .sequence_manager
            .get_element(leave_seq, 0)
            .unwrap()
            .state
            == SequenceState::Impossible
        {
            break;
        }
    }
    let leave = engine
        .orders
        .sequence_manager
        .get_element(leave_seq, 0)
        .unwrap();
    assert_eq!(leave.state, SequenceState::Impossible);
    assert!(
        !engine
            .orders
            .sequence_manager
            .is_registered_to_go(leave_seq, 0),
        "released LeaveListen action must be consumed exactly once"
    );
    assert!(leave.orders.is_empty());
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active(),
        "released LeaveListen must not install stale ability ownership"
    );
}

#[test]
fn melee_completion_precedes_a_later_ability_dispatch() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::weapons::SwordStrike;

    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let later_actor = engine.add_entity(make_test_pc(Posture::Upright));

    let melee_sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::SwordstrikeThrustA,
            Some(attacker),
        ));
    engine
        .orders
        .sequence_manager
        .element_in_progress(melee_sequence, 0);
    let mut active_melee =
        crate::movement::ActiveMelee::new(later_actor, SwordStrike::A, Some(melee_sequence), 0);
    active_melee.frames_remaining = 1;
    active_melee.hit_applied = true;
    engine
        .get_entity_mut(attacker)
        .expect("attacker present")
        .actor_data_mut()
        .expect("attacker actor data")
        .active_melee = active_melee;

    bind_test_action_point(
        &mut engine,
        later_actor,
        OrderType::Eating,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    let ability_sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::EatCmd, Some(later_actor)));
    assert_eq!(
        crate::abilities::begin_eat(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            later_actor,
            ability_sequence,
            0,
            &mut engine.orders.next_order_id,
        ),
        crate::abilities::BeginResult::Started
    );
    engine
        .orders
        .sequence_manager
        .element_in_progress(ability_sequence, 0);

    let assets = LevelAssets::new();
    engine.tick_melee_completion_for(sim, &assets, attacker);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(melee_sequence, 0)
            .expect("melee sequence present")
            .state,
        SequenceState::Terminated,
    );
    assert_eq!(
        engine
            .get_entity(later_actor)
            .expect("later ability actor present")
            .element_data()
            .sprite
            .last_processed_order_id,
        u32::MAX,
        "completing an earlier melee actor must not globally advance a later ability"
    );
}

fn install_owner_selected_test_melee(
    engine: &mut EngineInner,
    attacker: EntityId,
    target: EntityId,
    strike: crate::weapons::SwordStrike,
    order_type: crate::order::OrderType,
    frames_remaining: u16,
    hit_applied: bool,
) {
    let sequence =
        engine
            .orders
            .sequence_manager
            .launch_element(crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::SwordstrikeThrustA,
                Some(attacker),
            ));
    let order_id = engine.orders.allocate_order_id();
    engine.orders.sequence_manager.push_order_on(
        sequence,
        0,
        crate::order::Order::new(order_type, 0.0, 0.0, order_id),
    );
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let mut active = crate::movement::ActiveMelee::new(target, strike, Some(sequence), 0);
    active.frames_remaining = frames_remaining;
    active.hit_applied = hit_applied;
    active.order_id = Some(order_id);
    engine
        .get_entity_mut(attacker)
        .expect("selected melee test attacker exists")
        .actor_data_mut()
        .expect("selected melee test attacker has actor data")
        .active_melee = active;
}

fn chained_straight_strike_target_life(interrupter_first: bool) -> i16 {
    use crate::coordinates::WorldPoint3D;
    use crate::element::Posture;
    use crate::movement::{MELEE_HIT_FRAME, MELEE_STRIKE_DURATION};
    use crate::profiles::{CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile};
    use crate::weapons::SwordStrike;

    fn position(entity: &mut Entity, x: f32) {
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position(WorldPoint3D { x, y: 0.0, z: 0.0 });
        entity
            .element_data_mut()
            .set_position_map(MapPoint { x, y: 0.0 });
    }

    let mut engine = EngineInner::new();
    let mut interrupter = make_test_pc(Posture::Upright);
    position(&mut interrupter, 0.0);
    let mut chained_attacker = make_test_soldier(Posture::Upright);
    position(&mut chained_attacker, 20.0);
    let Entity::Soldier(soldier) = &mut chained_attacker else {
        unreachable!();
    };
    soldier.npc.life_points = 1;
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test soldier has enemy AI")
        .hth_weapon_id = 1;
    soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
    let mut final_target = make_test_pc(Posture::Upright);
    position(&mut final_target, 40.0);
    let Entity::Pc(pc) = &mut final_target else {
        unreachable!();
    };
    pc.pc.life_points = 50;

    let (interrupter_id, chained_attacker_id) = if interrupter_first {
        (
            engine.add_entity(interrupter),
            engine.add_entity(chained_attacker),
        )
    } else {
        let chained_attacker_id = engine.add_entity(chained_attacker);
        let interrupter_id = engine.add_entity(interrupter);
        (interrupter_id, chained_attacker_id)
    };
    let final_target_id = engine.add_entity(final_target);
    engine
        .get_entity_mut(chained_attacker_id)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .me = chained_attacker_id.index();

    for (attacker, target) in [
        (interrupter_id, chained_attacker_id),
        (chained_attacker_id, final_target_id),
    ] {
        install_owner_selected_test_melee(
            &mut engine,
            attacker,
            target,
            SwordStrike::A,
            crate::order::OrderType::StrikingStraightSword,
            MELEE_STRIKE_DURATION - MELEE_HIT_FRAME,
            false,
        );
    }

    let mut profiles = ProfileManager::new();
    let mut weapon = HtHWeaponProfile::default();
    weapon.thrusts[SwordStrike::A as usize].minimal_distance = 0;
    weapon.thrusts[SwordStrike::A as usize].maximal_distance = 100;
    weapon.thrusts[SwordStrike::A as usize].cutting = 100;
    profiles.hth_weapons.push(weapon);
    profiles.characters.push(CharacterProfile {
        hth_weapon_id: 1,
        ..CharacterProfile::default()
    });
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        ..SoldierProfile::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };
    crate::sim_rng::with_seed(0xA_B_C, |sim| {
        let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        engine.tick_actor_owner_envelopes(sim, &assets, &positions);
    });

    let Entity::Pc(target) = engine
        .get_entity(final_target_id)
        .expect("final chained-strike target present")
    else {
        panic!("final chained-strike target must be a PC");
    };
    target.pc.life_points
}

#[test]
fn straight_strike_damage_interrupts_only_later_creation_slots() {
    assert_eq!(
        chained_straight_strike_target_life(true),
        50,
        "an earlier attacker's synchronous damage must stop the later actor before its strike"
    );
    assert!(
        chained_straight_strike_target_life(false) < 50,
        "a chained attacker that already ran this frame must hit before the later interruption"
    );
}

#[derive(Clone, Copy)]
enum NonstraightInterrupt {
    Lateral,
    Push,
}

fn chained_nonstraight_strike_lives(
    interrupt: NonstraightInterrupt,
    interrupter_first: bool,
) -> (i16, i16) {
    use crate::coordinates::{MapVec, MoveBox, WorldPoint3D};
    use crate::element::Posture;
    use crate::movement::{MELEE_HIT_FRAME, MELEE_STRIKE_DURATION, SweepState};
    use crate::profiles::{
        CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile, WeaponThrustDirection,
        WeaponThrustKind,
    };
    use crate::weapons::SwordStrike;

    fn position(entity: &mut Entity, x: f32, y: f32) {
        let element = entity.element_data_mut();
        element.active = true;
        element.set_position(WorldPoint3D { x, y, z: 0.0 });
        element.set_position_map(MapPoint { x, y });
        element.set_direction_instantly(0);
        element
            .sprite
            .position_iface
            .set_move_box(MoveBox::from_corners(
                MapVec::new(-5.0, -5.0),
                MapVec::new(5.0, 5.0),
            ));
    }

    let mut engine = EngineInner::new();
    let mut interrupter = make_test_pc(Posture::Upright);
    position(&mut interrupter, 0.0, 100.0);
    let mut chained_attacker = make_test_soldier(Posture::Upright);
    position(&mut chained_attacker, 0.0, 50.0);
    let Entity::Soldier(soldier) = &mut chained_attacker else {
        unreachable!();
    };
    soldier.npc.life_points = 1;
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test soldier has enemy AI")
        .hth_weapon_id = 1;
    soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
    let mut final_target = make_test_pc(Posture::Upright);
    // Remain within the chained attacker's 100-unit straight range but
    // outside the interrupter's 100x100 push rectangle (half-width 50).
    position(&mut final_target, 60.0, 50.0);
    let Entity::Pc(pc) = &mut final_target else {
        unreachable!();
    };
    pc.pc.life_points = 50;

    let (interrupter_id, chained_attacker_id) = if interrupter_first {
        (
            engine.add_entity(interrupter),
            engine.add_entity(chained_attacker),
        )
    } else {
        let chained_attacker_id = engine.add_entity(chained_attacker);
        let interrupter_id = engine.add_entity(interrupter);
        (interrupter_id, chained_attacker_id)
    };
    let final_target_id = engine.add_entity(final_target);
    engine
        .get_entity_mut(chained_attacker_id)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .me = chained_attacker_id.index();

    install_owner_selected_test_melee(
        &mut engine,
        chained_attacker_id,
        final_target_id,
        SwordStrike::A,
        crate::order::OrderType::StrikingStraightSword,
        MELEE_STRIKE_DURATION - MELEE_HIT_FRAME,
        false,
    );

    match interrupt {
        NonstraightInterrupt::Lateral => {
            install_owner_selected_test_melee(
                &mut engine,
                interrupter_id,
                chained_attacker_id,
                SwordStrike::D,
                crate::order::OrderType::StrikingLeftSword,
                2,
                true,
            );
            engine
                .get_entity_mut(interrupter_id)
                .expect("lateral attacker present")
                .actor_data_mut()
                .expect("lateral attacker has actor data")
                .sweep_state = Some(SweepState {
                pending_victims: vec![chained_attacker_id],
                initial_angle: 0.0,
                current_angle: 0.0,
                final_angle: std::f32::consts::FRAC_PI_2,
                rotation_per_frame: 0.1,
                direction: WeaponThrustDirection::LeftToRight,
                strike: SwordStrike::D,
                attacker_profile_idx: Some(1),
                strike_kind: WeaponThrustKind::Lateral,
            });
        }
        NonstraightInterrupt::Push => {
            install_owner_selected_test_melee(
                &mut engine,
                interrupter_id,
                chained_attacker_id,
                SwordStrike::D,
                crate::order::OrderType::StrikingLeftSword,
                MELEE_STRIKE_DURATION - MELEE_HIT_FRAME,
                false,
            );
        }
    }

    let mut profiles = ProfileManager::new();
    let mut weapon = HtHWeaponProfile::default();
    let straight = &mut weapon.thrusts[SwordStrike::A as usize];
    straight.minimal_distance = 0;
    straight.maximal_distance = 100;
    straight.cutting = 100;
    let nonstraight = &mut weapon.thrusts[SwordStrike::D as usize];
    nonstraight.kind = match interrupt {
        NonstraightInterrupt::Lateral => WeaponThrustKind::Lateral,
        NonstraightInterrupt::Push => WeaponThrustKind::PushAside,
    };
    nonstraight.direction = WeaponThrustDirection::LeftToRight;
    nonstraight.minimal_distance = 0;
    nonstraight.maximal_distance = 100;
    nonstraight.repulsion = 100;
    nonstraight.cutting = 100;
    profiles.hth_weapons.push(weapon);
    profiles.characters.push(CharacterProfile {
        hth_weapon_id: 1,
        ..CharacterProfile::default()
    });
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        ..SoldierProfile::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };
    crate::sim_rng::with_seed(0xD_E_F, |sim| {
        let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        engine.tick_actor_owner_envelopes(sim, &assets, &positions);
    });

    let Entity::Pc(target) = engine
        .get_entity(final_target_id)
        .expect("final chained-strike target present")
    else {
        panic!("final chained-strike target must be a PC");
    };
    let final_target_life = target.pc.life_points;
    let Entity::Soldier(chained_attacker) = engine
        .get_entity(chained_attacker_id)
        .expect("interrupted chained attacker remains present")
    else {
        panic!("chained attacker must remain a soldier");
    };
    (final_target_life, chained_attacker.npc.life_points)
}

#[test]
fn hourglass_nonstraight_damage_interrupts_only_later_creation_slots() {
    for (interrupt, label) in [
        (NonstraightInterrupt::Lateral, "lateral"),
        (NonstraightInterrupt::Push, "push"),
    ] {
        let (final_life, interrupted_life) = chained_nonstraight_strike_lives(interrupt, true);
        assert!(
            interrupted_life <= 0,
            "the earlier {label} must synchronously mutate the later victim"
        );
        assert_eq!(
            final_life, 50,
            "an earlier lethal {label} must stop the later actor before its strike"
        );
        let (final_life, interrupted_life) = chained_nonstraight_strike_lives(interrupt, false);
        assert!(
            interrupted_life <= 0,
            "the later-created {label} must still land after the chained attacker's slot"
        );
        assert!(
            final_life < 50,
            "the chained actor must hit before a later-created lethal {label} interrupts it"
        );
    }
}

#[test]
fn entity_slot_order_is_append_only_and_survives_save_round_trip() {
    let mut engine = EngineInner::new();
    let first = engine.add_entity(Entity::Scroll(crate::element::ElementScroll::default()));
    let second = engine.add_entity(Entity::Scroll(crate::element::ElementScroll::default()));

    engine.remove_entity(first);
    let third = engine.add_entity(Entity::Scroll(crate::element::ElementScroll::default()));

    assert_eq!(first.index(), 0);
    assert_eq!(second.index(), 1);
    assert_eq!(third.index(), 2, "removed slots must never be reused");
    assert_eq!(
        engine
            .world
            .entities
            .occupied()
            .map(|(id, _)| id.index())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );

    let encoded = serde_json::to_string(&engine.world.entities).expect("serialize entity slots");
    let decoded: crate::entities::Entities =
        serde_json::from_str(&encoded).expect("deserialize entity slots");
    assert_eq!(
        decoded
            .occupied()
            .map(|(id, _)| id.index())
            .collect::<Vec<_>>(),
        vec![1, 2],
        "save loading must preserve slot/creation order and holes"
    );
}

#[test]
fn hourglass_advances_mission_length_from_sim_seconds() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Campaign::new();

    for _ in 0..25 {
        let result = engine
            .perform_hourglass(&mut display, &assets, &mut dev)
            .code;
        assert_eq!(result, GameCode::LevelInProgress);
    }

    assert_eq!(engine.control.frame_counter, 25);
    assert_eq!(
        engine
            .mission_domain
            .campaign
            .get_value(CampaignValue::MissionLength),
        1
    );
}

#[test]
fn fade_to_black_presents_without_advancing_simulation_timers() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.control.frame_counter = 25;

    let mut campaign = Campaign::new();
    campaign.set_value(CampaignValue::MissionLength, 7);
    engine.mission_domain.campaign = campaign;

    engine
        .feedback
        .sound_sim
        .sources
        .sources_push_some(crate::sound_source::SoundSource {
            source_kind: crate::sound_source::SoundSourceKind::Delayed,
            timer: 9,
            active: true,
            ..Default::default()
        });

    engine.apply_host_commands(
        sim,
        &assets,
        vec![crate::natives::EngineCommand::FadeToBlack { speed: 3 }],
    );

    let fade = engine
        .feedback
        .pending_side_effects
        .fade_to_black
        .take()
        .flatten()
        .expect("fade command should emit a host ramp");
    assert_eq!(fade.frames_remaining, 6);
    assert_eq!(engine.fade_freeze_frames_remaining(), 5);

    // The trigger tick presents the first of six frames. Each of the five
    // subsequent hourglass calls represents one more presentation, but is
    // not a simulation tick in the original game.
    for expected_remaining in (0..5).rev() {
        let side_effects = engine.perform_hourglass(&mut display, &assets, &mut dev);
        assert_eq!(side_effects.code, GameCode::LevelInProgress);
        assert!(!side_effects.skip_render);
        assert_eq!(engine.fade_freeze_frames_remaining(), expected_remaining);
        assert_eq!(engine.control.frame_counter, 25);
        assert_eq!(
            engine
                .mission_domain
                .campaign
                .get_value(CampaignValue::MissionLength),
            7
        );
        assert_eq!(engine.feedback.sound_sim.sources.get(0).unwrap().timer, 9);
    }

    // The next call is the first real simulation tick after the blocking
    // fade and resumes every clock from exactly its pre-fade value.
    engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert_eq!(engine.control.frame_counter, 26);
    assert_eq!(engine.feedback.sound_sim.sources.get(0).unwrap().timer, 8);
}

#[test]
fn fade_to_black_host_countdown_advances_once_per_presented_frame() {
    let mut fade = crate::engine::types::FadeToBlack {
        speed: 2,
        frames_remaining: 4,
    };

    for expected_remaining in [3, 2, 1] {
        assert!(fade.advance_presented_frame());
        assert_eq!(fade.frames_remaining, expected_remaining);
    }
    assert!(!fade.advance_presented_frame());
    assert_eq!(fade.frames_remaining, 0);
    assert!(!fade.advance_presented_frame());
}

#[test]
fn enter_helping_climb_sequence_dispatches_stealth_transition() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let mut assets = LevelAssets::new();
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(crate::profiles::CharacterProfile {
        actions: [
            crate::profiles::Action::HelpToClimb,
            crate::profiles::Action::NoAction,
            crate::profiles::Action::NoAction,
        ],
        ..Default::default()
    });
    assets.profile_manager = std::sync::Arc::new(profiles);
    let mut engine = EngineInner::new();

    let pc_id = engine.add_entity(crate::element::Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            posture: crate::element::Posture::Upright,
            ..Default::default()
        },
        actor: crate::element::ActorData {
            action_state: crate::element::ActionState::Waiting,
            ..Default::default()
        },
        human: Default::default(),
        pc: crate::element::PcData {
            life_points: 50,
            ..Default::default()
        },
    }));

    let elem = crate::sequence::SequenceElement::new(
        1,
        crate::element::Command::EnterHelpingClimb,
        Some(pc_id),
    );
    engine.launch_element(elem);
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;

    assert_eq!(result, GameCode::LevelInProgress);
    let pc = engine.get_entity(pc_id).expect("pc still exists");
    assert_eq!(
        pc.element_data().posture,
        crate::element::Posture::HelpingToClimb
    );
    assert_eq!(
        pc.actor_data().unwrap().action_state,
        crate::element::ActionState::Waiting
    );
}

#[test]
fn hourglass_quit_won() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.state.quit_won = true;
    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelSucceeded);
}

#[test]
fn hourglass_quit_lost() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.state.quit_lost = true;
    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelFailed);
}

#[test]
fn hourglass_quit_interrupted() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.state.quit_interrupted = true;
    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelInterrupted);
}

#[test]
fn hourglass_locked_skips_logic() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.set_engine_locked(true);
    // Even with a chorus timer, lock should prevent it from being decremented
    // (actually, chorus timer IS decremented before the lock check)
    engine.control.chorus_timer = 5;
    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelInProgress);
    // Chorus timer still decremented (it's before the lock check)
    assert_eq!(engine.control.chorus_timer, 4);
    // But frame counter is still incremented
    assert_eq!(engine.control.frame_counter, 1);
}

#[test]
fn fast_forward() {
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.camera_slide = crate::coordinates::MapPoint::new(100.0, 200.0);
    engine.set_fast_forward();
    assert!(engine.is_fast_forward());
    // Camera should have jumped to slide target
    assert_eq!(engine.feedback.cutscene_camera.view_position.x, 100.0);
    assert_eq!(engine.feedback.cutscene_camera.view_position.y, 200.0);
    // Slide should be deactivated
    assert!(!engine.feedback.cutscene_camera.is_sliding());
}

/// Rollback determinism: clone the engine mid-run, advance the clone and
/// the original the same number of ticks, and verify they end up in the
/// same state. This is the foundation test for rollback multiplayer — if
/// it ever fails, determinism is broken somewhere in the tick path.
///
/// We advance past `frame_counter % 25 == 0` (the script-hourglass
/// boundary) a few times to exercise the scripted slow path as well as
/// the regular frame path, and we seed the RNG to a non-zero state so
/// any RNG consumer during the tick would diverge between seeded and
/// un-seeded paths.
///
/// This will grow as more sim surface comes online — right now there are
/// no entities, so it mostly exercises frame counters, script ticks,
/// chorus timer, mission state, and the RNG/sound-queue plumbing.
#[test]
fn rollback_clone_stays_in_sync() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let seed = 0xDEAD_BEEF_CAFE_BABE;

    let mut original = EngineInner::new();
    original.restore_rng_from_seed(seed);

    // Warm up a few ticks so the clone is taken from a non-initial state.
    for _ in 0..30 {
        original.perform_hourglass(&mut display, &assets, &mut dev);
    }

    // Snapshot now. This is the rollback-from point.
    let snapshot = original.clone();

    // Advance both copies by the same number of ticks.
    let mut replay = snapshot.clone();
    for _ in 0..50 {
        original.perform_hourglass(&mut display, &assets, &mut dev);
        replay.perform_hourglass(&mut display, &assets, &mut dev);
    }

    assert_eq!(original.control.frame_counter, replay.control.frame_counter);
    assert_eq!(original.rng_seed(), replay.rng_seed());
    assert_eq!(original.control.chorus_timer, replay.control.chorus_timer);
    assert_eq!(
        original.mission_domain.state.mission_won,
        replay.mission_domain.state.mission_won
    );
    assert_eq!(original.scripts.globals, replay.scripts.globals);

    // Double-check: re-cloning the original snapshot and replaying the
    // SAME number of ticks a second time must also match — guarding
    // against state that silently leaks across clones (e.g. an RNG allocation
    // accidentally shared between the snapshot and the live engine).
    let mut second_replay = snapshot;
    for _ in 0..50 {
        second_replay.perform_hourglass(&mut display, &assets, &mut dev);
    }
    assert_eq!(
        second_replay.control.frame_counter,
        original.control.frame_counter
    );
    assert_eq!(second_replay.rng_seed(), original.rng_seed());
}

/// `RHGame::GameLoop` calls mission `PostInitialize` only after its
/// first `Refresh(true, true)` and `RHSound::Hourglass` calls.  Keep the
/// engine tick and that host-owned boundary observably separate: frame
/// zero must finish without flipping the serialized one-shot flag, and
/// the explicit post-refresh stage must flip it without advancing time.
#[test]
fn post_initialize_waits_for_post_refresh_stage() {
    use crate::scb::{ClassEntry, Function, ScbFile};
    use crate::vm::{Opcode, Quad};

    let begin = Quad {
        operation: Opcode::BeginFunction as u8,
        operands: [0; 8],
    };
    let ret = Quad {
        operation: Opcode::Return as u8,
        operands: [0; 8],
    };
    let startup = ClassEntry {
        source_file: "post_initialize_ordering_test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "PostInitialize".into(),
            address: 0,
            num_parameters: 0,
            size_of_return_value: 0,
            size_of_parameters: 0,
            size_of_volatile: 0,
            size_of_temporary: 0,
        }],
        quads: vec![begin, ret],
    };

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.scripts.mission = Some(
        MissionScript::from_scb(ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![startup],
        })
        .expect("synthetic StartUp script"),
    );
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert_eq!(
        engine.control.frame_counter, 1,
        "the first simulation frame ran"
    );
    assert!(
        !engine.scripts.mission.as_ref().unwrap().post_initialized,
        "PostInitialize must not run before the first host refresh and sound hourglass"
    );

    let rng_seed_before_post_initialize = engine.rng_seed();
    let first_post_initialize_effects = engine.perform_post_initialize(&mut display, &assets);
    assert!(first_post_initialize_effects.is_some());
    assert_eq!(
        engine.rng_seed(),
        rng_seed_before_post_initialize,
        "an empty PostInitialize must reclaim the unchanged simulation RNG stream"
    );
    assert_eq!(
        engine.control.frame_counter, 1,
        "the post-refresh stage must not advance simulation time"
    );
    assert!(
        engine.scripts.mission.as_ref().unwrap().post_initialized,
        "the post-refresh stage must dispatch PostInitialize exactly at the frame-one boundary"
    );

    let second_post_initialize_effects = engine.perform_post_initialize(&mut display, &assets);
    assert!(second_post_initialize_effects.is_none());
    assert_eq!(engine.control.frame_counter, 1);
    assert!(engine.scripts.mission.as_ref().unwrap().post_initialized);
}
