use super::*;

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

fn chained_straight_strike_target_life(interrupter_first: bool) -> i16 {
    use crate::coordinates::WorldPoint3D;
    use crate::element::Posture;
    use crate::movement::{ActiveMelee, MELEE_HIT_FRAME, MELEE_STRIKE_DURATION};
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

    for (attacker, target) in [
        (interrupter_id, chained_attacker_id),
        (chained_attacker_id, final_target_id),
    ] {
        let mut active = ActiveMelee::new(target, SwordStrike::A, None, 0);
        active.frames_remaining = MELEE_STRIKE_DURATION - MELEE_HIT_FRAME;
        engine
            .get_entity_mut(attacker)
            .expect("strike attacker present")
            .actor_data_mut()
            .expect("strike attacker has actor data")
            .active_melee = active;
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
    let mut display = HostDisplayState::default();

    crate::sim_rng::with_seed(0xA_B_C, |sim| {
        engine.hourglass_phase_gameplay_systems(sim, &mut display, &assets);
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
    use crate::movement::{ActiveMelee, MELEE_HIT_FRAME, MELEE_STRIKE_DURATION, SweepState};
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

    let mut chained = ActiveMelee::new(final_target_id, SwordStrike::A, None, 0);
    chained.frames_remaining = MELEE_STRIKE_DURATION - MELEE_HIT_FRAME;
    engine
        .get_entity_mut(chained_attacker_id)
        .expect("chained attacker present")
        .actor_data_mut()
        .expect("chained attacker has actor data")
        .active_melee = chained;

    match interrupt {
        NonstraightInterrupt::Lateral => {
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
            let mut push = ActiveMelee::new(chained_attacker_id, SwordStrike::D, None, 0);
            push.frames_remaining = MELEE_STRIKE_DURATION - MELEE_HIT_FRAME;
            engine
                .get_entity_mut(interrupter_id)
                .expect("push attacker present")
                .actor_data_mut()
                .expect("push attacker has actor data")
                .active_melee = push;
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
    let mut display = HostDisplayState::default();

    crate::sim_rng::with_seed(0xD_E_F, |sim| {
        engine.hourglass_phase_gameplay_systems(sim, &mut display, &assets);
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
