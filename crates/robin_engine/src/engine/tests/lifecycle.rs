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
        |_, _, _| {},
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
fn inactive_unsupported_projectile_mapping_panics_before_owner_slot_retention() {
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
fn inactive_unsupported_net_mapping_panics_before_owner_slot_retention() {
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

    assert!(engine.get_entity(apple).is_some());
    assert!(engine.get_entity(stone).is_some());
    assert!(engine.get_entity(flying_purse).is_some());
    assert!(engine.get_entity(grounded_purse).is_some());
    assert!(engine.get_entity(grounded_coin).is_some());
    assert!(engine.get_entity(flying_coin).is_some());
    assert!(engine.get_entity(grounded_net).is_some());
    assert!(engine.get_entity(flying_net).is_some());
    for id in [apple, stone, flying_purse] {
        assert!(
            !engine.get_entity(id).unwrap().is_active(),
            "{id:?} must remain as an inactive tombstone"
        );
    }
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
        "each inactive derived sprite tail must run before its virtual bool controls tombstone retention"
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
fn grounded_arrow_exposes_terminal_active_frame_then_refresh_retires_its_slot() {
    use crate::element::{
        Animation, ElementData, ElementKind, ElementProjectile, ObjectData, ObjectType,
        ProjectileData,
    };

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let arrow = engine.add_entity(Entity::Projectile(ElementProjectile {
        element: ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..Default::default()
        },
        object: ObjectData {
            object_type: ObjectType::Arrow,
            animation: Animation::ObjectFlying,
            ..Default::default()
        },
        projectile: ProjectileData {
            flying: false,
            ..Default::default()
        },
    }));
    {
        let Entity::Projectile(projectile) = engine.get_entity_mut(arrow).unwrap() else {
            unreachable!()
        };
        projectile
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(12.0, 8.0, 4.0));
        projectile
            .element
            .sprite
            .position_iface
            .set_old_position(crate::coordinates::WorldPoint3D::ZERO);
        projectile
            .element
            .sprite
            .position_iface
            .set_old_map_position(crate::coordinates::MapPoint::ZERO);
    }
    let assets = LevelAssets::new();

    engine.tick_projectile_or_net_hourglass(&sim, &assets, arrow);
    let Entity::Projectile(projectile) = engine.get_entity(arrow).unwrap() else {
        unreachable!()
    };
    assert!(
        projectile.element.active,
        "Original records terminal Hourglass state before arrow Refresh"
    );
    assert!(
        !projectile.element.sprite.position_iface.is_moving(),
        "active non-flying Projectile::Hourglass still calls NewMove"
    );
    engine.control.arrow_refresh_pending = true;
    engine.apply_pending_arrow_refresh(&sim);
    let Entity::Projectile(projectile) = engine.get_entity(arrow).unwrap() else {
        unreachable!()
    };
    assert!(
        !projectile.element.active,
        "the between-frame Refresh must retire a stationary empty arrow"
    );

    engine.tick_projectile_or_net_hourglass(&sim, &assets, arrow);
    assert!(
        engine.get_entity(arrow).is_some(),
        "inactive arrow must remain as a tombstone"
    );
}

#[test]
fn successful_projectile_human_hit_rewind_settles_and_deletes_trajectory() {
    use crate::element::{
        ElementData, ElementKind, ElementProjectile, ObjectData, ObjectType, ProjectileData,
        TrajectoryPoint,
    };

    let mut engine = EngineInner::new();
    let old = crate::coordinates::WorldPoint3D::new(12.0, 8.0, 4.0);
    let projectile = engine.add_entity(Entity::Projectile(ElementProjectile {
        element: ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..Default::default()
        },
        object: ObjectData {
            object_type: ObjectType::Arrow,
            ..Default::default()
        },
        projectile: ProjectileData {
            flying: false,
            trajectory: vec![TrajectoryPoint {
                position: crate::coordinates::WorldPoint3D::new(20.0, 10.0, 2.0),
                time: 4,
            }],
            ..Default::default()
        },
    }));

    engine.rewind_projectile_to_human_hit_old_position(projectile, old);
    let Entity::Projectile(projectile) = engine.get_entity(projectile).unwrap() else {
        unreachable!()
    };
    assert!(projectile.projectile.trajectory.is_empty());
    assert_eq!(projectile.element.position(), old);
    assert!(!projectile.element.sprite.position_iface.is_moving());
    assert_eq!(
        projectile.element.position_map(),
        projectile.element.sprite.position_iface.old_map_position()
    );
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
    use crate::sprite::MotionState;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .active = false;

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

    let mut executed_idle = false;
    let mut forwarded_termination = false;
    for _ in 0..16 {
        let (_, _, executed) = engine.tick_actor_animation_for(&sim, &assets, animated);
        executed_idle |= executed.is_some();
        forwarded_termination |= executed
            .as_ref()
            .is_some_and(|result| result.motion == MotionState::Terminated);
        if forwarded_termination {
            break;
        }
    }
    let entity = engine.get_entity(animated).unwrap();
    assert!(
        executed_idle,
        "inactive Actor::Hourglass must execute the selected idle order"
    );
    assert_eq!(entity.sprite().last_action, OrderType::WaitingUpright);
    assert!(
        forwarded_termination,
        "WaitingUpright must forward sprite termination so Hourglass can advance into the bored transition"
    );
}

#[test]
fn unconscious_tied_wait_keeps_advancing_its_hold_animation() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite::MotionState;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Tied));
    let mut selected = SequenceElement::new(1, Command::Wait, Some(owner));
    let order = Order::test_new(OrderType::BeingTied, 0.0, 0.0);
    let order_id = order.order_id;
    selected.orders.push_back(order);
    let sequence = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let script = SpriteScript {
        action_id: OrderType::BeingTied as u16,
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
    conversion[OrderType::BeingTied as usize] = 0;
    let entity = engine.get_entity_mut(owner).unwrap();
    entity.human_data_mut().unwrap().unconscious = true;
    entity.actor_data_mut().unwrap().action_state = ActionState::Waiting;
    entity.element_data_mut().sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );
    entity.element_data_mut().sprite.last_action = OrderType::BeingTied;
    entity.element_data_mut().sprite.last_processed_order_id = order_id.get();

    let (_, _, executed) = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );

    assert_eq!(
        executed.map(|result| result.motion),
        Some(MotionState::InProgress),
        "BeingTied is a live Human::Execute hold even though tied humans carry the unconscious flag"
    );
    assert_eq!(
        engine.get_entity(owner).unwrap().sprite().frame_count,
        1,
        "the tied hold must retain Original's per-Hourglass PerformAction tick"
    );
}

#[test]
fn move_ok_bored_exit_transition_uses_generic_actor_execute() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::Bored;

    let transition = OrderType::TransitionWaitingUprightBoredWaitingUpright;
    let script = SpriteScript {
        action_id: transition as u16,
        action_done: 2,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![2; 3],
        distances: vec![0; 3],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[transition as usize] = 0;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );

    let mut selected =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    selected
        .orders
        .push_back(Order::test_new(transition, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let mut executed_transition = false;
    for _ in 0..16 {
        let (_, _, executed) = engine.tick_actor_animation_for(&sim, &assets, owner);
        executed_transition |= executed.is_some();
        if engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state
            == ActionState::Waiting
        {
            break;
        }
    }
    assert!(
        executed_transition,
        "a transition selected as GenericAnimation must not be suppressed merely because its element carries Movement data"
    );
    assert_eq!(
        engine.get_entity(owner).unwrap().sprite().last_action,
        transition
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        ActionState::Waiting,
        "bored-exit completion inside MoveOk must apply the base-Actor state transition"
    );
}

#[test]
fn deferred_face_to_generates_live_exit_transition_and_keeps_resolved_direction() {
    use crate::coordinates::MapPoint;
    use crate::element::{ActionState, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceState;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::MovingFast;
    let retained_goal = MapPoint::new(321.0, 654.0);

    let sequence = engine.launch_turn_sequence_deferred_no_transitions(
        owner,
        crate::element::Command::TurnFast,
        Some(9),
        0.0,
        0.0,
        Some(retained_goal),
    );
    let deferred = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(deferred.state, SequenceState::Todo);
    assert_eq!(deferred.command, crate::element::Command::TurnFast);
    assert_eq!(
        deferred.priority,
        crate::sequence::SequencePriority::NotYetSet,
        "FaceTo priority belongs to the later Actor::Instruct boundary"
    );
    assert_eq!(deferred.posture_after_transition, Posture::Undefined);
    assert!(
        deferred.orders.is_empty(),
        "deferred FaceTo must remain untranslated until its ordered InstructOwner boundary"
    );

    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let instructed = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(instructed.state, SequenceState::InProgress);
    assert_eq!(instructed.command, crate::element::Command::TurnFast);
    assert_eq!(
        instructed
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![
            OrderType::TransitionRunningUprightWaitingUpright,
            OrderType::Turning,
        ],
        "ordered instruction must sample the live running state and prepend its exit transition"
    );
    assert!(
        !instructed.orders.back().unwrap().compute_direction,
        "Turning must retain the direction resolved from FaceTo's Direction field"
    );
    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(u8::from(entity.position_iface().get_direction_goal()), 9);
    assert_eq!(entity.position_iface().map_goal(), retained_goal);
}

#[test]
fn deferred_face_to_does_not_overwrite_a_newer_live_movement_goal() {
    use crate::coordinates::MapPoint;
    use crate::element::{ActionState, Posture};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::Moving;

    let stale_retained_goal = MapPoint::new(70.0, 80.0);
    let live_goal = MapPoint::new(90.0, 100.0);
    engine.launch_turn_sequence_deferred_no_transitions(
        owner,
        crate::element::Command::Turn,
        Some(9),
        0.0,
        0.0,
        Some(stale_retained_goal),
    );

    // The outgoing actor slot may run after FaceTo registers its deferred
    // Turn and advance the movement goal before SequenceManager instructs it.
    engine
        .get_entity_mut(owner)
        .unwrap()
        .position_iface_mut()
        .set_map_goal(live_goal);
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        live_goal,
        "deferred Turn instruction must not replace a goal advanced by the outgoing actor slot"
    );
}

#[test]
fn positional_face_to_captures_direction_before_deferred_manager_instruction() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::order::AiOrderIntent;
    use crate::sequence::{Field, FieldValue, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    soldier
        .element_data_mut()
        .set_position_map(MapPoint::new(100.0, 100.0));
    let owner = engine.add_entity(soldier);
    let target = MapPoint::new(200.0, 100.0);
    let expected_direction =
        crate::position_interface::vector_to_sector_0_to_15_iso(target.x - 100.0, target.y - 100.0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .orders
        .push(AiOrderIntent::face_toward(target.x, target.y));

    engine.launch_pending_orders_for_npc_mode(&sim, &assets, owner, false);

    let turn_sequence = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find(|sequence| {
            sequence.elements.first().is_some_and(|element| {
                element.owner == Some(owner) && element.command == Command::Turn
            })
        })
        .expect("positional FaceTo registered a deferred Turn");
    let turn = &turn_sequence.elements[0];
    assert_eq!(turn.state, SequenceState::Todo);
    assert!(turn.orders.is_empty());
    assert!(matches!(
        turn.get_property(Field::Direction),
        Some(FieldValue::Integer(direction)) if *direction == expected_direction as u32
    ));
    assert!(turn.get_property(Field::CameraPoint).is_none());
    let turn_sequence_id = turn_sequence.id;

    // If manager-time instruction incorrectly re-resolves the point, this
    // position would reverse the requested direction.
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(300.0, 100.0));
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    assert_eq!(
        u8::from(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .get_direction_goal()
        ),
        expected_direction as u8
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(turn_sequence_id, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
}

#[test]
fn face_to_waits_for_manager_regardless_of_owner_drain_mode() {
    use crate::coordinates::MapPoint;
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{AiOrderIntent, Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};
    use std::num::NonZeroU32;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier);
    let mut movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::WalkingUpright,
        70.0,
        80.0,
        NonZeroU32::new(777).unwrap(),
    ));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        let actor = entity.actor_data_mut().unwrap();
        actor.action_state = ActionState::Moving;
        actor.active_movement = ActiveMovement::new(movement_sequence, 0);
        entity
            .position_iface_mut()
            .set_map_goal(MapPoint::new(70.0, 80.0));
    }
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .halt = true;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .orders
        .push(AiOrderIntent::face_direction(9));

    engine.launch_pending_orders_for_npc_mode(&sim, &assets, owner, false);

    let turn_sequence = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find(|sequence| {
            sequence.elements.first().is_some_and(|element| {
                element.owner == Some(owner) && element.command == Command::Turn
            })
        })
        .expect("deferred standalone FaceTo registered a Turn");
    assert_eq!(turn_sequence.elements[0].state, SequenceState::Todo);
    assert!(
        turn_sequence.elements[0].orders.is_empty(),
        "an ordinary walking actor must not execute or translate an AI-tail Turn in the same owner slot"
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        MapPoint::ZERO,
        "an explicit StopAll before Face must not resurrect the stopped movement goal"
    );
    let turn_sequence_id = turn_sequence.id;

    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let instructed = engine
        .orders
        .sequence_manager
        .get_element(turn_sequence_id, 0)
        .unwrap();
    assert_eq!(instructed.state, SequenceState::InProgress);
    assert_eq!(
        instructed
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![
            OrderType::TransitionWalkingUprightWaitingUpright,
            OrderType::Turning,
        ]
    );
}

#[test]
fn explicit_halt_then_goto_keeps_single_stop_transition() {
    use crate::coordinates::MapPoint;
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{AiOrderIntent, Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};
    use std::num::NonZeroU32;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier);
    let old_goal = MapPoint::new(1004.836, 1774.2802);

    let mut movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::WalkingUpright,
        old_goal.x,
        old_goal.y,
        NonZeroU32::new(779).unwrap(),
    ));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
        entity.actor_data_mut().unwrap().active_movement =
            ActiveMovement::new(movement_sequence, 0);
        entity.position_iface_mut().set_map_goal(old_goal);
        let ai = entity.ai_controller_mut().unwrap();
        ai.outbox.actor.halt = true;
        ai.outbox
            .actor
            .orders
            .push(AiOrderIntent::new(OrderType::RunningUpright, 900.0, 1700.0));
    }

    engine.launch_pending_orders_for_npc_mode(&sim, &assets, owner, false);

    let old = engine
        .orders
        .sequence_manager
        .get_element(movement_sequence, 0)
        .unwrap();
    assert_eq!(old.state, SequenceState::InProgress);
    assert_eq!(
        old.current_order().unwrap().order_type,
        OrderType::TransitionWalkingUprightWaitingUpright,
        "the explicit StopAll Halt rewrites one stop transition and GoTo must not halt it again"
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        old_goal
    );
    assert_eq!(
        engine.orders.pending_move_requests.len(),
        1,
        "GoTo remains queued behind the preserved stop transition"
    );
}

#[test]
fn goto_replacement_retains_selected_movement_goal_while_path_is_pending() {
    use crate::coordinates::MapPoint;
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::sequence::{CascadeFlags, SequenceElement, SequencePriority};
    use std::num::NonZeroU32;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier);

    let old_goal = MapPoint::new(70.0, 80.0);
    let mut movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::RunningUpright);
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::RunningUpright,
        old_goal.x,
        old_goal.y,
        NonZeroU32::new(778).unwrap(),
    ));
    let old_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(old_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().action_state = ActionState::MovingFast;
        entity.actor_data_mut().unwrap().active_movement = ActiveMovement::new(old_sequence, 0);
        entity.position_iface_mut().set_map_goal(old_goal);
    }

    let mut replacement = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(owner),
        OrderType::RunningUpright,
    );
    replacement.priority = SequencePriority::Normal;
    replacement.retained_movement_goal = Some(old_goal);
    let replacement_sequence = engine.orders.sequence_manager.launch_element(replacement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(replacement_sequence, 0);
    engine.orders.sequence_manager.set_halt_pending(true);
    engine
        .orders
        .sequence_manager
        .element_interrupted_after_replacement_selected(old_sequence, 0, CascadeFlags::NEXT_LEVEL);
    engine.orders.sequence_manager.set_halt_pending(false);
    engine.dispatch_condolations(&sim, &assets);

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        old_goal,
        "replacement selection must happen before the old movement's condolence can clear its cached transition goal"
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
        |_, _, _| {},
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
        |_, _, _| {},
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
        |_, _, _| {},
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
        |_, _, _| {},
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
fn ration_set_path_updates_eat_or_guzzle_slot_without_out_of_ammo_speech() {
    use crate::campaign::PcDescription;
    use crate::profiles::{Action, CharacterProfile, CharacterProfileIdx};

    for (action, starting_ammo) in [
        (Action::Eat, 1),
        (Action::Guzzle, 1),
        (Action::Eat, 2),
        (Action::Guzzle, 2),
    ] {
        let mut engine = EngineInner::new();
        let mut pc = make_test_pc(crate::element::Posture::Upright);
        let pc_data = pc.pc_data_mut().unwrap();
        pc_data.profile_index = CharacterProfileIdx(0);
        pc_data.current_action = action;
        pc_data.saved_action = action;
        pc_data.disabled_actions[0] = true;
        let pc_id = engine.add_entity(pc);

        let mut desc = PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(0)),
            ..Default::default()
        };
        desc.status.set_ammo(action, starting_ammo);
        engine.mission_domain.campaign.characters.push(desc);

        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .push(CharacterProfile {
                actions: [
                    action,
                    Action::NoAction,
                    Action::NoAction,
                ],
                ..Default::default()
            });

        engine.consume_ration_without_speech(&assets, pc_id, action);

        assert_eq!(
            engine.mission_domain.campaign.characters[0]
                .status
                .get_ammo(action),
            starting_ammo - 1
        );
        let pc = engine.get_entity(pc_id).unwrap().pc_data().unwrap();
        if starting_ammo == 1 {
            assert_eq!(pc.current_action, Action::NoAction);
            assert_eq!(pc.saved_action, Action::NoAction);
            assert!(pc.disabled_actions[0]);
        } else {
            assert_eq!(pc.current_action, action);
            assert_eq!(pc.saved_action, action);
            assert!(!pc.disabled_actions[0]);
        }
        assert!(engine.feedback.sound_sim.pending_exclamations.is_empty());
    }
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

fn install_owner_selected_test_melee(
    engine: &mut EngineInner,
    attacker: EntityId,
    target: EntityId,
    order_type: crate::order::OrderType,
    past_action_done: bool,
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
    let mut order = crate::order::Order::new(order_type, 0.0, 0.0, order_id);
    order.antagonist = Some(target);
    engine
        .orders
        .sequence_manager
        .push_order_on(sequence, 0, order);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    bind_test_action_point(
        engine,
        attacker,
        order_type,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    let sim = crate::sim_rng::test_context();
    let entity = engine
        .get_entity_mut(attacker)
        .expect("selected melee test attacker exists");
    let mut script = entity.element_data().sprite.scripts[0].clone();
    script.action_done = 1;
    script.frame_ids = vec![1, 2, 3];
    script.delays = vec![0, 0, 0];
    script.distances = vec![0, 0, 0];
    script.offsets = vec![crate::coordinates::SpriteFrameOffset::ZERO; 3];
    script.sound_ids = vec![0, 0, 0];
    entity.element_data_mut().sprite.scripts = std::sync::Arc::new(vec![script; 16]);
    let direction = entity.element_data().direction() as u16;
    let sprite = &mut entity.element_data_mut().sprite;
    assert_eq!(
        sprite.perform_action(
            &sim,
            Some(order_id),
            order_type,
            direction,
            crate::sprite::FrameProgression::Default,
            false,
        ),
        crate::sprite::MotionState::Start
    );
    while sprite.frames_from_now_till_action_done() > 0 {
        assert_eq!(
            sprite.perform_action(
                &sim,
                Some(order_id),
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            ),
            crate::sprite::MotionState::InProgress
        );
    }
    if past_action_done {
        assert_eq!(
            sprite.perform_action(
                &sim,
                Some(order_id),
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            ),
            crate::sprite::MotionState::Done
        );
    }
}

fn chained_straight_strike_target_life(interrupter_first: bool) -> i16 {
    use crate::coordinates::WorldPoint3D;
    use crate::element::Posture;
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
            crate::order::OrderType::StrikingStraightSword,
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
    use crate::movement::SweepState;
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
        crate::order::OrderType::StrikingStraightSword,
        false,
    );

    match interrupt {
        NonstraightInterrupt::Lateral => {
            install_owner_selected_test_melee(
                &mut engine,
                interrupter_id,
                chained_attacker_id,
                crate::order::OrderType::StrikingLeftSword,
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
                crate::order::OrderType::StrikingLeftSword,
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
fn enter_helping_climb_from_tree_retains_exit_prefix_until_animation_done() {
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
            posture: crate::element::Posture::Tree,
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
        crate::element::Posture::Tree,
        "Translate must not apply the DONE-side posture early"
    );
    assert_eq!(
        pc.actor_data().unwrap().action_state,
        crate::element::ActionState::Waiting
    );
    let (sequence_id, element_index) = engine
        .orders
        .sequence_manager
        .current_element_for_actor(pc_id)
        .expect("helping-climb command remains selected while its animation runs");
    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence_id, element_index)
        .expect("selected helping-climb element still exists");
    assert_eq!(element.command, crate::element::Command::EnterHelpingClimb);
    assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(crate::order::OrderType::TransitionWaitingHiddenWaitingUpright)
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
#[test]
fn lethal_swordfight_cleanup_only_unlinks_the_survivor() {
    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let survivor = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let victim = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));

    {
        let survivor_entity = engine.get_entity_mut(survivor).unwrap();
        survivor_entity.actor_data_mut().unwrap().action_state =
            crate::element::ActionState::WaitingSword;
        survivor_entity.human_data_mut().unwrap().opponents = vec![victim];
        survivor_entity.pc_data_mut().unwrap().melee_target = Some(victim);
    }
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.actor_data_mut().unwrap().action_state =
            crate::element::ActionState::WaitingSword;
        victim_entity.human_data_mut().unwrap().opponents = vec![survivor];
        *victim_entity.human_and_life_points_mut().unwrap().1 = 0;
    }

    engine.quit_swordfight(&sim, &assets, victim);

    let survivor_entity = engine.get_entity(survivor).unwrap();
    assert!(survivor_entity.human_data().unwrap().opponents.is_empty());
    assert_eq!(
        survivor_entity.actor_data().unwrap().action_state,
        crate::element::ActionState::WaitingSword,
        "death cleanup must not lower the surviving opponent's sword"
    );
    assert_eq!(survivor_entity.pc_data().unwrap().melee_target, None);
    assert_eq!(
        engine.orders.sequence_manager.sequences_iter().count(),
        0,
        "relationship-only cleanup must not synthesize a QuitSwordfight command"
    );
}

#[test]
fn explicit_quit_dispatch_unlinks_but_defers_state_change_to_lowering_start() {
    use crate::element::Command;
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let opponent = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    {
        let owner_entity = engine.get_entity_mut(owner).unwrap();
        owner_entity.actor_data_mut().unwrap().action_state =
            crate::element::ActionState::WaitingSword;
        owner_entity.human_data_mut().unwrap().opponents = vec![opponent];
    }
    engine
        .get_entity_mut(opponent)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents = vec![owner];

    let sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::QuitSwordfight,
            Some(owner),
        ));
    engine.dispatch_quit_swordfight(&sim, &assets, owner, sequence, 0);

    assert!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .is_empty()
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        crate::element::ActionState::WaitingSword,
        "translation must not switch to Waiting before lowering-sword START"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .and_then(|element| element.current_order())
            .map(|order| order.order_type),
        Some(OrderType::TransitionLoweringSword)
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .map(|element| element.state),
        Some(crate::sequence::SequenceState::InProgress),
        "QuitSwordfight translation must expose the command as current before lowering executes"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        Some((sequence, 0))
    );
}

#[test]
fn lethal_sword_damage_hands_the_corpse_hold_to_wait() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceState};
    use crate::weapons::SwordStrike;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_soldier(Posture::Upright));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        *victim_entity.human_and_life_points_mut().unwrap().1 = 0;
    }

    let mut damage = SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data = SequenceElementData::new_sword_damage(attacker, SwordStrike::E, 0);
    let damage_sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(damage_sequence, 0);

    engine.handle_death_with_damage_element(&sim, &assets, victim, (damage_sequence, 0), None);

    let damage = engine
        .orders
        .sequence_manager
        .get_element(damage_sequence, 0)
        .expect("lethal damage element remains inspectable");
    assert_eq!(
        damage
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![OrderType::DyingSword],
        "ReceiveSwordDamage owns only the one-shot death animation"
    );

    // Model DYING_SWORD's START side effect before its eventual
    // TERMINATED result advances the damage element.
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.set_posture(Posture::Dead);
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
    }
    engine.do_next_order(damage_sequence, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(damage_sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(engine.actor_command(victim), Command::Wait);

    engine.ensure_wait_element(victim);
    let wait_sequence = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find_map(|sequence| {
            sequence
                .elements
                .first()
                .filter(|element| element.owner == Some(victim) && element.command == Command::Wait)
                .map(|_| sequence.id)
        })
        .expect("dead actor receives its ordinary Wait element");
    crate::engine::sequence_runtime::WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        next_order_id: &mut engine.orders.next_order_id,
        profiles: &assets.profile_manager,
    }
    .dispatch(victim, Command::Wait, wait_sequence, 0);

    let wait = engine
        .orders
        .sequence_manager
        .get_element(wait_sequence, 0)
        .unwrap();
    assert_eq!(wait.state, SequenceState::InProgress);
    assert_eq!(
        wait.current_order().map(|order| order.order_type),
        Some(OrderType::BeingDeadSword)
    );
    assert_eq!(engine.actor_command(victim), Command::Wait);
}
