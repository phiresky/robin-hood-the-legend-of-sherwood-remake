#![allow(unused_mut)]

use super::movement::mercenary_formation_destinations;
use super::tick::{
    HourglassPhase, begin_hourglass_phase_capture, capture_ordered_gameplay_entities,
    end_hourglass_phase_capture,
};
use super::*;
use crate::campaign::{Campaign, CampaignValue};
use crate::coordinates::{MapBBox, MapPoint, MapSize, MapVec, SpriteFrameOffset};
use crate::game_operation::GameCode;

/// Distinctive but internally inert state used to lock the top-level Engine
/// serde and bincode contracts while reorganizing its runtime fields.
///
/// Keep this fixture free of level attachments: it must remain serializable at
/// the same snapshot boundary as `EngineInner::new()`.
fn engine_compatibility_fixture() -> EngineInner {
    let mut engine = EngineInner::new();

    let mut config = SimConfig::default();
    config.highlander2 = true;
    config.golden_eye = true;
    config.ignore_default_loose = true;
    engine.attach_sim_config(config);

    engine.mission_domain.state.mission_won = true;
    engine.mission_domain.state.quit_interrupted = true;
    engine.mission_domain.state.map_name = "compatibility-map".into();
    engine.mission_domain.state.victory_defeat_id = 0x1020_3040;
    engine.control.frame_counter = 0x1122_3344;
    engine.set_engine_locked(true);
    engine.set_actors_frozen(true);
    engine.set_fade_freeze_frames_remaining(7);
    engine.control.speed = 1.75;
    engine.control.speed_int = 9;
    engine.world.shield.is_protected = true;
    engine.scripts.globals = vec![-7, 0, 42, i32::MAX];
    engine.mission_domain.cheat_used_flags = 0xA5A5_5A5A;
    engine.ai.standard_view_polygon_radius = 321;
    engine.orders.next_order_id = 0x5566_7788;
    engine.control.chorus_timer = 23;
    engine.script_domains.mission_ui.force_check = true;
    engine.mission_domain.mission_stat.collected_money = 1234;
    engine.mission_domain.mission_stat.added_score = 5678;
    engine.feedback.cutscene_camera.view_position = MapPoint::new(101.5, 202.25);
    engine.restore_rng_from_seed(0xCAFE_BABE_1020_3040);
    engine.feedback.pending_side_effects.invalidate_background = true;
    engine.players.user_locked = true;
    engine.players.qa_recording_slot = 2;
    engine.control.fast_forward = true;
    engine.orders.pending_reinforcements.push(None);
    engine.world.static_sight_obstacle_active = vec![true, false, true];

    engine
}

#[test]
fn engine_top_level_save_schema_and_current_bytes_are_locked() {
    use std::collections::BTreeSet;

    let engine = engine_compatibility_fixture();
    let json = serde_json::to_value(&engine).expect("serialize compatibility fixture to JSON");
    let object = json
        .as_object()
        .expect("EngineInner snapshot must remain a top-level map");
    let actual_keys = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_keys = [
        "action_before_recording_macro",
        "ai_global",
        "campaign",
        "cheat_used_flags",
        "chorus_timer",
        "cutscene_camera",
        "dead_pc",
        "dynamic_sight_obstacles",
        "entities",
        "failed_path_requests",
        "fast_forward",
        "fast_grid",
        "force_check",
        "frame_counter",
        "ground_mark",
        "macro_store",
        "messenger",
        "mission",
        "mission_script",
        "mission_stat",
        "next_order_id",
        "pathfinder",
        "pc_ids",
        "pending_concussion_side_effects",
        "pending_hades_kills",
        "pending_hero_speeches",
        "pending_move_requests",
        "pending_path_requests",
        "pending_reinforcements",
        "pending_scroll_amulets",
        "pending_side_effects",
        "qa_recording_for",
        "qa_recording_slot",
        "rng",
        "script_domains",
        "script_globals",
        "script_zone_data",
        "seats",
        "sequence_manager",
        "shield",
        "short_briefings",
        "sim_config",
        "simulation_gates",
        "sound_sim",
        "speed",
        "speed_int",
        "standard_view_polygon_radius",
        "static_sight_obstacle_active",
        "timer_elements",
        "titbit_manager",
        "user_locked",
        "weather",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(actual_keys, expected_keys);

    let bytes = bincode::serde::encode_to_vec(&engine, bincode::config::standard())
        .expect("encode compatibility fixture to bincode");
    let encoded_hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        encoded_hex,
        "01010101010100010000000111636f6d7061746962696c6974792d6d6170fc40302010fc4433221100000000000101070000e03f090000000100040d0054fcfefffffffc5a5aa5a5fb4101fc88776655170100000000000001000000fbd20400000000000000fb2e160000000000000000000f000000000000010000000000000000000000000000000000cb4200404a43000080bf000080bf0000000000000000000000803f0000803f000000000000000000000000000000000000000000000000000000000000000000c0400000c0400000c0400000c040000000410000004100002041000020410000404100004041000060410000804100008041000090410000a0410000b0410000c0410000d0410000e0410000f0410000004200000042000000420000004200000042000000420000004200000042000000420000004200000042000000000000c0400000c0400000c0400000c040000000410000004100002041000020410000404100004041000060410000804100008041000090410000a0410000b0410000c0410000d0410000e0410000f0410000004200000042000000420000004200000042000000420000004200000042000000420000004200000042010000003f0000803f000000400000000000000000000000000000000000000000000000000000803f0000803f000000000000000000000000000000000500000000fd40302010bebafeca00000000010000000000000000000000000000000000000000010002000100000000000000000000000000000000000000000000000000000200000000000000000000000000000001010001000000000000000003010001000100000000000000000000fcffffffff00000000"
    );
}

#[test]
fn engine_state_hash_is_deterministic_within_the_current_build() {
    let engine = engine_compatibility_fixture();
    let clone = engine.clone();
    assert_eq!(
        crate::replay::state_hash(&engine),
        crate::replay::state_hash(&clone)
    );

    let bytes = bincode::serde::encode_to_vec(&engine, bincode::config::standard())
        .expect("encode compatibility fixture");
    let (restored, consumed): (EngineInner, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
            .expect("decode compatibility fixture");
    assert_eq!(consumed, bytes.len());
    assert_eq!(
        crate::replay::state_hash(&engine),
        crate::replay::state_hash(&restored)
    );
}

#[test]
fn engine_creation() {
    let mut display = HostDisplayState::default();
    let engine = EngineInner::new();
    assert_eq!(engine.feedback.cutscene_camera.zoom_factor, 1.0);
    assert_eq!(engine.control.frame_counter, 0);
    assert!(!engine.control.fast_forward);
    assert!(!engine.engine_locked());
    assert!(!engine.mission_domain.state.mission_won);
    assert_eq!(display.display_op, DisplayOpCode::Redraw);
}

#[test]
fn simulation_gate_aggregate_roundtrips_without_hash_drift() {
    let engine = EngineInner::new();
    let expected_hash = crate::replay::state_hash(&engine);

    let json = serde_json::to_value(&engine).expect("serialize engine");
    let object = json
        .as_object()
        .expect("EngineInner should serialize as a map");
    let gates = object
        .get("simulation_gates")
        .and_then(serde_json::Value::as_object)
        .expect("simulation gates should serialize as a nested aggregate");
    assert_eq!(
        gates.get("lock_engine"),
        Some(&serde_json::Value::Bool(false))
    );
    assert_eq!(
        gates.get("freeze_all"),
        Some(&serde_json::Value::Bool(false))
    );
    assert_eq!(
        gates.get("fade_freeze_frames_remaining"),
        Some(&serde_json::Value::from(0))
    );
    assert!(!object.contains_key("lock_engine"));
    assert!(!object.contains_key("freeze_all"));

    let restored: EngineInner = serde_json::from_value(json).expect("deserialize engine");
    assert_eq!(crate::replay::state_hash(&restored), expected_hash);
}

#[test]
fn simulation_gates_survive_rollback_restore_and_replay() {
    let assets = LevelAssets::new();
    let mut original = EngineInner::new();
    original.set_engine_locked(true);
    original.set_actors_frozen(true);
    original.set_fade_freeze_frames_remaining(2);

    let bytes =
        bincode::serde::encode_to_vec(&original, bincode::config::standard()).expect("encode");
    let (mut replay, consumed): (EngineInner, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).expect("decode");
    assert_eq!(consumed, bytes.len());
    assert!(replay.engine_locked());
    assert!(replay.actors_frozen());
    assert_eq!(replay.fade_freeze_frames_remaining(), 2);
    assert_eq!(
        crate::replay::state_hash(&original),
        crate::replay::state_hash(&replay)
    );

    let mut original_display = HostDisplayState::default();
    let mut replay_display = original_display.clone();
    let mut original_dev = DevState::default();
    let mut replay_dev = DevState::default();
    for _ in 0..4 {
        original.perform_hourglass(&mut original_display, &assets, &mut original_dev);
        replay.perform_hourglass(&mut replay_display, &assets, &mut replay_dev);
        assert_eq!(
            crate::replay::state_hash(&original),
            crate::replay::state_hash(&replay)
        );
    }
}

#[test]
fn rng_snapshot_restores_next_gameplay_draw_and_state_hash() {
    let mut live = EngineInner::new();
    live.restore_rng_from_seed(0xA036_5EED_CAFE_BEEF);
    live.with_sim_rng(|_| {
        let _ = crate::sim_rng::script_rand(crate::sim_rng::RngSite::ScriptRand, 97)
            .expect("positive script bound");
    });

    let bytes = bincode::serde::encode_to_vec(&live, bincode::config::standard())
        .expect("encode RNG snapshot");
    let (mut restored, consumed): (EngineInner, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
            .expect("decode RNG snapshot");
    assert_eq!(consumed, bytes.len());
    assert_eq!(
        crate::replay::state_hash(&live),
        crate::replay::state_hash(&restored)
    );

    let next_live = live.with_sim_rng(|_| {
        (
            crate::sim_rng::script_rand(crate::sim_rng::RngSite::ScriptRand, 101)
                .expect("positive script bound"),
            crate::sim_rng::script_rand(crate::sim_rng::RngSite::ScriptRand, 17)
                .expect("positive script bound"),
        )
    });
    let next_restored = restored.with_sim_rng(|_| {
        (
            crate::sim_rng::script_rand(crate::sim_rng::RngSite::ScriptRand, 101)
                .expect("positive script bound"),
            crate::sim_rng::script_rand(crate::sim_rng::RngSite::ScriptRand, 17)
                .expect("positive script bound"),
        )
    });
    assert_eq!(next_live, next_restored);
    assert_eq!(live.rng_seed(), restored.rng_seed());
    assert_eq!(
        crate::replay::state_hash(&live),
        crate::replay::state_hash(&restored)
    );
}

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
    let mut display = HostDisplayState::default();
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
            Some(order_id),
            OrderType::ShootingWithBow,
            shoot_direction as u16,
            crate::sprite::FrameProgression::Default,
            false,
        );
    assert_eq!(motion, crate::sprite::MotionState::InProgress);

    let (_, visited) = engine.with_sim_rng(|engine| {
        capture_ordered_gameplay_entities(|| {
            engine.hourglass_phase_gameplay_systems(&mut display, &assets)
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
    engine.tick_ability_for(&mut display, &assets, first);

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
    engine.tick_melee_completion_for(&assets, attacker);

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

    crate::sim_rng::with_seed(0xA_B_C, || {
        engine.hourglass_phase_gameplay_systems(&mut display, &assets);
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
    engine.mission_domain.campaign = Some(Campaign::new());

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
            .as_ref()
            .unwrap()
            .get_value(CampaignValue::MissionLength),
        1
    );
}

#[test]
fn fade_to_black_presents_without_advancing_simulation_timers() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.control.frame_counter = 25;

    let mut campaign = Campaign::new();
    campaign.set_value(CampaignValue::MissionLength, 7);
    engine.mission_domain.campaign = Some(campaign);

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
                .as_ref()
                .unwrap()
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
    // against state that silently leaks across clones (e.g. a
    // thread-local that wasn't properly re-seeded on install).
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
    engine.mission_domain.campaign = Some(crate::campaign::Campaign::default());
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

/// Serialize the engine to JSON, deserialize it back, advance the
/// re-hydrated copy, and check it keeps in sync with an equivalent
/// Clone-only copy. This proves the serde audit is complete enough for
/// the fields that matter and that explicit runtime reattachment/default
/// paths do not corrupt gameplay state.
#[test]
fn serde_roundtrip_stays_in_sync() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let seed = 0xFEED_FACE_0123_4567;

    let mut original = EngineInner::new();
    original.restore_rng_from_seed(seed);

    for _ in 0..30 {
        original.perform_hourglass(&mut display, &assets, &mut dev);
    }

    // Serialize + deserialize — this is the capability we just landed.
    let json = serde_json::to_string(&original).expect("engine serialize");
    let mut rehydrated: EngineInner = serde_json::from_str(&json).expect("engine deserialize");

    // A straight Clone is our reference for determinism — the
    // deserialized engine must behave identically.
    let mut clone_ref = original.clone();

    for _ in 0..20 {
        rehydrated.perform_hourglass(&mut display, &assets, &mut dev);
        clone_ref.perform_hourglass(&mut display, &assets, &mut dev);
    }

    assert_eq!(
        rehydrated.control.frame_counter,
        clone_ref.control.frame_counter
    );
    assert_eq!(rehydrated.rng_seed(), clone_ref.rng_seed());
    assert_eq!(
        rehydrated.control.chorus_timer,
        clone_ref.control.chorus_timer
    );
    assert_eq!(
        rehydrated.mission_domain.state.mission_won,
        clone_ref.mission_domain.state.mission_won
    );
    assert_eq!(rehydrated.scripts.globals, clone_ref.scripts.globals);
}

#[test]
fn camera_display_scratch_is_not_serialized_or_hashed() {
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.old_view_position =
        crate::coordinates::MapPoint::new(11.0, 22.0);
    engine.feedback.cutscene_camera.old_zoom_factor = 0.5;
    engine.feedback.cutscene_camera.zoom_init_done = true;
    engine.feedback.cutscene_camera.mechanized_zoom = true;
    engine.feedback.cutscene_camera.displacement = MapVec::new(3.0, 4.0);
    engine.feedback.cutscene_camera.displacement_counter = 7;
    engine.feedback.cutscene_camera.pending_zoom_mouse_screen =
        Some(crate::coordinates::ScreenPoint::new(123.0, 456.0));

    let baseline_hash = crate::replay::state_hash(&engine);
    let json = serde_json::to_string(&engine).expect("serialize engine");
    assert!(!json.contains("old_view_position"));
    assert!(!json.contains("old_zoom_factor"));
    assert!(!json.contains("zoom_init_done"));
    assert!(!json.contains("mechanized_zoom"));
    assert!(!json.contains("displacement_counter"));
    assert!(!json.contains("pending_zoom_mouse_screen"));

    let mut changed = engine.clone();
    changed.feedback.cutscene_camera.old_view_position =
        crate::coordinates::MapPoint::new(99.0, 100.0);
    changed.feedback.cutscene_camera.old_zoom_factor = 2.0;
    changed.feedback.cutscene_camera.zoom_init_done = false;
    changed.feedback.cutscene_camera.mechanized_zoom = false;
    changed.feedback.cutscene_camera.displacement = MapVec::new(-30.0, -40.0);
    changed.feedback.cutscene_camera.displacement_counter = 0;
    changed.feedback.cutscene_camera.pending_zoom_mouse_screen = None;
    assert_eq!(baseline_hash, crate::replay::state_hash(&changed));

    let restored: EngineInner = serde_json::from_str(&json).expect("deserialize engine");
    assert_eq!(
        restored.feedback.cutscene_camera.old_view_position,
        crate::coordinates::MapPoint::new(0.0, 0.0)
    );
    assert_eq!(restored.feedback.cutscene_camera.old_zoom_factor, 1.0);
    assert!(!restored.feedback.cutscene_camera.zoom_init_done);
    assert!(!restored.feedback.cutscene_camera.mechanized_zoom);
    assert_eq!(restored.feedback.cutscene_camera.displacement, MapVec::ZERO);
    assert_eq!(restored.feedback.cutscene_camera.displacement_counter, 0);
    assert_eq!(
        restored.feedback.cutscene_camera.pending_zoom_mouse_screen,
        None
    );
}

#[test]
fn host_display_scroll_does_not_mutate_script_camera() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(100.0, 200.0);

    display.display_op = DisplayOpCode::Scroll;
    display.background_transform.scrolling_vector = MapVec::new(25.0, 0.0);

    engine.perform_hourglass(&mut display, &assets, &mut dev);

    assert_eq!(
        engine.feedback.cutscene_camera.view_position,
        crate::coordinates::MapPoint::new(100.0, 200.0)
    );
}

#[test]
fn camera_display_scroll_mutates_script_camera() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(100.0, 200.0);

    engine.feedback.cutscene_camera.display.display_op = DisplayOpCode::Scroll;
    engine
        .feedback
        .cutscene_camera
        .display
        .background_transform
        .scrolling_vector = MapVec::new(25.0, 0.0);

    engine.perform_hourglass(&mut display, &assets, &mut dev);

    assert_eq!(
        engine.feedback.cutscene_camera.view_position,
        crate::coordinates::MapPoint::new(125.0, 200.0)
    );
}

/// Regression test for the PI-into-Sprite refactor (save-format v2).
///
/// `ElementData.sprite` is now fully serialized, so the embedded
/// `PositionInterface` + animation counters (`current_row`,
/// `current_frame`, `frame_count`, `last_action`) survive a save-load
/// round trip.  The Arc-shared script caches (`scripts`,
/// `alternate_scripts`, `conversion`, `alternate_conversion`) are
/// level-owned attachments and must come back as defaults — they
/// re-hydrate from the sprite cache on load using the serialized profile
/// keys.
///
/// If any of the expected-to-survive fields starts zeroing out, or any
/// of the expected-to-reset fields starts round-tripping, the sprite
/// serialization surface has shifted and the save version needs another
/// bump.
#[test]
fn sprite_serialization_surface_matches_v2_contract() {
    let mut display = HostDisplayState::default();
    use crate::coordinates::WorldPoint3D;
    use crate::element::{ActorSoldier, ElementData, ElementKind, Entity};
    use crate::order::OrderType;
    use std::sync::Arc;
    let mut engine = EngineInner::new();
    let mut element = ElementData {
        kind: ElementKind::ActorSoldier,
        ..Default::default()
    };
    {
        let s = &mut element.sprite;
        s.position_iface.set_position(WorldPoint3D {
            x: 123.5,
            y: 456.25,
            z: 7.0,
        });
        s.position_iface
            .set_direction_instantly(crate::position_interface::Direction::from_raw(11));
        s.current_row = 5;
        s.current_frame = 3;
        s.frame_count = 7;
        s.current_width = 64;
        s.current_height = 80;
        s.last_action = OrderType::WalkingUpright;
        s.last_processed_order_id = 42;
        s.action_done_frame = 9;
        s.action_done_counter = 4;
        s.use_alternate_profile = true;
        s.anims_to_be_replaced = vec![OrderType::WalkingUpright];
        s.replacing_anims = vec![OrderType::RunningUpright];

        // Runtime attachment fields — seed with non-defaults to prove
        // only Arc-shared level-owned attachments get wiped on deserialize.
        s.frame_profile_name = "FakeProfile".into();
        s.profile_cache_key = "FakeFile/FakeProfile".into();
        s.alternate_profile_cache_key = "FakeFile/FakeAlternate".into();
        s.center = crate::coordinates::SpriteAnchor { x: 32.0, y: 48.0 };
        s.scripts = Arc::new(Vec::new());
        s.alternate_scripts = Some(Arc::new(Vec::new()));
        s.conversion = Arc::new(vec![0, 1, 2]);
        s.alternate_conversion = Some(Arc::new(vec![3, 4, 5]));
    }
    engine.add_entity(Entity::Soldier(ActorSoldier {
        element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    let json = serde_json::to_string(&engine).expect("serialize");
    let rehydrated: EngineInner = serde_json::from_str(&json).expect("deserialize");

    // Pull the sprite back out of the rehydrated engine.
    let rehydrated_sprite = rehydrated
        .world
        .entities
        .occupied()
        .map(|(_, entity)| entity)
        .next()
        .expect("one entity")
        .element_data()
        .sprite
        .clone();

    // ── Fields that MUST survive (PI + animation state) ────────────
    let pos = rehydrated_sprite.position_iface.get_position();
    assert_eq!(pos.x, 123.5);
    assert_eq!(pos.y, 456.25);
    assert_eq!(pos.z, 7.0);
    assert_eq!(
        rehydrated_sprite.position_iface.get_direction(),
        crate::position_interface::Direction::from_raw(11)
    );
    assert_eq!(rehydrated_sprite.current_row, 5);
    assert_eq!(rehydrated_sprite.current_frame, 3);
    assert_eq!(rehydrated_sprite.frame_count, 7);
    assert_eq!(rehydrated_sprite.current_width, 64);
    assert_eq!(rehydrated_sprite.current_height, 80);
    assert_eq!(rehydrated_sprite.last_action, OrderType::WalkingUpright);
    assert_eq!(rehydrated_sprite.last_processed_order_id, 42);
    assert_eq!(rehydrated_sprite.action_done_frame, 9);
    assert_eq!(rehydrated_sprite.action_done_counter, 4);
    assert!(rehydrated_sprite.use_alternate_profile);
    assert_eq!(
        rehydrated_sprite.anims_to_be_replaced,
        vec![OrderType::WalkingUpright]
    );
    assert_eq!(
        rehydrated_sprite.replacing_anims,
        vec![OrderType::RunningUpright]
    );

    // ── Fields that MUST reset on deserialize (re-bound via sprite cache) ──
    // Primary scripts/conversion are non-`Option` Arcs now: round-trip
    // gives back the empty-placeholder Arc from `Sprite::default()`
    // rather than `None`.
    assert!(rehydrated_sprite.scripts.is_empty());
    assert!(rehydrated_sprite.alternate_scripts.is_none());
    assert!(rehydrated_sprite.conversion.is_empty());
    assert!(rehydrated_sprite.alternate_conversion.is_none());
    assert_eq!(rehydrated_sprite.frame_profile_name, "FakeProfile");
    assert_eq!(rehydrated_sprite.profile_cache_key, "FakeFile/FakeProfile");
    assert_eq!(
        rehydrated_sprite.alternate_profile_cache_key,
        "FakeFile/FakeAlternate"
    );
    assert_eq!(rehydrated_sprite.center.x, 32.0);
    assert_eq!(rehydrated_sprite.center.y, 48.0);

    // Model the level loader rebinding an alternate profile before ticking.
    // Empty attachments are sufficient here because this contract test only
    // exercises deterministic state progression, not animation resources.
    let mut rehydrated = rehydrated;
    let sprite = &mut rehydrated
        .world
        .entities
        .occupied_mut()
        .next()
        .expect("one entity")
        .1
        .element_data_mut()
        .sprite;
    sprite.alternate_scripts = Some(Arc::new(Vec::new()));
    sprite.alternate_conversion = Some(Arc::new(Vec::new()));

    // Ticking twice must not diverge from an equivalent in-memory
    // clone; what matters for sim determinism is that the tick path treats
    // both copies identically after the normal runtime attachments are bound.
    let mut dev = DevState::default();
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut rehydrated, &mut assets);
    let mut clone = rehydrated.clone();
    for _ in 0..2 {
        rehydrated.perform_hourglass(&mut display, &assets, &mut dev);
        clone.perform_hourglass(&mut display, &assets, &mut dev);
    }
    assert_eq!(
        rehydrated.control.frame_counter,
        clone.control.frame_counter
    );
    assert_eq!(rehydrated.rng_seed(), clone.rng_seed());
}

#[test]
fn measure_engine_size() {
    use std::mem;

    let struct_size = mem::size_of::<EngineInner>();
    eprintln!("EngineInner struct (stack): {} bytes", struct_size);
    eprintln!(
        "Entity enum size: {} bytes",
        mem::size_of::<crate::element::Entity>()
    );
    eprintln!(
        "Option<Entity> size: {} bytes",
        mem::size_of::<Option<crate::element::Entity>>()
    );

    // Create an engine with entities similar to a real level
    let mut engine = EngineInner::new();
    for i in 0..100u32 {
        let mut element = crate::element::ElementData {
            kind: crate::element::ElementKind::ActorSoldier,
            ..Default::default()
        };
        element.set_position_map(MapPoint::new(i as f32 * 10.0, i as f32 * 10.0));
        let entity = crate::element::Entity::Soldier(crate::element::ActorSoldier {
            element,
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        });
        engine.add_entity(entity);
    }
    for i in 0..4u32 {
        let mut element = crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            ..Default::default()
        };
        element.set_position_map(MapPoint::new(100.0 + i as f32 * 20.0, 100.0));
        let entity = crate::element::Entity::Pc(crate::element::ActorPc {
            element,
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        });
        engine.add_entity(entity);
    }

    // JSON size = meaningful serialized state
    let json = serde_json::to_string(&engine).expect("serialize");
    eprintln!(
        "JSON compact: {} bytes ({:.1} KB)",
        json.len(),
        json.len() as f64 / 1024.0
    );

    // Clone timing
    let n_clones = 1000;
    let start = web_time::Instant::now();
    for _ in 0..n_clones {
        let clone = engine.clone();
        std::hint::black_box(&clone);
    }
    let clone_elapsed = start.elapsed();
    let clone_us = clone_elapsed.as_micros() as f64 / n_clones as f64;
    eprintln!(
        "Clone: {:.1} µs per clone ({} clones in {:.1} ms)",
        clone_us,
        n_clones,
        clone_elapsed.as_millis()
    );

    // Serialize timing
    let start = web_time::Instant::now();
    let n_ser = 100;
    for _ in 0..n_ser {
        let j = serde_json::to_string(&engine).unwrap();
        std::hint::black_box(&j);
    }
    let ser_elapsed = start.elapsed();
    eprintln!(
        "Serialize: {:.1} µs per serialize",
        ser_elapsed.as_micros() as f64 / n_ser as f64
    );

    eprintln!("\n=== Summary ===");
    eprintln!("Stack shell: {} bytes", struct_size);
    eprintln!(
        "Serialized state (104 entities): {:.1} KB",
        json.len() as f64 / 1024.0
    );
    eprintln!(
        "Clone: {:.1} µs | Serialize: {:.1} µs",
        clone_us,
        ser_elapsed.as_micros() as f64 / n_ser as f64
    );
    eprintln!(
        "At 25fps: clone budget = 40ms/frame → {:.0} clones/frame",
        40_000.0 / clone_us
    );

    assert!(struct_size > 0);
}

#[test]
fn script_globals() {
    let mut engine = EngineInner::new();
    engine.init_script_global(5, 42);
    assert_eq!(engine.get_script_global(5), 42);
    // `init_script_global` resizes to `id + 16`, giving scripts a
    // 16-slot slack window of valid reads beyond the last-initialised
    // index.
    assert_eq!(engine.scripts.globals.len(), 5 + 16);
    for i in 6..(5 + 16) {
        assert_eq!(engine.get_script_global(i), 0);
    }

    engine.set_script_global(5, 99);
    assert_eq!(engine.get_script_global(5), 99);

    assert!(engine.is_valid_script_global_id(5));
    assert!(engine.is_valid_script_global_id(20));
    assert!(!engine.is_valid_script_global_id(21));
}

#[test]
#[should_panic(expected = "out of range")]
fn script_global_set_out_of_range_panics() {
    let mut engine = EngineInner::new();
    engine.set_script_global(100, 1);
}

#[test]
fn global_options_default() {
    let opts = GlobalOptions::default();
    assert_eq!(opts.major_version, 1);
    assert_eq!(opts.minor_version, 2);
    assert!(opts.sound_enabled);
    assert!(opts.script_enabled);
    assert!(!opts.highlander2);
    assert_eq!(opts.level_directory, "Data/Levels");
}

#[test]
fn draw_fast_forward_skips() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.control.fast_forward = true;
    engine.control.frame_counter = 1; // Not a multiple of 32
    let result = engine.tick_display_state(&mut display);
    assert_eq!(result, 1); // Should skip
}

#[test]
fn draw_fast_forward_every_32nd() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.control.fast_forward = true;
    engine.control.frame_counter = 32; // Multiple of 32
    let result = engine.tick_display_state(&mut display);
    assert_eq!(result, 0); // Should render
}

#[test]
fn ambiance_night_colors() {
    assert_eq!(Ambiance::Day.night_color_rgb(), (45, 45, 35));
    assert_eq!(Ambiance::Fog.night_color_rgb(), (85, 77, 90));
    assert_eq!(Ambiance::Night.night_color_rgb(), (0, 0, 0));
}

#[test]
fn center_on_point() {
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4000.0, 3000.0);
    engine.center_on_point(0, crate::coordinates::MapPoint::new(1000.0, 800.0));
    // View should be offset by half the full screen on both axes
    // (raw screen vector divided by 2*zoom; the bottom-panel exclusion
    // applies only to the clamp, not the centering).  The result is
    // floored before assignment.
    let expected_x = (1000.0f32 - 512.0f32).floor(); // 1024/2
    let expected_y = (800.0f32 - 384.0f32).floor(); // 768/2
    assert!((engine.feedback.cutscene_camera.view_position.x - expected_x).abs() < 0.01);
    assert!((engine.feedback.cutscene_camera.view_position.y - expected_y).abs() < 0.01);
}

#[test]
fn mission_state_transitions() {
    let mut engine = EngineInner::new();
    assert!(!engine.mission_domain.state.mission_won);

    engine.win(true);
    assert!(engine.mission_domain.state.mission_won);
    assert!(engine.mission_domain.state.mission_won_first_time);

    // `win` writes both flags unconditionally, so a second call
    // re-toggles `mission_won_first_time`.
    engine.mission_domain.state.mission_won_first_time = false;
    engine.win(true);
    assert!(engine.mission_domain.state.mission_won_first_time);

    // A silent win (show_window=false) queues the start/quit-mission
    // widget swap as a side-effect for the host to drain.
    engine.feedback.pending_side_effects = Default::default();
    engine.win(false);
    assert!(!engine.mission_domain.state.mission_won_first_time);
    assert!(
        engine
            .feedback
            .pending_side_effects
            .pending_silent_win_widget_swap
    );
}

#[test]
fn initialize_sends_stature_message() {
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    assert_eq!(engine.orders.messenger.count(), 0);
    engine.initialize(&mut assets);
    // Should have sent a Stature message
    let msg = engine
        .orders
        .messenger
        .poll()
        .expect("expected stature message");
    assert_eq!(msg.msg_type, MessageType::Simple(SimpleMessage::Stature));
}

#[test]
fn mission_won_first_time_raises_mission_state_notice() {
    let mut display = HostDisplayState::default();
    // On the first post-win frame with no PC guarded, the engine
    // fires the `LEAVE_MISSION_NOW` mission-state notice +
    // `EnableWidgetQuitMission(false)`.  Both are routed through
    // `SideEffects.pending_mission_state_notice`.
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.state.mission_won_first_time = true;
    let side_effects = engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert!(!engine.mission_domain.state.mission_won_first_time);
    assert!(
        side_effects.pending_mission_state_notice,
        "expected pending_mission_state_notice side effect"
    );
}

#[test]
fn post_load_fixups_aborts_midzoom() {
    let mut display = HostDisplayState::default();
    // Build an engine mid-zoom and run the post-load fixup path
    // directly.  The zoom-abort block previously lived in
    // `tick_display_state` under `!cache_valid`; it now runs inside
    // `EngineInner::post_load_fixups` so `Engine::restore` can't
    // leave the engine mid-zoom.
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    engine
        .feedback
        .cutscene_camera
        .display
        .background_transform
        .zoom_to_up = true;
    engine.feedback.cutscene_camera.zoom_init_done = true;

    engine.post_load_fixups(&mut display);

    assert!(
        !engine
            .feedback
            .cutscene_camera
            .display
            .background_transform
            .zoom_to_up
    );
    assert!(!engine.feedback.cutscene_camera.zoom_init_done);
    let msg = engine
        .orders
        .messenger
        .poll()
        .expect("expected zoom end message");
    assert_eq!(msg.msg_type, MessageType::Simple(SimpleMessage::ZoomUpEnd));
}

#[test]
fn mercenary_formation_single_pc_lands_on_click() {
    let click = crate::coordinates::map_pt(200.0, 300.0);
    let dests = mercenary_formation_destinations(&[crate::coordinates::map_pt(50.0, 50.0)], click);
    assert_eq!(dests.len(), 1);
    assert_eq!(dests[0].x, click.x);
    assert_eq!(dests[0].y, click.y);
}

#[test]
fn mercenary_formation_preserves_relative_offsets() {
    // 3 PCs in a horizontal line at (0,0), (50,0), (100,0).
    // Centroid = (50, 0).  Click at (200, 300).
    // Per-PC dests should preserve the (-50, 0), (0, 0), (+50, 0) offsets
    // relative to the click point.
    let pcs = [
        crate::coordinates::map_pt(0.0, 0.0),
        crate::coordinates::map_pt(50.0, 0.0),
        crate::coordinates::map_pt(100.0, 0.0),
    ];
    let click = crate::coordinates::map_pt(200.0, 300.0);
    let dests = mercenary_formation_destinations(&pcs, click);
    assert_eq!(dests.len(), 3);
    assert_eq!(dests[0], crate::coordinates::map_pt(150.0, 300.0));
    assert_eq!(dests[1], crate::coordinates::map_pt(200.0, 300.0));
    assert_eq!(dests[2], crate::coordinates::map_pt(250.0, 300.0));
}

#[test]
fn mercenary_formation_empty_input() {
    let dests = mercenary_formation_destinations(&[], crate::coordinates::map_pt(0.0, 0.0));
    assert!(dests.is_empty());
}

#[test]
fn ground_mark_hourglass_advances_and_retires_on_screen_marks() {
    let mut display = HostDisplayState::default();
    // The per-mark animation advance is gated on `IsOnScreen` and
    // even universal-frame-counter ticks.  For rollback determinism
    // the state advance happens inside `perform_hourglass` instead —
    // render is read-only.
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    // 8×8 sprites centred on camera origin so every frame is on-screen.
    engine.set_ground_mark_sprite_data(
        0.0,
        0.0,
        vec![(8, 8); crate::markers::NUMBER_OF_GROUND_FRAMES as usize],
        vec![(0, 0); crate::markers::NUMBER_OF_GROUND_FRAMES as usize],
    );
    engine.feedback.ground_mark.add_mark(100.0, 100.0, 0);
    assert_eq!(engine.feedback.ground_mark.len(), 1);

    // Plenty of ticks to burn through all NUMBER_OF_GROUND_FRAMES advances
    // (half of them gated off by odd frame counters) and retire the mark.
    for _ in 0..(2 * crate::markers::NUMBER_OF_GROUND_FRAMES as usize + 4) {
        engine.perform_hourglass(&mut display, &assets, &mut dev);
    }
    assert!(
        engine.feedback.ground_mark.is_empty(),
        "mark should have animated through to retirement"
    );
}

#[test]
fn ground_mark_hourglass_freezes_off_screen_marks() {
    let mut display = HostDisplayState::default();
    // Off-screen marks must freeze in both live and replay — the
    // `IsOnScreen` gate suppresses advance.
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.set_ground_mark_sprite_data(
        0.0,
        0.0,
        vec![(8, 8); crate::markers::NUMBER_OF_GROUND_FRAMES as usize],
        vec![(0, 0); crate::markers::NUMBER_OF_GROUND_FRAMES as usize],
    );
    // Mark at (100_000, 100_000) is well outside the 800×600 viewport.
    engine
        .feedback
        .ground_mark
        .add_mark(100_000.0, 100_000.0, 0);

    for _ in 0..(2 * crate::markers::NUMBER_OF_GROUND_FRAMES as usize + 4) {
        engine.perform_hourglass(&mut display, &assets, &mut dev);
    }
    assert_eq!(engine.feedback.ground_mark.len(), 1);
    assert_eq!(engine.feedback.ground_mark.marks[0].current_frame, 0);
}

#[test]
fn mission_stat_resets_on_new_mission() {
    let mut assets = LevelAssets::new();
    let mut pending = PendingLevelData::default();
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Some(crate::campaign::Campaign::default());
    engine.mission_domain.mission_stat.add_collected_money(500);
    engine.mission_domain.short_briefings.add(42, true);

    let loaded = crate::level_data::LoadedLevel::empty_for_test();
    let _ = engine.initialize_from_mission(
        &mut assets,
        &mut pending,
        "test_mission",
        "test_proto",
        loaded,
        "Data/Levels",
        (0.0, 0.0),
        &mut |_| {},
    );

    assert_eq!(engine.mission_domain.mission_stat.collected_money, 0);
    assert_eq!(engine.mission_domain.short_briefings.count(true), 0);
}

#[test]
fn resize_snaps_zoom() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(500.0, 400.0); // Small level
    engine.feedback.cutscene_camera.zoom_factor = 0.5;
    display.background_transform.current_zoom_level = 0;

    engine.resize(&mut display, 1024.0, 768.0);

    // Should have snapped to 1.0 since 0.5x can't fit
    assert_eq!(engine.feedback.cutscene_camera.zoom_factor, 1.0);
    assert_eq!(display.background_transform.current_zoom_level, 1);
}

// ── Campaign integration tests ──────────────────────────────

#[test]
fn add_campaign_value_ransom_credits_mission_stat_and_emits_jingle() {
    use crate::sound::Jingle;
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Some(Campaign::default());
    engine.control.frame_counter = 100; // past frame 0 → jingle gate open

    engine.add_campaign_value(CampaignValue::Ransom, 250);

    assert_eq!(
        engine
            .mission_domain
            .campaign
            .as_ref()
            .unwrap()
            .get_value(CampaignValue::Ransom),
        crate::campaign::INITIAL_RANSOM + 250
    );
    assert_eq!(engine.mission_domain.mission_stat.collected_money, 250);
    let jingle_count = engine
        .feedback
        .pending_side_effects
        .sounds
        .iter()
        .filter(|s| matches!(s, SoundCommand::Jingle(Jingle::CashWon)))
        .count();
    assert_eq!(jingle_count, 1);
}

#[test]
fn add_campaign_value_score_credits_mission_stat() {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Some(Campaign::default());
    engine.control.frame_counter = 100;

    engine.add_campaign_value(CampaignValue::Score, 750);

    assert_eq!(
        engine
            .mission_domain
            .campaign
            .as_ref()
            .unwrap()
            .get_value(CampaignValue::Score),
        750
    );
    assert_eq!(engine.mission_domain.mission_stat.added_score, 750);
    // Score is silent.
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());
}

#[test]
fn add_campaign_value_negative_ransom_skips_jingle_but_credits_money() {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Some(Campaign::default());
    engine.control.frame_counter = 100;
    engine.mission_domain.campaign.as_mut().unwrap().values[CampaignValue::Ransom] = 500;
    engine.mission_domain.mission_stat.collected_money = 200;

    // A purse throw (`combat.rs:2433`) issues a negative delta.
    engine.add_campaign_value(CampaignValue::Ransom, -100);

    assert_eq!(
        engine
            .mission_domain
            .campaign
            .as_ref()
            .unwrap()
            .get_value(CampaignValue::Ransom),
        400
    );
    // `add_campaign_value` credits the mission-stat counter
    // unconditionally (wrapping_add_signed); only the jingle is gated.
    assert_eq!(engine.mission_domain.mission_stat.collected_money, 100);
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());
}

#[test]
fn add_campaign_value_skips_jingle_at_frame_zero() {
    // The `frame_counter > 0` gate ensures the pre-mission seed
    // (initial ransom = 100) doesn't sound a coin chime.
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Some(Campaign::default());
    engine.control.frame_counter = 0;

    engine.add_campaign_value(CampaignValue::Ransom, 100);

    assert_eq!(engine.mission_domain.mission_stat.collected_money, 100);
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());
}

#[test]
fn set_campaign_value_ransom_emits_jingle_only_when_growing() {
    use crate::sound::Jingle;
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Some(Campaign::default());
    engine.control.frame_counter = 50;
    engine.mission_domain.campaign.as_mut().unwrap().values[CampaignValue::Ransom] = 200;

    // Lower → no jingle (only growth fires the gate).
    engine.set_campaign_value(CampaignValue::Ransom, 100);
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());

    // Higher → jingle.
    engine.set_campaign_value(CampaignValue::Ransom, 500);
    let jingle_count = engine
        .feedback
        .pending_side_effects
        .sounds
        .iter()
        .filter(|s| matches!(s, SoundCommand::Jingle(Jingle::CashWon)))
        .count();
    assert_eq!(jingle_count, 1);
    // SetValue does NOT credit collected_money — only AddValue does.
    assert_eq!(engine.mission_domain.mission_stat.collected_money, 0);
}

#[test]
fn add_campaign_value_amulets_has_no_side_effects() {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Some(Campaign::default());
    engine.control.frame_counter = 100;

    engine.add_campaign_value(CampaignValue::Amulets, 3);

    assert_eq!(
        engine
            .mission_domain
            .campaign
            .as_ref()
            .unwrap()
            .get_value(CampaignValue::Amulets),
        3
    );
    assert_eq!(engine.mission_domain.mission_stat.collected_money, 0);
    assert_eq!(engine.mission_domain.mission_stat.added_score, 0);
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());
}

#[test]
fn sync_stats_to_campaign() {
    let mut engine = EngineInner::new();
    engine.mission_domain.mission_stat.collected_money = 500;
    engine.mission_domain.mission_stat.added_score = 1200;
    engine.mission_domain.mission_stat.living_soldier_count = 8;
    engine.mission_domain.mission_stat.total_soldier_count = 12;

    let mut campaign = Campaign::default();
    campaign.set_value(CampaignValue::Ransom, 100);

    engine.sync_stats_to_campaign(&mut campaign);

    // Money/score are credited during gameplay via add_campaign_value,
    // so sync at mission end must NOT re-add them — only soldier counts.
    assert_eq!(campaign.get_value(CampaignValue::Ransom), 100);
    assert_eq!(campaign.get_value(CampaignValue::Score), 0);
    assert_eq!(campaign.get_value(CampaignValue::LivingSoldiers), 8);
    assert_eq!(campaign.get_value(CampaignValue::DeadSoldiers), 4); // 12 - 8
}

#[test]
fn current_mission_profile_none_when_no_mission() {
    let engine = EngineInner::new();
    let campaign = Campaign::default();
    let profiles = crate::profiles::ProfileManager::new();
    assert!(
        engine
            .current_mission_profile(&campaign, &profiles)
            .is_none()
    );
}

#[test]
fn is_sherwood_mission_no_mission() {
    let engine = EngineInner::new();
    let campaign = Campaign::default();
    let profiles = crate::profiles::ProfileManager::new();
    assert!(!engine.is_sherwood_mission(&campaign, &profiles));
}

// ── New tests for ported engine internals ──────────────────

#[test]
fn perform_check_scroll_clamps_right() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(2000.0, 1500.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(1500.0, 0.0);
    display.background_transform.scrolling_vector = MapVec::new(400.0, 0.0);

    let valid = engine.perform_check_scroll(&mut display);
    assert!(!valid);
    // Scroll should be clamped: 2000 - 1500 - 800/1.0 = -300
    // (negative means "can't go further right")
    assert!(display.background_transform.scrolling_vector.x <= 2000.0 - 1500.0 - 800.0 + 0.01);
}

#[test]
fn perform_check_scroll_clamps_left() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(2000.0, 1500.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(10.0, 0.0);
    display.background_transform.scrolling_vector = MapVec::new(-50.0, 0.0);

    let valid = engine.perform_check_scroll(&mut display);
    assert!(!valid);
    assert!((display.background_transform.scrolling_vector.x - (-10.0)).abs() < 0.01);
}

#[test]
fn perform_check_scroll_valid() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4000.0, 3000.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(500.0, 500.0);
    display.background_transform.scrolling_vector = MapVec::new(10.0, 10.0);

    let valid = engine.perform_check_scroll(&mut display);
    assert!(valid);
    assert!((display.background_transform.scrolling_vector.x - 10.0).abs() < 0.01);
}

#[test]
fn timer_tick_decrements_and_removes() {
    let mut display = HostDisplayState::default();
    use crate::sequence::{SequenceElementRef, SequenceId};
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let ref_a = SequenceElementRef::new(SequenceId(100), 0);
    let ref_b = SequenceElementRef::new(SequenceId(200), 0);
    engine.add_timer(3, ref_a);
    engine.add_timer(1, ref_b);
    assert_eq!(engine.orders.timer_elements.len(), 2);

    engine.perform_hourglass(&mut display, &assets, &mut dev);
    // Timer 200 (remaining=1) should be removed, timer 100 decremented to 2
    assert_eq!(engine.orders.timer_elements.len(), 1);
    assert_eq!(engine.orders.timer_elements[0].remaining, 2);
    assert_eq!(engine.orders.timer_elements[0].element_ref, ref_a);

    engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert_eq!(engine.orders.timer_elements[0].remaining, 1);

    engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert!(engine.orders.timer_elements.is_empty());
}

#[test]
fn win_respects_show_window_false() {
    let mut engine = EngineInner::new();
    engine.win(false);
    assert!(engine.mission_domain.state.mission_won);
    assert!(!engine.mission_domain.state.mission_won_first_time);
}

#[test]
fn win_respects_show_window_true() {
    let mut engine = EngineInner::new();
    engine.win(true);
    assert!(engine.mission_domain.state.mission_won);
    assert!(engine.mission_domain.state.mission_won_first_time);
}

#[test]
fn zoom_change_state_updates_level() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    assert_eq!(display.background_transform.current_zoom_level, 1);

    // Zoom up: level should increment to 2
    engine.change_state(&mut display, 0, EngineStateRequest::ZoomingUp);
    assert_eq!(display.background_transform.current_zoom_level, 2);
    assert!(display.background_transform.zoom_to_up);

    // Reset for next test
    display.background_transform.zoom_to_up = false;
    engine.feedback.cutscene_camera.zoom_init_done = false;
    display.display_op = DisplayOpCode::Nothing;

    // Zoom down: level should decrement to 1
    engine.change_state(&mut display, 0, EngineStateRequest::ZoomingDown);
    assert_eq!(display.background_transform.current_zoom_level, 1);
    assert!(display.background_transform.zoom_to_down);
}

#[test]
fn zoom_deferred_when_scrolling() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    // Simulate active scrolling
    display.background_transform.current_x_scrolling_level = 5;

    engine.change_state(&mut display, 0, EngineStateRequest::ZoomingUp);
    // Should be deferred, not immediate
    assert!(display.background_transform.required_zoom_up);
    assert!(!display.background_transform.zoom_to_up);
    assert_eq!(display.background_transform.current_zoom_level, 1); // unchanged
}

#[test]
fn sort_for_minimap_priority_order() {
    use crate::element::{ActorPc, ActorSoldier, ElementBonus, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();

    // Add entities of each priority tier.  Minimap priority ranking:
    // soldier (low) < pc < object (high).
    let mut soldier_elem = ElementData {
        kind: ElementKind::ActorSoldier,
        ..Default::default()
    };
    soldier_elem.set_position_map(MapPoint::new(20.0, 20.0));
    let soldier_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: soldier_elem,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    let mut pc_elem = ElementData {
        kind: ElementKind::ActorPc,
        ..Default::default()
    };
    pc_elem.set_position_map(MapPoint::new(30.0, 30.0));
    let pc_id = engine.add_entity(Entity::Pc(ActorPc {
        element: pc_elem,
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    }));

    let mut bonus_elem = ElementData {
        kind: ElementKind::ObjectBonus,
        ..Default::default()
    };
    bonus_elem.set_position_map(MapPoint::new(40.0, 40.0));
    let object_id = engine.add_entity(Entity::Bonus(ElementBonus {
        element: bonus_elem,
        object: Default::default(),
    }));

    let sorted = engine.sort_for_minimap();
    assert_eq!(sorted, vec![soldier_id, pc_id, object_id]);
}

#[test]
fn swordfight_los_ignores_crossing_motion_line() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{ActorSoldier, ElementData, ElementKind, Entity, Posture};
    use crate::element_kinds::ActionState;
    use crate::fast_find_grid::GridLine;

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    engine.world.fast_grid.size_map(4, 4);
    engine.world.fast_grid.allocate_layers(1);
    engine.world.fast_grid.add_line(
        GridLine::new(
            MapPoint::new(115.0, 50.0),
            MapPoint::new(115.0, 150.0),
            true,
        ),
        0,
    );

    let make_fighter = |x| {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            posture: Posture::Upright,
            ..Default::default()
        };
        element.set_position(WorldPoint3D {
            x,
            y: 100.0,
            z: 0.0,
        });
        Entity::Soldier(ActorSoldier {
            element,
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        })
    };
    let left_id = engine.add_entity(make_fighter(100.0));
    let right_id = engine.add_entity(make_fighter(130.0));

    for (fighter_id, opponent_id) in [(left_id, right_id), (right_id, left_id)] {
        let fighter = engine.world.entities.get_mut(fighter_id).unwrap();
        fighter.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        fighter
            .human_data_mut()
            .unwrap()
            .opponents
            .push(opponent_id);
    }

    assert!(
        engine
            .world
            .fast_grid
            .impact_intersection_ratio(MapPoint::new(100.0, 100.0), MapPoint::new(130.0, 100.0), 0,)
            .is_some(),
        "fixture must contain a movement barrier between the fighters"
    );

    engine.tick_evaluate_swordfight(&assets);

    assert_eq!(
        engine
            .get_entity(left_id)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![right_id]
    );
    assert_eq!(
        engine
            .get_entity(right_id)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![left_id]
    );
}

#[test]
fn smalltalk_strike_does_not_transfer_initiative_immediately() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{ActorSoldier, Command, ElementData, ElementKind, Entity, Posture};
    use crate::element_kinds::ActionState;

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();

    let mut attacker_element = ElementData {
        kind: ElementKind::ActorSoldier,
        // Soldiers built ad-hoc in tests need an explicit posture —
        // the level deserialiser remaps `Undefined` to a kind-specific
        // default, but `ElementData::default()` does not.
        posture: Posture::Upright,
        ..Default::default()
    };
    attacker_element.set_position(WorldPoint3D {
        x: 100.0,
        y: 100.0,
        z: 0.0,
    });
    let attacker_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: attacker_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    let mut defender_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    defender_element.set_position(WorldPoint3D {
        x: 160.0,
        y: 100.0,
        z: 0.0,
    });
    let defender_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: defender_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    if let Some(attacker) = engine.world.entities.get_mut(attacker_id) {
        attacker.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = attacker.human_data_mut().unwrap();
        human.opponents.push(defender_id);
        human.smalltalk_initiative = true;
        human.received_smalltalk_initiative = true;
    }
    if let Some(defender) = engine.world.entities.get_mut(defender_id) {
        defender.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        defender
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker_id);
    }

    engine.control.frame_counter = 15;
    crate::sim_rng::with_seed(1, || {
        engine.tick_smalltalk(&assets, &[]);
    });

    let attacker_human = engine
        .get_entity(attacker_id)
        .and_then(|e| e.human_data())
        .unwrap();
    let defender_human = engine
        .get_entity(defender_id)
        .and_then(|e| e.human_data())
        .unwrap();

    assert!(attacker_human.smalltalk_initiative);
    assert!(!defender_human.smalltalk_initiative);
    assert!(matches!(
        defender_human.smalltalk_hint,
        crate::element::SmalltalkHint::Left | crate::element::SmalltalkHint::Right
    ));
    assert_eq!(defender_human.smalltalk_hint_opponent, Some(attacker_id));

    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(defender_id, |command| matches!(
                command,
                Command::ParrySmalltalkLeft | Command::ParrySmalltalkRight
            ))
    );
}

#[test]
fn smalltalk_hint_suppresses_normal_swordfight_evaluation() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ActorPc, ActorSoldier, Command, ElementData, ElementKind, Entity, Posture, SmalltalkHint,
    };
    use crate::element_kinds::ActionState;

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();

    let mut pc_element = ElementData {
        kind: ElementKind::ActorPc,
        posture: Posture::Upright,
        ..Default::default()
    };
    pc_element.set_position(WorldPoint3D {
        x: 100.0,
        y: 100.0,
        z: 0.0,
    });
    let pc_id = engine.add_entity(Entity::Pc(ActorPc {
        element: pc_element,
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    }));

    let mut soldier_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    soldier_element.set_position(WorldPoint3D {
        x: 130.0,
        y: 100.0,
        z: 0.0,
    });
    let soldier_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: soldier_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    if let Some(pc) = engine.world.entities.get_mut(pc_id) {
        pc.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = pc.human_data_mut().unwrap();
        human.opponents.push(soldier_id);
        human.tiredness = 100;
        human.smalltalk_hint = SmalltalkHint::Left;
        human.smalltalk_hint_opponent = Some(soldier_id);
    }
    if let Some(soldier) = engine.world.entities.get_mut(soldier_id) {
        soldier.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        soldier.human_data_mut().unwrap().opponents.push(pc_id);
    }

    let consumed_smalltalk_hint_actors = engine.tick_evaluate_swordfight(&assets);

    let pc_human = engine
        .get_entity(pc_id)
        .and_then(|e| e.human_data())
        .unwrap();
    assert_eq!(consumed_smalltalk_hint_actors, vec![pc_id]);
    assert_eq!(pc_human.smalltalk_hint, SmalltalkHint::None);
    assert_eq!(pc_human.smalltalk_hint_opponent, None);
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(pc_id, |command| {
                command == Command::SwordstrikeTired
            })
    );
}

#[test]
fn consumed_smalltalk_hint_suppresses_same_frame_smalltalk_strike_only_for_that_actor() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ActorSoldier, Command, ElementData, ElementKind, Entity, Posture, SmalltalkHint,
    };
    use crate::element_kinds::ActionState;

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();

    let mut hinted_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    hinted_element.set_position(WorldPoint3D {
        x: 100.0,
        y: 100.0,
        z: 0.0,
    });
    let hinted_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: hinted_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    let mut hinted_opponent_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    hinted_opponent_element.set_position(WorldPoint3D {
        x: 160.0,
        y: 100.0,
        z: 0.0,
    });
    let hinted_opponent_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: hinted_opponent_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    let mut free_attacker_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    free_attacker_element.set_position(WorldPoint3D {
        x: 300.0,
        y: 100.0,
        z: 0.0,
    });
    let free_attacker_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: free_attacker_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    let mut free_defender_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    free_defender_element.set_position(WorldPoint3D {
        x: 360.0,
        y: 100.0,
        z: 0.0,
    });
    let free_defender_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: free_defender_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    if let Some(hinted) = engine.world.entities.get_mut(hinted_id) {
        hinted.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = hinted.human_data_mut().unwrap();
        human.opponents.push(hinted_opponent_id);
        human.smalltalk_initiative = true;
        human.received_smalltalk_initiative = true;
        human.smalltalk_hint = SmalltalkHint::Left;
        human.smalltalk_hint_opponent = Some(hinted_opponent_id);
    }
    if let Some(hinted_opponent) = engine.world.entities.get_mut(hinted_opponent_id) {
        hinted_opponent.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        hinted_opponent
            .human_data_mut()
            .unwrap()
            .opponents
            .push(hinted_id);
    }
    if let Some(free_attacker) = engine.world.entities.get_mut(free_attacker_id) {
        free_attacker.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = free_attacker.human_data_mut().unwrap();
        human.opponents.push(free_defender_id);
        human.smalltalk_initiative = true;
        human.received_smalltalk_initiative = true;
    }
    if let Some(free_defender) = engine.world.entities.get_mut(free_defender_id) {
        free_defender.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        free_defender
            .human_data_mut()
            .unwrap()
            .opponents
            .push(free_attacker_id);
    }

    let consumed_smalltalk_hint_actors = engine.tick_evaluate_swordfight(&assets);
    crate::sim_rng::with_seed(1, || {
        engine.tick_smalltalk(&assets, &consumed_smalltalk_hint_actors);
    });

    assert_eq!(consumed_smalltalk_hint_actors, vec![hinted_id]);
    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(hinted_id, |command| {
                matches!(
                    command,
                    Command::ParrySmalltalkLeft | Command::ParrySmalltalkRight
                )
            })
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(hinted_id, |command| {
                matches!(
                    command,
                    Command::SwordstrikeSmalltalkLeft | Command::SwordstrikeSmalltalkRight
                )
            })
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(free_attacker_id, |command| {
                matches!(
                    command,
                    Command::SwordstrikeSmalltalkLeft | Command::SwordstrikeSmalltalkRight
                )
            })
    );
    assert_ne!(
        engine
            .get_entity(free_defender_id)
            .and_then(|e| e.human_data())
            .unwrap()
            .smalltalk_hint,
        SmalltalkHint::None
    );
}

#[test]
fn sword_movement_start_transfers_smalltalk_initiative() {
    use crate::element::{ActorSoldier, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();

    let attacker_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));
    let defender_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    if let Some(attacker) = engine.world.entities.get_mut(attacker_id) {
        let human = attacker.human_data_mut().unwrap();
        human.opponents.push(defender_id);
        human.smalltalk_initiative = true;
    }
    if let Some(defender) = engine.world.entities.get_mut(defender_id) {
        let human = defender.human_data_mut().unwrap();
        human.opponents.push(attacker_id);
        human.smalltalk_initiative = false;
        human.received_smalltalk_initiative = false;
    }

    engine.apply_sword_movement_start_initiative_transfer(attacker_id);

    let attacker_human = engine
        .get_entity(attacker_id)
        .and_then(|e| e.human_data())
        .unwrap();
    let defender_human = engine
        .get_entity(defender_id)
        .and_then(|e| e.human_data())
        .unwrap();
    assert!(!attacker_human.smalltalk_initiative);
    assert!(defender_human.smalltalk_initiative);
    assert!(defender_human.received_smalltalk_initiative);
}

#[test]
fn sort_for_minimap_display_then_creation_tiebreak() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{ActorSoldier, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();

    // All same priority (soldier); sort falls back to display_order
    // then EntityId (insertion / creation order).  Soldiers with no
    // sprite fall back to position.y as their display_order (matches
    // sort_for_display).
    let mk = |y: f32| {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            ..Default::default()
        };
        element.set_position(WorldPoint3D { x: 0.0, y, z: 0.0 });
        Entity::Soldier(ActorSoldier {
            element,
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        })
    };

    let late_high_y = engine.add_entity(mk(100.0));
    let early_low_y = engine.add_entity(mk(10.0));
    let mid_mid_y = engine.add_entity(mk(50.0));
    // Two entities share a y value — EntityId (insertion order) breaks the tie.
    let first_tie = engine.add_entity(mk(10.0));
    let second_tie = engine.add_entity(mk(10.0));

    let sorted = engine.sort_for_minimap();

    // Among y=10 entities, EntityId decides: early_low_y < first_tie < second_tie.
    let idx = |id| sorted.iter().position(|&e| e == id).unwrap();
    assert!(idx(early_low_y) < idx(first_tie));
    assert!(idx(first_tie) < idx(second_tie));
    // Higher y values come later in the sort.
    assert!(idx(second_tie) < idx(mid_mid_y));
    assert!(idx(mid_mid_y) < idx(late_high_y));
}

#[test]
fn camera_slide_approaches_target() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4000.0, 3000.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(100.0, 100.0);
    engine.feedback.cutscene_camera.camera_slide = crate::coordinates::MapPoint::new(500.0, 300.0);
    engine.feedback.cutscene_camera.camera_wanted = crate::coordinates::MapPoint::new(500.0, 300.0);
    engine.control.speed = 1.0;

    engine.perform_director_work(&mut display);

    // Should have set Scroll display op (or moved toward target)
    // The scrolling vector should point toward the target
    let sv = display.background_transform.scrolling_vector;
    // At speed=1, direction is normalized*1 then floored, so we check general direction
    assert!(sv.x >= 0.0 || sv.y >= 0.0);
}

#[test]
fn camera_slide_cancels_at_target() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4000.0, 3000.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(500.0, 300.0);
    engine.feedback.cutscene_camera.camera_slide = crate::coordinates::MapPoint::new(500.0, 300.0);

    engine.perform_director_work(&mut display);

    // Should have cancelled the slide
    assert!(!engine.feedback.cutscene_camera.is_sliding());
}

#[test]
fn resize_aborts_zoom() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    display.display_op = DisplayOpCode::InZoom;
    display.background_transform.zoom_to_up = true;
    engine.feedback.cutscene_camera.zoom_init_done = true;

    engine.resize(&mut display, 1024.0, 768.0);

    assert!(!display.background_transform.zoom_to_up);
    assert!(!engine.feedback.cutscene_camera.zoom_init_done);
}

#[test]
fn dead_pc_triggers_failure() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4000.0, 3000.0);

    // Add a PC entity
    let mut pc_elem = crate::element::ElementData {
        kind: crate::element::ElementKind::ActorPc,
        ..Default::default()
    };
    pc_elem.set_position_map(crate::coordinates::MapPoint::new(100.0, 200.0));
    let entity = Entity::Pc(crate::element::ActorPc {
        element: pc_elem,
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let id = engine.add_entity(entity);
    engine.mission_domain.dead_pc = Some(id);

    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelFailed);
}

#[test]
fn non_playable_pc_does_not_prevent_default_loss() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();

    let entity = Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            active: true,
            posture: crate::element::Posture::Upright,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        pc: crate::element::PcData {
            playable: false,
            life_points: 100,
            ..Default::default()
        },
    });
    engine.add_entity(entity);

    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;

    assert_eq!(result, GameCode::LevelFailed);
}

#[test]
fn zoom_step_completes_after_8_steps() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    display.background_transform.zoom_to_up = true;
    display.background_transform.zoom_count = 0;
    display.background_transform.number_of_zoom_steps = 8;
    engine.feedback.cutscene_camera.zoom_init_done = true;
    // Apply the post-draw reset to `NoBackgroundMove` so
    // `set_operation(InZoom)` can propagate (`set_operation` is
    // monotonic).
    display.display_op = DisplayOpCode::NoBackgroundMove;

    // Run 7 steps — should stay in InZoom
    for _ in 0..7 {
        engine.perform_zoom_step(&mut display);
        assert_eq!(display.display_op, DisplayOpCode::InZoom);
    }

    // 8th step — should finalize
    engine.perform_zoom_step(&mut display);
    assert_eq!(display.display_op, DisplayOpCode::NoBackgroundMove);
    assert!(!display.background_transform.zoom_to_up);
    assert!(!engine.feedback.cutscene_camera.zoom_init_done);
}

// ── Scroll hourglass / IsTaken dispatch ──────────────────────

/// The scroll tick counter starts at 0.
#[test]
fn scroll_default_hourglass_counter_is_zero() {
    let s = crate::element::ElementScroll::default();
    assert_eq!(s.script_hourglass_timeout, 0);
}

/// Without a mission script, the per-scroll Hourglass dispatcher
/// is a no-op and doesn't touch scroll state.
#[test]
fn dispatch_scroll_hourglasses_no_script_is_noop() {
    let mut engine = EngineInner::new();
    let scroll = Entity::Scroll(crate::element::ElementScroll {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ObjectScroll,
            active: true,
            ..Default::default()
        },
        ..Default::default()
    });
    let scroll_id = engine.add_entity(scroll);

    // No mission_script → nothing to dispatch, counter stays zero.
    let assets = crate::engine::LevelAssets::new();
    engine.dispatch_scroll_hourglasses(&assets);
    let entity = engine.get_entity(scroll_id);
    let counter = match entity {
        Some(Entity::Scroll(s)) => s.script_hourglass_timeout,
        _ => unreachable!("scroll entity missing"),
    };
    assert_eq!(counter, 0);
}

/// `scroll_is_taken` on a scroll without a bound script flips the
/// sprite to the "opened" pose and sets status to `Opened`, but
/// returns `false`.
#[test]
fn scroll_is_taken_without_script_returns_false_and_opens() {
    use super::scroll_reveal::ScrollStatus;

    let mut engine = EngineInner::new();
    let scroll = Entity::Scroll(crate::element::ElementScroll {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ObjectScroll,
            active: true,
            ..Default::default()
        },
        // No script_class — `IsClassInstanciate()` returns false.
        ..Default::default()
    });
    let scroll_id = engine.add_entity(scroll);
    // A PC to pass as the taker.  Its handle value is irrelevant
    // here since no script is bound; the non-instanciated branch
    // doesn't look at the PC pointer.
    let pc = Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let pc_id = engine.add_entity(pc);

    let assets = crate::engine::LevelAssets::new();
    let accepted = engine.scroll_is_taken(&assets, scroll_id, pc_id);
    assert!(!accepted);
    // Without `mission_script`, the status store isn't populated
    // either — the setter early-returns.  Covering the "happens to
    // have GameHost but no class" flow is left to the integration
    // level, so here we just confirm `false` + no panic.
    let _ = ScrollStatus::Opened; // keep symbol live
}

/// Build a minimal soldier entity for posture / command tests.
fn make_test_soldier(posture: crate::element::Posture) -> Entity {
    Entity::Soldier(crate::element::ActorSoldier {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorSoldier,
            posture,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    })
}

const SPEECH_TIMING_PROFILE_ID: u32 = 0x1234_0000;

fn build_mytalk_timing_test(duration_frames: Option<u32>) -> (EngineInner, EntityId, LevelAssets) {
    use crate::ai::{Remark, SpeechFlags};
    use crate::element::AiBrain;
    use crate::profiles::SoldierProfile;
    use crate::sound::ExclamationGroup;

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;

    let mut soldier_entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut soldier_entity else {
        unreachable!();
    };
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    let ai = soldier.npc.ai_brain.base_mut().unwrap();
    ai.current_remark = Remark::Arrow;
    ai.current_remark_flags = (SpeechFlags::MYTALK_1 | SpeechFlags::ALWAYS).bits();
    let soldier_id = engine.add_entity(soldier_entity);

    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .soldiers
        .push(SoldierProfile {
            profile_name: "timing-test-soldier".into(),
            exclamation_id: SPEECH_TIMING_PROFILE_ID,
            ..Default::default()
        });
    if let Some(frames) = duration_frames {
        std::sync::Arc::make_mut(&mut assets.exclamation_durations).insert(
            (
                ExclamationGroup::Civilian,
                SPEECH_TIMING_PROFILE_ID,
                Remark::Arrow as u16,
            ),
            frames,
        );
    }

    (engine, soldier_id, assets)
}

fn mytalk_ai(engine: &EngineInner, soldier_id: EntityId) -> &crate::ai::AiController {
    engine
        .get_entity(soldier_id)
        .and_then(Entity::ai_controller)
        .expect("timing-test soldier has an AI controller")
}

#[test]
fn mytalk_completion_obeys_exact_asset_duration_frame() {
    use crate::ai::{Remark, StimulusType};

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test(Some(3));
    engine.process_npc_speech(&assets);

    assert_eq!(engine.feedback.sound_sim.playing_exclamations.len(), 1);
    assert_eq!(
        engine.feedback.sound_sim.playing_exclamations[0].finish_frame,
        103
    );
    assert!(mytalk_ai(&engine, soldier_id).speech_in_flight);

    for frame in [101, 102] {
        engine.control.frame_counter = frame;
        super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, frame);
        engine.process_npc_speech(&assets);
        let ai = mytalk_ai(&engine, soldier_id);
        assert!(ai.speech_in_flight);
        assert_eq!(ai.current_remark, Remark::Arrow);
        assert!(ai.pending_self_stimuli.is_empty());
    }

    engine.control.frame_counter = 103;
    super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, 103);
    engine.process_npc_speech(&assets);
    let ai = mytalk_ai(&engine, soldier_id);
    assert!(!ai.speech_in_flight);
    assert_eq!(ai.current_remark, Remark::TheSoundOfSilence);
    assert_eq!(ai.pending_self_stimuli, vec![StimulusType::EventMyTalk1]);
}

#[test]
fn missing_exclamation_duration_completes_mytalk_at_next_boundary() {
    use crate::ai::{Remark, StimulusType};

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test(None);
    engine.process_npc_speech(&assets);

    assert_eq!(engine.feedback.sound_sim.playing_exclamations.len(), 1);
    assert_eq!(
        engine.feedback.sound_sim.playing_exclamations[0].finish_frame, 100,
        "missing metadata must not fabricate a 75-frame speech"
    );
    let ai = mytalk_ai(&engine, soldier_id);
    assert!(ai.speech_in_flight);
    assert_eq!(ai.current_remark, Remark::Arrow);

    engine.control.frame_counter = 101;
    super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, 101);
    engine.process_npc_speech(&assets);

    let ai = mytalk_ai(&engine, soldier_id);
    assert_eq!(engine.control.frame_counter, 101);
    assert!(!ai.speech_in_flight);
    assert_eq!(ai.current_remark, Remark::TheSoundOfSilence);
    assert_eq!(ai.pending_mytalk_flags, 0);
    assert_eq!(ai.pending_self_stimuli, vec![StimulusType::EventMyTalk1]);
}

/// Build a minimal civilian entity for NPC-translate tests.
fn make_test_civilian(posture: crate::element::Posture) -> Entity {
    Entity::Civilian(crate::element::ActorCivilian {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorCivilian,
            posture,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        civilian: Default::default(),
    })
}

fn make_test_pc(posture: crate::element::Posture) -> Entity {
    Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            posture,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    })
}

fn install_test_building_sector(engine: &mut EngineInner, raw_sector: u16) {
    let _sector = crate::position_interface::SectorHandle::new(raw_sector)
        .expect("test building sector must be non-zero");
    let mut level = crate::fast_find_grid::LevelGrid::default();
    level
        .sector_number_map
        .insert(crate::sector::SectorNumber::new(raw_sector as i16), 0);
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: MapBBox::new(),
        sector_type: crate::sector::SectorType::BUILDING,
        layer: 0,
        sector_number: crate::sector::SectorNumber::new(raw_sector as i16),
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
    engine.world.fast_grid.level = std::sync::Arc::new(level);
}

#[test]
fn selection_mark_skips_hidden_and_building_pcs() {
    let mut engine = EngineInner::new();
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    engine.players.seats[0].selection.push(pc_id);

    assert!(engine.pc_draws_selection_mark(pc_id));
    assert!(engine.any_selected_pc_drawing_selection_mark());

    if let Some(Entity::Pc(pc)) = engine.get_entity_mut(pc_id) {
        pc.element.hidden_in_building = true;
    }
    assert!(!engine.pc_draws_selection_mark(pc_id));
    assert!(!engine.any_selected_pc_drawing_selection_mark());

    if let Some(Entity::Pc(pc)) = engine.get_entity_mut(pc_id) {
        pc.element.hidden_in_building = false;
    }

    let sector_num = crate::position_interface::SectorHandle::new(42).unwrap();
    install_test_building_sector(&mut engine, 42);

    if let Some(Entity::Pc(pc)) = engine.get_entity_mut(pc_id) {
        pc.element.set_sector(Some(sector_num));
    }

    assert!(!engine.pc_draws_selection_mark(pc_id));
    assert!(!engine.any_selected_pc_drawing_selection_mark());
}

#[test]
fn enter_swordfight_clears_pending_bow_shot_list() {
    let mut engine = EngineInner::new();
    let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let opponent = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));

    let mut shot = crate::sequence::SequenceElement::new_interaction(
        1,
        crate::element::Command::ShootBow,
        Some(pc),
        Some(opponent),
    );
    shot.priority = crate::sequence::SequencePriority::Preference;
    let shot_seq = engine.orders.sequence_manager.launch_element(shot);
    assert!(engine.pc_has_pending_shoot_bow(pc));

    let _ = engine.enter_swordfight(&LevelAssets::new(), pc, opponent, false);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(shot_seq, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Interrupted
    );
    assert!(
        !engine.pc_has_pending_shoot_bow(pc),
        "C++ EnterSwordFight clears the actor's pending shoot list before validity checks"
    );
}

fn make_test_ai_soldier(camp: crate::element::Camp) -> Entity {
    let mut entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut entity else {
        unreachable!("make_test_soldier returned non-soldier");
    };
    soldier.soldier.cached_camp = camp;
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    entity
}

#[test]
fn nearby_fighters_keeps_inactive_self_and_filters_ineligible_others() {
    use crate::element::Posture;

    let mut engine = EngineInner::new();
    let self_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let other_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));

    for id in [self_id, other_id] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("test fighter exists")
        else {
            panic!("test fighter changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test fighter has enemy AI")
            .base
            .me = id.index();
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(self_soldier) =
        engine.get_entity_mut(self_id).expect("self fighter exists")
    else {
        panic!("self fighter changed kind")
    };
    self_soldier.element.active = false;

    let Entity::Soldier(other_soldier) = engine
        .get_entity_mut(other_id)
        .expect("other fighter exists")
    else {
        panic!("other fighter changed kind")
    };
    other_soldier.element.posture = Posture::Tied;

    let fighters = engine.build_nearby_fighters_for(self_id, &assets);
    assert_eq!(fighters.len(), 1);
    assert_eq!(fighters[0].handle, self_id.index());
    assert!(!fighters[0].is_able_to_fight);
    assert!(!fighters[0].is_dead);
    assert!(!fighters[0].is_unconscious);
    assert!(!fighters[0].is_carried);
}

fn run_synchronous_charly_report(officer_state: crate::ai::AiState) -> EngineInner {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::EyeStatus;

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    engine.world.weather.ambiance = crate::engine::types::Ambiance::Night;
    let charly_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let officer_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    for (id, x) in [(charly_id, 0.0), (officer_id, 200.0)] {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(id)
            .expect("test report soldier exists")
        else {
            panic!("test report entity changed kind")
        };
        soldier.element.active = true;
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.element.set_direction_instantly(4);
        soldier.npc.view_direction = [1.0, 0.0];
        soldier.npc.view_radius = 400;
        soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        soldier.npc.eye_status = EyeStatus::LookForward;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test report soldier has enemy AI")
            .base
            .me = id.index();
    }

    {
        let charly = engine
            .get_entity_mut(charly_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("test Charly has enemy AI");
        charly.base.antagonist = officer_id.index();
        charly.set_state(AiState::Seeking, Substate::SeekingCharlyGoToOfficer);
        charly.base.launch_timer(0, 100);
        charly.base.timer_is_running = false;
    }
    {
        let officer = engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("test officer has enemy AI");
        officer.set_state(officer_state, Substate::DefaultOnPost);
    }

    let scratch = engine.build_sim_scratch(&assets);
    let ctx = {
        let entity = engine
            .get_entity(charly_id)
            .expect("test Charly exists for context");
        crate::engine::ai::build_ai_context_from_entity(
            entity,
            engine.control.frame_counter,
            None,
            engine.world.weather.is_forest_level,
            engine.world.weather.ambiance,
            engine.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &engine.world.fast_grid,
            &assets.hiking_paths,
            &engine.ai.global.all_soldier_handles,
        )
    };
    assert!(ctx.is_night_or_fog);
    let tick = engine.build_npc_tick_data(charly_id, &scratch, &assets);
    engine.dispatch_think_with_drain(
        charly_id,
        &Stimulus::new(StimulusType::EventTimer),
        &ctx,
        &tick,
        &assets,
    );
    engine
}

#[test]
fn charly_report_uses_synchronous_officer_acceptance_and_refusal() {
    use crate::ai::{AiState, Substate};

    let accepted = run_synchronous_charly_report(AiState::Default);
    let charly = accepted
        .world
        .entities
        .soldiers()
        .next()
        .expect("accepted Charly exists")
        .1
        .npc
        .ai_brain
        .enemy()
        .expect("accepted Charly has enemy AI");
    assert_eq!(
        charly.base.current_substate,
        Substate::SeekingCharlyGoToOfficerSeen
    );
    assert_eq!(charly.base.when_does_timer_ring, 110);

    let refused = run_synchronous_charly_report(AiState::Attacking);
    let charly = refused
        .world
        .entities
        .soldiers()
        .next()
        .expect("refused Charly exists")
        .1
        .npc
        .ai_brain
        .enemy()
        .expect("refused Charly has enemy AI");
    assert_eq!(charly.base.current_state, AiState::Default);
    assert_ne!(
        charly.base.current_substate,
        Substate::SeekingCharlyGoToOfficerSeen
    );
}

#[test]
fn ai_entity_views_keep_inactive_humans_for_same_building_detection() {
    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("inactive snapshot soldier exists")
    else {
        panic!("inactive snapshot entity changed kind")
    };
    soldier.element.active = false;

    let scratch = engine.build_sim_scratch(&LevelAssets::new());
    let view = scratch
        .ai_entity_views
        .get(&soldier_id.index())
        .expect("inactive human must remain available to same-building IsDetecting");
    assert!(!view.active);
}

#[test]
fn messenger_selection_followup_retargets_recording_before_frame_returns() {
    use crate::messenger::{Message, MessageType, PcMessage};

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let second = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    engine.players.seats[0].selection = vec![first];

    // RHMessenger::ForwardMessage handles these calls synchronously.  In
    // particular, SelectCharacter's recursive UpdateRecordingMacro must
    // run before ForwardMessage returns, so the recording target changes
    // in this frame rather than surviving as queued work for the next one.
    engine
        .orders
        .messenger
        .send(Message::pc(PcMessage::StartRecordingMacro, Some(first)));
    engine
        .orders
        .messenger
        .send(Message::pc(PcMessage::SelectCharacter, Some(second)));

    let mut assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    assert_eq!(engine.players.seats[0].selection, vec![second]);
    assert_eq!(
        engine.players.qa_recording_for,
        vec![second],
        "SelectCharacter -> UpdateRecordingMacro must complete in the originating frame"
    );
    assert!(
        engine
            .orders
            .messenger
            .drain()
            .into_iter()
            .all(|msg| msg.msg_type != MessageType::Pc(PcMessage::UpdateRecordingMacro, None)),
        "the recursive recording update must not remain queued for the next frame"
    );
}

fn set_test_soldier_brawl_got_hit(engine: &mut EngineInner, soldier: EntityId) {
    use crate::ai::{AiState, Substate};

    let entity = engine
        .get_entity_mut(soldier)
        .expect("test soldier present");
    let npc = entity.npc_data_mut().expect("test soldier is an NPC");
    npc.ai_brain =
        crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::new(soldier.index())));
    npc.ai_brain
        .enemy_mut()
        .expect("enemy brain installed")
        .set_state(AiState::Wondering, Substate::WonderingBrawlGotHit);
}

#[test]
fn self_stimulus_chain_reenters_until_stable_in_originating_frame() {
    use crate::ai::{StimulusType, Substate};

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    set_test_soldier_brawl_got_hit(&mut engine, soldier);
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .fire_self_stimulus(StimulusType::EventDone);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.drain_pending_self_stimuli(&assets);

    let ai = engine.get_entity(soldier).unwrap().ai_controller().unwrap();
    assert_eq!(
        ai.current_substate,
        Substate::WonderingWatchingForMoreMoney,
        "GotHit EventDone recursively fires EventDone in Recovering before the outer Think returns"
    );
    assert!(
        ai.pending_self_stimuli.is_empty(),
        "a recursive self-stimulus must not leak into the next frame"
    );
    assert!(ai.pending_look_sidewards.is_none());
    assert!(
        engine.orders.sequence_manager.sequences_iter().any(|seq| {
            seq.elements.iter().any(|elem| {
                matches!(
                    elem.command,
                    crate::element::Command::LookLeft | crate::element::Command::LookRight
                )
            })
        }),
        "the recursively selected look action must enter same-frame sequence arbitration"
    );
}

#[test]
fn condolation_reenters_think_before_dispatch_returns() {
    use crate::ai::Substate;
    use crate::element::Command;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    set_test_soldier_brawl_got_hit(&mut engine, soldier);

    let seq_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::LookLeft, Some(soldier)));
    engine
        .orders
        .sequence_manager
        .element_in_progress(seq_id, 0);
    engine.orders.sequence_manager.element_terminated(seq_id, 0);
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.dispatch_condolations(&assets);

    let ai = engine.get_entity(soldier).unwrap().ai_controller().unwrap();
    assert_eq!(
        ai.current_substate,
        Substate::WonderingWatchingForMoreMoney,
        "SetState -> SendCondolationCard -> Think(EventDone) must finish before dispatch returns"
    );
    assert!(ai.pending_self_stimuli.is_empty());
    assert!(ai.pending_look_sidewards.is_none());
    assert!(
        engine.orders.sequence_manager.sequences_iter().any(|seq| {
            seq.elements.iter().any(|elem| {
                matches!(
                    elem.command,
                    crate::element::Command::LookLeft | crate::element::Command::LookRight
                )
            })
        }),
        "condolation re-entry must launch its follow-up before dispatch returns"
    );
}

#[test]
fn condolation_followup_arbitrates_before_parent_sequence_successor() {
    use crate::ai::{AiState, Substate};
    use crate::element::Command;
    use crate::sequence::{Sequence, SequenceAction, SequenceElement};

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    set_test_soldier_brawl_got_hit(&mut engine, soldier);

    let mut parent = Sequence::new();
    parent.append_element(SequenceElement::new(1, Command::LookLeft, Some(soldier)));
    // IsLastRealAction explicitly skips Wait/AssertPosition successors,
    // so the LookLeft condolence still fires before Ready queues this.
    parent.append_element(SequenceElement::new(2, Command::Wait, Some(soldier)));
    let parent_id = engine.orders.sequence_manager.launch_sequence(parent);

    let initial = engine.orders.sequence_manager.hourglass();
    assert_eq!(initial.len(), 1);
    engine
        .orders
        .sequence_manager
        .element_in_progress(parent_id, 0);
    engine
        .orders
        .sequence_manager
        .element_terminated(parent_id, 0);
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.dispatch_condolations(&assets);

    let commands: Vec<_> = engine
        .orders
        .sequence_manager
        .hourglass()
        .into_iter()
        .map(|action| {
            let (seq_id, elem_idx) = match action {
                SequenceAction::InstructOwner {
                    sequence_id,
                    element_index,
                    ..
                }
                | SequenceAction::EngineCommand {
                    sequence_id,
                    element_index,
                }
                | SequenceAction::ExecuteImmediateOwner {
                    sequence_id,
                    element_index,
                    ..
                }
                | SequenceAction::ExecuteImmediateEngine {
                    sequence_id,
                    element_index,
                } => (sequence_id, element_index),
            };
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .expect("queued action still has an element")
                .command
        })
        .collect();

    assert_eq!(
        commands,
        vec![
            Command::EnterAttentiveMode,
            Command::LookLeft,
            Command::Wait,
        ],
        "SendCondolationCard's recursive Think must launch/arbitrate its action before Ready queues the parent's next level"
    );

    let ai = engine.get_entity(soldier).unwrap().ai_controller().unwrap();
    assert_eq!(ai.current_state, AiState::Wondering);
    assert_eq!(ai.current_substate, Substate::WonderingWatchingForMoreMoney);
}

#[test]
fn condolation_cascade_crosses_owners_before_outer_dispatch_returns() {
    use crate::element::Command;
    use crate::sequence::{CascadeFlags, Sequence, SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let second = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let third = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    for owner in [second, third] {
        engine
            .get_entity_mut(owner)
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .wasp_victim = true;
    }

    let mut seq = Sequence::new();
    seq.append_element(SequenceElement::new(1, Command::LookLeft, Some(first)));
    seq.append_element(SequenceElement::new(
        2,
        Command::ReceiveWaspSting,
        Some(second),
    ));
    seq.append_element(SequenceElement::new(
        3,
        Command::ReceiveWaspSting,
        Some(third),
    ));
    let seq_id = engine.orders.sequence_manager.launch_sequence(seq);

    engine
        .orders
        .sequence_manager
        .element_interrupted(seq_id, 0, CascadeFlags::NEXT_LEVEL);
    engine.dispatch_condolations_for_npc(first, &LevelAssets::new());

    for (idx, owner) in [(1, second), (2, third)] {
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, idx)
                .unwrap()
                .state,
            SequenceState::Interrupted
        );
        assert!(
            !engine
                .get_entity(owner)
                .unwrap()
                .npc_data()
                .unwrap()
                .wasp_victim,
            "cross-owner card {idx} must run inside the originating SetState cascade"
        );
    }
    assert!(
        engine
            .orders
            .sequence_manager
            .drain_pending_condolations()
            .is_empty()
    );
}

#[test]
fn primary_target_tracking_precedes_view_refresh() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    let soldier_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let soldier_pos = MapPoint::new(100.0, 100.0);
    let target_pos = MapPoint::new(100.0, 200.0);

    if let Some(Entity::Soldier(soldier)) = engine.get_entity_mut(soldier_id) {
        soldier.element.active = true;
        soldier.element.set_position_map(soldier_pos);
        soldier.element.set_direction_instantly(4);
        soldier.npc.direction_old = 4;
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test soldier has enemy AI");
        ai.base.me = soldier_id.index();
        ai.base.primary_target = pc_id.index();
        ai.base.current_state = crate::ai::AiState::Attacking;
        ai.base.current_substate = crate::ai::Substate::AttackingReactiontime;
    }
    if let Some(Entity::Pc(pc)) = engine.get_entity_mut(pc_id) {
        pc.element.active = true;
        pc.element.set_position_map(target_pos);
    }

    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let expected = crate::position_interface::vector_to_sector_0_to_15_iso(
        target_pos.x - soldier_pos.x,
        target_pos.y - soldier_pos.y,
    );
    let Entity::Soldier(soldier) = engine.get_entity(soldier_id).unwrap() else {
        panic!("test soldier changed entity kind");
    };
    assert_eq!(soldier.element.direction(), expected);
    assert_eq!(
        soldier.npc.direction_old, expected,
        "RefreshView must observe the combat tracking direction in the same frame"
    );
}

#[test]
fn npc_hourglass_observes_exact_original_phase_order() {
    use super::tick::{NpcHourglassPhase as Phase, capture_npc_hourglass_phases};

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    let (_, phases) =
        capture_npc_hourglass_phases(|| engine.perform_hourglass(&mut display, &assets, &mut dev));

    assert_eq!(
        phases,
        vec![
            Phase::SoldierPrelude,
            Phase::Patrol,
            Phase::BaseHuman,
            Phase::Broadcasts,
            Phase::View,
            Phase::Detection,
            Phase::Ambush,
            Phase::Busy,
            Phase::Ladder,
            Phase::LockGate,
            Phase::SixteenthFrame,
            Phase::NormalTimer,
            Phase::MacroTimer,
            Phase::QueuedStimuli,
        ]
    );
}

#[test]
fn npc_detection_observes_friend_state_at_creation_order_boundary() {
    use crate::ai::{AiState, Substate};
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};
    use crate::profiles::ProfileRank;

    fn observe(attacker_before_officer: bool) -> (AiState, AiState, Substate) {
        let mut engine = EngineInner::new();
        // Keep the relevant NPCs in slots 1/2 in both arrangements so the
        // swapped oracle is not confounded by a slot-zero special case.
        engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..ElementData::default()
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let attacker = make_test_ai_soldier(Camp::Lacklandists);
        let officer = make_test_ai_soldier(Camp::Lacklandists);
        let (attacker_id, officer_id) = if attacker_before_officer {
            (engine.add_entity(attacker), engine.add_entity(officer))
        } else {
            let officer_id = engine.add_entity(officer);
            let attacker_id = engine.add_entity(attacker);
            (attacker_id, officer_id)
        };
        let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

        for (id, x) in [(officer_id, 0.0), (attacker_id, 120.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("creation-order detection soldier exists")
            else {
                panic!("creation-order detection entity changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D { x, y: 0.0, z: 0.0 });
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.element.set_direction_instantly(4);
            soldier.npc.life_points = 100;
            soldier.npc.view_direction = [1.0, 0.0];
            soldier.npc.view_radius = 135;
            soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
            soldier.npc.eye_status = crate::element::EyeStatus::Stare;
            let ai = soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("creation-order detection soldier has enemy AI");
            ai.base.me = id.index();
            ai.soldier_profile_rank = if id == officer_id {
                ProfileRank::Officer
            } else {
                ProfileRank::Soldier
            };
        }

        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("creation-order detection PC exists")
        else {
            panic!("creation-order detection target changed kind")
        };
        pc.element.active = true;
        pc.element.set_position(crate::coordinates::WorldPoint3D {
            x: 175.0,
            y: 0.0,
            z: 0.0,
        });
        pc.element.set_position_map(MapPoint::new(175.0, 0.0));
        pc.pc.life_points = 100;

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        // Isolate the exact A-EVENT_VIEW → B-FRIEND edge after fixture
        // initialization has installed profiles and AI runtime defaults.
        let Entity::Soldier(attacker) = engine
            .get_entity_mut(attacker_id)
            .expect("attacker exists before detection")
        else {
            panic!("attacker changed kind")
        };
        attacker.npc.detectable_lists[DetectableType::Enemy as usize].clear();
        attacker.npc.detectable_lists[DetectableType::Friend as usize].clear();
        attacker.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        attacker.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(pc_id),
            detectable_type: DetectableType::Enemy,
            // This oracle starts after the ordinary predetection shadow edge.
            shadow_seen_last_frame: true,
            ..Detectable::default()
        });

        let Entity::Soldier(officer) = engine
            .get_entity_mut(officer_id)
            .expect("officer exists before detection")
        else {
            panic!("officer changed kind")
        };
        officer.npc.detectable_lists[DetectableType::Friend as usize].clear();
        officer.npc.detectable_lists[DetectableType::Friend as usize].push(Detectable {
            element: Some(attacker_id),
            detectable_type: DetectableType::Friend,
            ..Detectable::default()
        });

        crate::sim_rng::with_seed(0xA013, || engine.tick_enemy_ai(&assets));

        let attacker_ai = engine
            .get_entity(attacker_id)
            .and_then(Entity::enemy_ai)
            .expect("attacker remains an enemy AI");
        let officer_ai = engine
            .get_entity(officer_id)
            .and_then(Entity::enemy_ai)
            .expect("officer remains an enemy AI");
        (
            attacker_ai.base.current_state,
            officer_ai.base.current_state,
            officer_ai.base.current_substate,
        )
    }

    let attacker_first = observe(true);
    assert_eq!(attacker_first.0, AiState::Attacking);
    assert_eq!(
        attacker_first.1,
        AiState::Default,
        "later officer must see that the earlier EVENT_VIEW made its friend unable to help"
    );
    assert_ne!(attacker_first.2, Substate::SeekingOfficerCallSoldier);
    assert_eq!(
        observe(false),
        (
            AiState::Attacking,
            AiState::Seeking,
            Substate::SeekingOfficerCallSoldier,
        ),
        "earlier officer must see the still-helpful soldier before that soldier handles EVENT_VIEW"
    );
}

#[test]
fn npc_hearing_thinks_before_same_slot_optical_detection() {
    use crate::ai::AiState;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    // Keep the NPC out of legacy slot zero and choose the frame so its
    // `(frame + creation_order) % DETECTION_FREQUENCY_SOUNDS` gate is open.
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    engine.control.frame_counter = 2;

    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let stale_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Dead));

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("hearing-order soldier exists")
    else {
        panic!("hearing-order entity changed kind")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(crate::coordinates::WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(4);
    soldier.npc.life_points = 100;
    soldier.npc.view_direction = [1.0, 0.0];
    soldier.npc.view_radius = 135;
    soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    soldier.npc.eye_status = crate::element::EyeStatus::Stare;
    let ai = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("hearing-order soldier has enemy AI");
    ai.base.me = soldier_id.index();

    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("hearing-order PC exists")
    else {
        panic!("hearing-order target changed kind")
    };
    pc.element.active = true;
    pc.element.set_position(crate::coordinates::WorldPoint3D {
        x: 55.0,
        y: 0.0,
        z: 0.0,
    });
    pc.element.set_position_map(MapPoint::new(55.0, 0.0));
    pc.pc.life_points = 100;

    let Entity::Pc(stale_pc) = engine
        .get_entity_mut(stale_pc_id)
        .expect("stale hearing-order PC exists")
    else {
        panic!("stale hearing-order target changed kind")
    };
    stale_pc.element.active = false;
    stale_pc.pc.life_points = 0;

    // RunningUpright on ground produces a 70-volume TAPTAPTAP. At 55 units
    // this becomes the original's 15-volume subjective noise, while keeping
    // EVENT_VIEW outside the unrelated close-combat branch (< 50 units).
    // Install it as the production snapshot builder's current animation
    // instead of injecting a synthetic noise into the detection helper.
    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(pc_id),
        OrderType::RunningUpright,
    );
    movement
        .orders
        .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("soldier exists before hearing-order detection")
    else {
        panic!("hearing-order soldier changed kind")
    };
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    soldier.npc.detection_suspects[DetectableType::Enemy as usize] = 0;
    // Acoustics runs before optical CleanUpDetectables. A just-dead PC can
    // therefore still occupy an earlier enemy-list slot without appearing in
    // the alive-only world snapshot; it must not block the later audible PC.
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(stale_pc_id),
        detectable_type: DetectableType::Enemy,
        ..Detectable::default()
    });
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(pc_id),
        detectable_type: DetectableType::Enemy,
        // Keep the oracle about HEAR → VIEW rather than predetection shadow.
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0EAD, || engine.tick_enemy_ai(&assets));

    assert_eq!(
        engine
            .get_entity(pc_id)
            .and_then(Entity::actor_data)
            .expect("hearing-order PC remains an actor")
            .last_noise_volume,
        70,
        "production PC snapshot must derive the expected running noise"
    );
    assert!(
        engine
            .get_entity(soldier_id)
            .and_then(Entity::npc_data)
            .expect("hearing-order soldier remains an NPC")
            .detectable_lists[DetectableType::Enemy as usize][0]
            .heard_last_frame,
        "production acoustic pass must reach UpdateHearing's latch"
    );
    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("soldier remains an enemy AI");
    assert_eq!(
        ai.base.current_state,
        AiState::Attacking,
        "synchronous EVENT_HEAR must make optical detection instant in the same RefreshDetection call"
    );
}

#[test]
fn lackland_detection_scans_and_retains_full_fifo_while_ai_locked() {
    use crate::ai::{AiLockFlags, AiState, StimulusInfo, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{
        Camp, Detectable, DetectableType, ElementBonus, ElementData, ElementKind, Entity,
    };
    use crate::element_kinds::ObjectType;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    engine.control.frame_counter = 2;

    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let first_visible_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let lost_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let last_visible_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let body_id = engine.add_entity(make_test_pc(crate::element::Posture::Dead));
    let object_id = engine.add_entity(Entity::Bonus(ElementBonus {
        element: ElementData {
            kind: ElementKind::ObjectBonus,
            active: true,
            ..ElementData::default()
        },
        object: crate::element::ObjectData {
            object_type: ObjectType::Coin,
            ..crate::element::ObjectData::default()
        },
    }));
    let friend_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("locked detection observer exists")
    else {
        panic!("locked detection observer changed kind")
    };
    observer.element.active = true;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.life_points = 100;
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    for (id, x, life_points) in [
        (first_visible_id, 55.0, 100),
        (lost_id, -200.0, 100),
        (last_visible_id, 80.0, 100),
        (body_id, 100.0, 0),
    ] {
        let Entity::Pc(pc) = engine
            .get_entity_mut(id)
            .expect("locked detection PC exists")
        else {
            panic!("locked detection PC changed kind")
        };
        pc.element.active = true;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(x, 0.0));
        pc.pc.life_points = life_points;
    }

    let Entity::Bonus(object) = engine
        .get_entity_mut(object_id)
        .expect("locked detection object exists")
    else {
        panic!("locked detection object changed kind")
    };
    object
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0));
    object.element.set_position_map(MapPoint::new(100.0, 0.0));

    let Entity::Soldier(friend) = engine
        .get_entity_mut(friend_id)
        .expect("locked observer's friend exists")
    else {
        panic!("locked observer's friend changed kind")
    };
    friend.element.active = true;
    friend
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(-20.0, 20.0, 0.0));
    friend.element.set_position_map(MapPoint::new(-20.0, 20.0));
    friend.npc.life_points = 100;
    friend.npc.eye_status = crate::element::EyeStatus::Closed;

    // RunningUpright produces the production 70-volume TAPTAPTAP used by
    // RefreshDetection's acoustic pass. With observer slot 1 and frame 2,
    // its three-frame hearing cadence is open.
    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(first_visible_id),
        OrderType::RunningUpright,
    );
    movement
        .orders
        .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("locked detection observer exists after fixture")
    else {
        panic!("locked detection observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("locked detection observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::FREEZE;

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    observer.npc.detectable_lists[DetectableType::Body as usize].clear();
    observer.npc.detectable_lists[DetectableType::Object as usize].clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.detection_suspects[DetectableType::Body as usize] = 999;
    observer.npc.detection_suspects[DetectableType::Object as usize] = 999;
    for (target_id, seen_last_frame) in [
        (first_visible_id, false),
        (lost_id, true),
        (last_visible_id, false),
    ] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            ..Detectable::default()
        });
    }
    observer.npc.detectable_lists[DetectableType::Body as usize].push(Detectable {
        element: Some(body_id),
        detectable_type: DetectableType::Body,
        // Keep this oracle's shadow prefix confined to the Enemy bucket.
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });
    observer.npc.detectable_lists[DetectableType::Object as usize].push(Detectable {
        element: Some(object_id),
        detectable_type: DetectableType::Object,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0B22, || engine.tick_enemy_ai(&assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("locked detection observer remains an NPC");
    assert_eq!(
        observer.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| (
                det.heard_last_frame,
                det.shadow_seen_last_frame,
                det.seen_last_frame,
            ))
            .collect::<Vec<_>>(),
        vec![
            (true, true, true),
            (false, false, false),
            (false, true, true)
        ],
        "AI lock must not suppress acoustic, predetection, or optical latch updates"
    );
    assert_eq!(
        observer.detection_suspects[DetectableType::Enemy as usize],
        0,
        "locked Enemy detection must still commit and reset suspects"
    );
    assert!(
        observer.detectable_lists[DetectableType::Body as usize].is_empty(),
        "locked non-Enemy buckets must still commit one-shot detectables"
    );
    assert!(
        observer.detectable_lists[DetectableType::Object as usize].is_empty(),
        "locked Object detection must still commit its one-shot detectable"
    );

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("locked detection observer retains enemy AI");
    assert_eq!(
        (ai.base.current_state, ai.base.current_substate),
        (AiState::Default, Substate::DefaultOnPost)
    );
    assert!(ai.base.pending_stimuli.is_empty());
    assert_eq!(ai.base.last_stimulus_actor, Some(body_id.index()));
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![
            StimulusType::EventHear,
            StimulusType::EventSeesShadow,
            StimulusType::EventSeesShadow,
            StimulusType::EventView,
            StimulusType::EventOutOfView,
            StimulusType::EventView,
            StimulusType::EventSeesBody,
            StimulusType::EventSeesObject,
        ],
        "StartThink must retain the complete HEAR then optical FIFO under AI lock"
    );
    assert!(matches!(
        ai.base.stimulus_queue[0].info,
        StimulusInfo::Noise(_)
    ));
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .filter_map(|stimulus| match stimulus.info {
                StimulusInfo::Human(target) => Some(target),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            first_visible_id.index(),
            lost_id.index(),
            last_visible_id.index(),
            body_id.index(),
        ]
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .last()
            .expect("Object event closes the retained detection FIFO")
            .info,
        StimulusInfo::Object(object_id.index()),
        "EVENT_SEES_OBJECT must retain an object payload, not impersonate a human"
    );
    assert_eq!(
        engine
            .get_entity(friend_id)
            .and_then(Entity::npc_data)
            .expect("locked observer's friend remains an NPC")
            .ai_state(),
        AiState::Default,
        "a retained VIEW must not leak through the later out-of-band ally alert"
    );
    assert!(
        !engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("locked detection observer remains an NPC")
            .alerted,
        "AILOCK_FREEZE must retain VIEW without pre-alerting its observer"
    );

    // Static RHArtificialIntelligence::mbFreeze is a separate mode: the
    // next RefreshDetection still scans and commits its latch, but StartThink
    // discards the resulting VIEW instead of retaining it.
    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("static-freeze detection observer exists")
    else {
        panic!("static-freeze detection observer changed kind")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("static-freeze detection observer retains enemy AI");
    ai.base.locks_flag_field = AiLockFlags::empty();
    ai.base.stimulus_queue.clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame = false;
    observer.npc.detectable_lists[DetectableType::Enemy as usize][0].shadow_seen_last_frame = true;

    engine.ai.global.freeze = true;
    crate::sim_rng::with_seed(0xA013_0B24, || engine.tick_enemy_ai(&assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("static-freeze detection observer remains an NPC");
    assert!(
        observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame,
        "static AI freeze must not suppress RefreshDetection latch commits"
    );
    let ai = observer
        .ai_brain
        .enemy()
        .expect("static-freeze detection observer retains enemy AI");
    assert!(
        ai.base.stimulus_queue.is_empty(),
        "static AI freeze must discard detection stimuli"
    );
    assert_eq!(ai.base.current_state, AiState::Default);
    assert!(!observer.alerted);
}

#[test]
fn inactive_building_viewer_runs_hearing_then_optics_while_outdoor_viewer_is_a_noop() {
    use crate::ai::{AiLockFlags, AiState, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    for observer_inside in [true, false] {
        let mut engine = EngineInner::new();
        let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
        let indoor_target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        let inactive_outdoor_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        let runner_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        let building = crate::position_interface::SectorHandle::new(42).unwrap();
        install_test_building_sector(&mut engine, 42);

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("inactive-viewer observer exists")
        else {
            panic!("inactive-viewer observer changed kind")
        };
        observer.element.active = true;
        observer
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        observer.element.set_position_map(MapPoint::new(0.0, 0.0));
        observer.element.set_direction_instantly(4);
        observer.npc.life_points = 100;
        observer.npc.view_direction = [1.0, 0.0];
        observer.npc.view_radius = 300;
        observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        observer.npc.eye_status = crate::element::EyeStatus::Stare;
        if observer_inside {
            observer.element.set_sector(Some(building));
        }

        let Entity::Pc(indoor_target) = engine
            .get_entity_mut(indoor_target_id)
            .expect("inactive same-building target exists")
        else {
            panic!("inactive same-building target changed kind")
        };
        indoor_target.element.active = true;
        indoor_target
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(40.0, 0.0, 0.0));
        indoor_target
            .element
            .set_position_map(MapPoint::new(40.0, 0.0));
        indoor_target.element.set_sector(Some(building));
        indoor_target.pc.life_points = 100;

        let Entity::Pc(inactive_outdoor) = engine
            .get_entity_mut(inactive_outdoor_id)
            .expect("inactive outdoor target exists")
        else {
            panic!("inactive outdoor target changed kind")
        };
        inactive_outdoor.element.active = true;
        inactive_outdoor
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(45.0, 0.0, 0.0));
        inactive_outdoor
            .element
            .set_position_map(MapPoint::new(45.0, 0.0));
        inactive_outdoor.pc.life_points = 100;

        let Entity::Pc(runner) = engine
            .get_entity_mut(runner_id)
            .expect("inactive-viewer runner exists")
        else {
            panic!("inactive-viewer runner changed kind")
        };
        runner.element.active = true;
        runner
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(55.0, 0.0, 0.0));
        runner.element.set_position_map(MapPoint::new(55.0, 0.0));
        runner.pc.life_points = 100;

        let mut movement = SequenceElement::new_movement(
            1,
            crate::element::Command::Move,
            Some(runner_id),
            OrderType::RunningUpright,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
        let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(movement_sequence, 0);

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        let Entity::Pc(indoor_target) = engine
            .get_entity_mut(indoor_target_id)
            .expect("same-building target exists after fixture")
        else {
            panic!("same-building target changed kind after fixture")
        };
        indoor_target.element.active = false;
        engine
            .get_entity_mut(inactive_outdoor_id)
            .expect("inactive outdoor target exists after fixture")
            .element_data_mut()
            .active = false;

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("inactive-viewer observer exists after fixture")
        else {
            panic!("inactive-viewer observer changed kind after fixture")
        };
        observer.element.active = false;
        let ai = observer
            .npc
            .ai_brain
            .enemy_mut()
            .expect("inactive-viewer observer has enemy AI");
        ai.base.me = observer_id.index();
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
        ai.current_task_priority = task_priority::NONE;
        ai.base.locks_flag_field = AiLockFlags::BUSY;
        observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![
            Detectable {
                element: Some(indoor_target_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
            Detectable {
                element: Some(inactive_outdoor_id),
                detectable_type: DetectableType::Enemy,
                seen_last_frame: true,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
            Detectable {
                element: Some(runner_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
        ];
        observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        observer.npc.maximal_detection_suspect = 777;

        crate::sim_rng::with_seed(0xA013_1A51, || engine.tick_enemy_ai(&assets));

        let observer = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("inactive-viewer observer remains an NPC");
        let ai = observer
            .ai_brain
            .enemy()
            .expect("inactive-viewer observer retains enemy AI");
        assert_eq!(ai.base.current_state, AiState::Default);

        if observer_inside {
            assert_eq!(
                ai.base
                    .stimulus_queue
                    .iter()
                    .map(|stimulus| stimulus.stimulus_type)
                    .collect::<Vec<_>>(),
                vec![
                    StimulusType::EventHear,
                    StimulusType::EventView,
                    StimulusType::EventOutOfView,
                ],
                "an inactive building viewer must retain acoustic-before-optical FIFO order"
            );
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
            assert!(
                !observer.detectable_lists[DetectableType::Enemy as usize][1].seen_last_frame,
                "a living inactive outdoor PC must stay in the list and produce a falling edge"
            );
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][2].heard_last_frame);
            assert_eq!(
                observer.detection_suspects[DetectableType::Enemy as usize],
                0
            );
            assert_eq!(observer.maximal_detection_suspect, 0);
            assert_eq!(
                engine
                    .get_entity(indoor_target_id)
                    .and_then(Entity::actor_data)
                    .expect("inactive indoor target remains an actor")
                    .last_noise_volume,
                0,
                "retaining an inactive PC for same-building sight must not make it audible"
            );
        } else {
            assert!(
                ai.base.stimulus_queue.is_empty(),
                "the first RefreshDetection gate must make an inactive outdoor viewer a no-op"
            );
            assert_eq!(
                observer.detectable_lists[DetectableType::Enemy as usize].len(),
                3,
                "living inactive targets remain Enemy detectables until they die"
            );
            assert!(!observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][1].seen_last_frame);
            assert!(!observer.detectable_lists[DetectableType::Enemy as usize][2].heard_last_frame);
            assert_eq!(
                observer.detection_suspects[DetectableType::Enemy as usize],
                999
            );
            assert_eq!(observer.maximal_detection_suspect, 777);
        }
    }
}

#[test]
fn inactive_npc_blip_detection_requires_door_or_building_eligibility() {
    use crate::element::{Camp, Entity};

    for observer_inside in [true, false] {
        let mut engine = EngineInner::new();
        let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
        let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        install_test_building_sector(&mut engine, 42);

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("blipped inactive observer exists")
        else {
            panic!("blipped inactive observer changed kind")
        };
        observer.element.active = true;
        observer.element.blipped = true;
        observer.npc.life_points = 100;
        observer
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        observer.element.set_position_map(MapPoint::new(0.0, 0.0));
        if observer_inside {
            observer.element.set_sector(Some(
                crate::position_interface::SectorHandle::new(42).unwrap(),
            ));
        }

        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("blip-viewing PC exists")
        else {
            panic!("blip-viewing PC changed kind")
        };
        pc.element.active = true;
        pc.pc.playable = true;
        pc.pc.life_points = 100;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(20.0, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(20.0, 0.0));

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        engine
            .get_entity_mut(observer_id)
            .expect("blipped observer exists after fixture")
            .element_data_mut()
            .active = false;

        crate::sim_rng::with_seed(0xA013_B11F, || engine.tick_enemy_ai(&assets));

        assert_eq!(
            engine
                .get_entity(observer_id)
                .expect("blipped observer survives tick")
                .element_data()
                .blipped,
            !observer_inside,
            "inactive building NPCs run blip detection; inactive outdoor NPCs do not"
        );
    }
}

#[test]
fn inactive_door_transit_viewer_runs_blip_and_hearing_then_skips_optics() {
    use crate::ai::{AiLockFlags, AiState, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};
    use crate::order::{Order, OrderType};
    use crate::position_interface::DoorHandle;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let runner_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("door-transit observer exists")
    else {
        panic!("door-transit observer changed kind")
    };
    observer.element.active = true;
    observer.element.blipped = true;
    observer.npc.life_points = 100;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    let Entity::Pc(runner) = engine
        .get_entity_mut(runner_id)
        .expect("door-transit runner exists")
    else {
        panic!("door-transit runner changed kind")
    };
    runner.element.active = true;
    runner.pc.playable = true;
    runner.pc.life_points = 100;
    runner
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(55.0, 0.0, 0.0));
    runner.element.set_position_map(MapPoint::new(55.0, 0.0));

    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(runner_id),
        OrderType::RunningUpright,
    );
    movement
        .orders
        .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("door-transit observer exists after fixture")
    else {
        panic!("door-transit observer changed kind after fixture")
    };
    observer.element.active = false;
    observer
        .element
        .sprite
        .position_iface
        .set_door_for_test(DoorHandle(0));
    observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
        element: Some(runner_id),
        detectable_type: DetectableType::Enemy,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    }];
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.maximal_detection_suspect = 777;
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("door-transit observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;
    ai.base.max_visibility = 0.75;

    crate::sim_rng::with_seed(0xA013_D00F, || engine.tick_enemy_ai(&assets));

    let Entity::Soldier(observer) = engine
        .get_entity(observer_id)
        .expect("door-transit observer survives tick")
    else {
        panic!("door-transit observer changed kind during tick")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy()
        .expect("door-transit observer retains enemy AI");
    assert!(
        !observer.element.blipped,
        "door transit passes the blip gate"
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![StimulusType::EventHear],
        "door transit passes acoustics but the sector-only optical gate rejects it"
    );
    assert!(observer.npc.detectable_lists[DetectableType::Enemy as usize][0].heard_last_frame);
    assert!(!observer.npc.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
    assert_eq!(
        observer.npc.detection_suspects[DetectableType::Enemy as usize],
        999,
        "the door-only optical return must not scan or decay Enemy suspects"
    );
    assert_eq!(observer.npc.maximal_detection_suspect, 0);
    assert_eq!(ai.base.max_visibility, 0.0);
}

#[test]
fn royalist_blip_auto_reveal_obeys_the_common_sixteen_frame_cadence() {
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist blip observer exists")
    else {
        panic!("Royalist blip observer changed kind")
    };
    observer.element.active = true;
    observer.element.blipped = true;
    observer.npc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.control.frame_counter = 1;
    engine.tick_enemy_ai(&assets);
    assert!(
        engine
            .get_entity(observer_id)
            .expect("Royalist blip observer survives closed cadence")
            .element_data()
            .blipped
    );

    engine.control.frame_counter = 16;
    engine.tick_enemy_ai(&assets);
    assert!(
        !engine
            .get_entity(observer_id)
            .expect("Royalist blip observer survives open cadence")
            .element_data()
            .blipped
    );
}

#[test]
fn retained_detection_view_rebuilds_the_live_enemy_scan_on_replay() {
    use crate::ai::{AiLockFlags, AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let rising_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let already_seen_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("queued replay observer exists")
    else {
        panic!("queued replay observer changed kind")
    };
    observer.element.active = true;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.life_points = 100;
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    for (id, x) in [(rising_id, 80.0), (already_seen_id, 120.0)] {
        let Entity::Pc(pc) = engine.get_entity_mut(id).expect("queued replay PC exists") else {
            panic!("queued replay PC changed kind")
        };
        pc.element.active = true;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(x, 0.0));
        pc.pc.life_points = 100;
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("queued replay observer exists after fixture")
    else {
        panic!("queued replay observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("queued replay observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;
    ai.list_them.clear();

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    for (target_id, seen_last_frame) in [(rising_id, false), (already_seen_id, true)] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            shadow_seen_last_frame: true,
            ..Detectable::default()
        });
    }

    crate::sim_rng::with_seed(0xA013_0B23, || engine.tick_enemy_ai(&assets));
    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("queued replay observer retains enemy AI");
    assert_eq!(ai.base.stimulus_queue.len(), 1);
    assert_eq!(ai.base.current_state, AiState::Default);

    engine
        .get_entity_mut(observer_id)
        .and_then(Entity::ai_controller_mut)
        .expect("queued replay observer retains controller")
        .locks_flag_field = AiLockFlags::empty();
    engine.tick_ai_queued_stimuli(&assets);

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("queued replay observer retains enemy AI after replay");
    assert!(ai.base.stimulus_queue.is_empty());
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(ai.base.primary_target, rising_id.index());
    assert_eq!(
        ai.list_them,
        vec![rising_id.index(), already_seen_id.index()],
        "retained VIEW replay must rebuild all currently latched enemies, not seed only its payload"
    );
    assert!(
        engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("queued replay observer remains an NPC after replay")
            .alerted,
        "accepted retained VIEW must set the persistent alert marker at dispatch time"
    );
}

#[test]
fn npc_out_of_view_precedes_same_slot_body_fifo() {
    use crate::ai::{AiState, Position, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));

    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let lost_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let body_id = engine.add_entity(make_test_pc(crate::element::Posture::Dead));

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("out-of-view soldier exists")
    else {
        panic!("out-of-view observer changed kind")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(4);
    soldier.npc.life_points = 100;
    soldier.npc.view_direction = [1.0, 0.0];
    soldier.npc.view_radius = 135;
    soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    soldier.npc.eye_status = crate::element::EyeStatus::Stare;
    let ai = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("out-of-view soldier has enemy AI");
    ai.base.me = soldier_id.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingObserve;
    ai.base.primary_target = lost_pc_id.index();
    ai.base.seek_position = Position {
        x: -200.0,
        y: 0.0,
        ..Position::default()
    };
    ai.current_task_priority = task_priority::ENEMY;

    let Entity::Pc(lost_pc) = engine.get_entity_mut(lost_pc_id).expect("lost PC exists") else {
        panic!("lost target changed kind")
    };
    lost_pc.element.active = true;
    lost_pc
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(-200.0, 0.0, 0.0));
    lost_pc.element.set_position_map(MapPoint::new(-200.0, 0.0));
    lost_pc.pc.life_points = 100;

    let Entity::Pc(body) = engine.get_entity_mut(body_id).expect("body PC exists") else {
        panic!("body target changed kind")
    };
    body.element.active = true;
    body.element
        .set_position(crate::coordinates::WorldPoint3D::new(80.0, 0.0, 0.0));
    body.element.set_position_map(MapPoint::new(80.0, 0.0));
    body.pc.life_points = 0;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the living PC profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("out-of-view soldier exists before detection")
    else {
        panic!("out-of-view soldier changed kind")
    };
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    soldier.npc.detectable_lists[DetectableType::Body as usize].clear();
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(lost_pc_id),
        detectable_type: DetectableType::Enemy,
        seen_last_frame: true,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });
    soldier.npc.detectable_lists[DetectableType::Body as usize].push(Detectable {
        element: Some(body_id),
        detectable_type: DetectableType::Body,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0A7, || engine.tick_enemy_ai(&assets));

    let soldier = engine
        .get_entity(soldier_id)
        .and_then(Entity::npc_data)
        .expect("out-of-view soldier remains an NPC");
    assert!(
        !soldier.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame,
        "lost enemy must clear its seen latch"
    );
    assert!(
        soldier.detectable_lists[DetectableType::Body as usize].is_empty(),
        "visible body must commit and leave its one-shot detectable list"
    );
    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("out-of-view soldier retains enemy AI");
    assert_eq!(
        ai.base.detected_body,
        body_id.index(),
        "OUTOFVIEW must enter Seeking before the later BODY stimulus is handled"
    );
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(ai.base.current_substate, Substate::SeekingBodyReactiontime);
}

#[test]
fn npc_detection_queues_every_rising_enemy_in_detectable_order() {
    use crate::ai::{AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    fn observe(far_first: bool) -> (Vec<u32>, Vec<bool>, Vec<u32>) {
        let mut engine = EngineInner::new();
        engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..ElementData::default()
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
        // A learned-through disguise remains in the Enemy bucket and must
        // emit ordinary EVENT_VIEW, never EVENT_SEES_BEGGAR.
        let far_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::SimulatingBeggar));
        let near_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("multi-view soldier exists")
        else {
            panic!("multi-view observer changed kind")
        };
        soldier.element.active = true;
        soldier
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
        soldier.element.set_direction_instantly(4);
        soldier.npc.life_points = 100;
        soldier.npc.view_direction = [1.0, 0.0];
        soldier.npc.view_radius = 300;
        soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        soldier.npc.eye_status = crate::element::EyeStatus::Stare;

        for (pc_id, x) in [(far_pc_id, 120.0), (near_pc_id, 80.0)] {
            let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("multi-view PC exists") else {
                panic!("multi-view target changed kind")
            };
            pc.element.active = true;
            pc.element
                .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
            pc.element.set_position_map(MapPoint::new(x, 0.0));
            pc.pc.life_points = 100;
        }

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("multi-view soldier exists before detection")
        else {
            panic!("multi-view observer changed kind")
        };
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("multi-view soldier has enemy AI");
        ai.base.me = soldier_id.index();
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
        ai.current_task_priority = task_priority::NONE;
        ai.base.got_the_beggar_trick = true;
        ai.list_them.clear();

        let ordered_targets = if far_first {
            [far_pc_id, near_pc_id]
        } else {
            [near_pc_id, far_pc_id]
        };
        let expected_order: Vec<u32> = ordered_targets.iter().map(|id| id.index()).collect();
        soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
        soldier.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        for target_id in ordered_targets {
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
                element: Some(target_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            });
        }

        crate::sim_rng::with_seed(0xA013_0B1E, || engine.tick_enemy_ai(&assets));

        let soldier = engine
            .get_entity(soldier_id)
            .and_then(Entity::npc_data)
            .expect("multi-view soldier remains an NPC");
        let latches = soldier.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| det.seen_last_frame)
            .collect();
        assert_eq!(
            soldier.follow_target,
            Some(ordered_targets[0]),
            "the first accepted VIEW must retain focus after the later FIFO entry"
        );
        let ai = engine
            .get_entity(soldier_id)
            .and_then(Entity::enemy_ai)
            .expect("multi-view soldier retains enemy AI");
        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert_eq!(ai.base.current_substate, Substate::AttackingReactiontime);
        assert_eq!(
            ai.base.primary_target, expected_order[0],
            "the first detectable's VIEW must win even when a later target is nearer"
        );
        assert_eq!(
            ai.base.last_stimulus_actor,
            Some(expected_order[1]),
            "the second VIEW must run through its own complete Think boundary"
        );
        (ai.list_them.clone(), latches, expected_order)
    }

    let (far_then_near, far_then_near_latches, far_then_near_expected) = observe(true);
    let (near_then_far, near_then_far_latches, near_then_far_expected) = observe(false);

    assert_eq!(far_then_near_latches, vec![true, true]);
    assert_eq!(near_then_far_latches, vec![true, true]);
    assert_eq!(far_then_near, far_then_near_expected);
    assert_eq!(near_then_far, near_then_far_expected);
}

#[test]
fn npc_detection_view_rebinds_combat_data_to_the_queued_target() {
    use crate::ai::{AiState, Decision, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let old_target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let viewed_target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("target-rebind soldier exists")
    else {
        panic!("target-rebind observer changed kind")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(4);
    soldier.npc.life_points = 100;
    soldier.npc.view_direction = [1.0, 0.0];
    soldier.npc.view_radius = 300;
    soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    soldier.npc.eye_status = crate::element::EyeStatus::Stare;

    for (pc_id, x) in [(old_target_id, -200.0), (viewed_target_id, 40.0)] {
        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("target-rebind PC exists")
        else {
            panic!("target-rebind target changed kind")
        };
        pc.element.active = true;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(x, 0.0));
        pc.pc.life_points = 100;
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the target-rebind PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("target-rebind soldier exists before detection")
    else {
        panic!("target-rebind observer changed kind")
    };
    let ai = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("target-rebind soldier has enemy AI");
    ai.base.me = soldier_id.index();
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingJustWatching;
    ai.current_task_priority = task_priority::SEEKING;
    ai.base.primary_target = old_target_id.index();
    ai.base.seek_position = crate::ai::Position {
        x: -200.0,
        y: 0.0,
        ..crate::ai::Position::default()
    };
    ai.forced_next_battle_decision = Decision::Fight;

    soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    soldier.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(viewed_target_id),
        detectable_type: DetectableType::Enemy,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0B1F, || engine.tick_enemy_ai(&assets));

    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("target-rebind soldier retains enemy AI");
    assert_eq!(
        (
            ai.base.primary_target,
            ai.base.last_stimulus_actor,
            ai.base.current_state,
            ai.base.current_substate,
            ai.forced_next_battle_decision,
        ),
        (
            viewed_target_id.index(),
            Some(viewed_target_id.index()),
            AiState::Attacking,
            Substate::AttackingSwordfight,
            Decision::None,
        )
    );
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
}

#[test]
fn royalist_detection_think_opens_a_later_royalists_same_frame_view() {
    use crate::ai::{AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    fn observe(source_before_listener: bool) -> (bool, AiState, bool) {
        let mut engine = EngineInner::new();
        engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..ElementData::default()
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let source = make_test_ai_soldier(Camp::Royalists);
        let listener = make_test_ai_soldier(Camp::Royalists);
        let (source_id, listener_id) = if source_before_listener {
            (engine.add_entity(source), engine.add_entity(listener))
        } else {
            let listener_id = engine.add_entity(listener);
            let source_id = engine.add_entity(source);
            (source_id, listener_id)
        };
        let target_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

        for (id, x) in [(source_id, 0.0), (listener_id, 20.0), (target_id, 80.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("Royalist ordering soldier exists")
            else {
                panic!("Royalist ordering actor changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.npc.life_points = 100;
            soldier.npc.view_radius = 200;
            soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
            if id == target_id {
                soldier.element.blipped = true;
            }
        }

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);

        for id in [source_id, listener_id] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("Royalist observer exists after fixture")
            else {
                panic!("Royalist observer changed kind after fixture")
            };
            let ai = soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("Royalist observer has enemy AI");
            ai.base.me = id.index();
            ai.base.current_state = AiState::Default;
            ai.base.current_substate = Substate::DefaultOnPost;
            ai.base.current_music_alert_status = crate::ai::AlertLevel::Green;
            ai.current_task_priority = task_priority::NONE;
            ai.base.primary_target = 0;
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
                element: Some(target_id),
                detectable_type: DetectableType::Enemy,
                ..Detectable::default()
            });
        }

        let Entity::Soldier(source) = engine
            .get_entity_mut(source_id)
            .expect("source Royalist exists")
        else {
            panic!("source Royalist changed kind")
        };
        source.element.set_direction_instantly(4);
        source.npc.view_direction = [1.0, 0.0];
        source.npc.eye_status = crate::element::EyeStatus::Stare;

        let Entity::Soldier(listener) = engine
            .get_entity_mut(listener_id)
            .expect("listener Royalist exists")
        else {
            panic!("listener Royalist changed kind")
        };
        listener.element.set_direction_instantly(4);
        listener.npc.view_direction = [1.0, 0.0];
        listener.npc.eye_status = crate::element::EyeStatus::LookForward;

        engine.control.frame_counter = 7;
        assert!(
            !(engine.control.frame_counter + listener_id.index())
                .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC),
            "listener fixture must start on a closed Royalist NPC detection gate"
        );
        crate::sim_rng::with_seed(0xA013_0B20, || engine.tick_enemy_ai(&assets));
        assert!(
            !engine
                .get_entity(target_id)
                .expect("Royalist target remains present")
                .element_data()
                .blipped,
            "Royalist HandleDetection must reveal its blipped NPC target at the detecting slot"
        );

        let source = engine
            .get_entity(source_id)
            .and_then(Entity::npc_data)
            .expect("source Royalist remains an NPC");
        let source_latch = source.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|det| det.element == Some(target_id))
            .expect("source retains target detectable")
            .seen_last_frame;
        let source_ai = engine
            .get_entity(source_id)
            .and_then(Entity::enemy_ai)
            .expect("source Royalist retains enemy AI");
        assert_eq!(
            (source_latch, source_ai.base.current_state),
            (true, AiState::Attacking),
            "source Royalist must detect before its alert can test creation ordering"
        );

        let listener = engine
            .get_entity(listener_id)
            .and_then(Entity::npc_data)
            .expect("listener Royalist remains an NPC");
        let latch = listener.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|det| det.element == Some(target_id))
            .expect("listener retains target detectable")
            .seen_last_frame;
        let ai = engine
            .get_entity(listener_id)
            .and_then(Entity::enemy_ai)
            .expect("listener Royalist retains enemy AI");
        (
            latch,
            ai.base.current_state,
            ai.base.primary_target == target_id.index(),
        )
    }

    let source_first = observe(true);
    assert_eq!(
        source_first,
        (true, AiState::Attacking, true),
        "the first Royalist's synchronous VIEW alert must turn and open detection for the later slot"
    );

    let listener_first = observe(false);
    assert_eq!(
        listener_first,
        (false, AiState::Wondering, false),
        "an earlier listener slot must not retroactively rescan after the later source alerts it"
    );
}

#[test]
fn royalist_detection_retains_every_ordered_view_edge_while_ai_locked() {
    use crate::ai::{AiLockFlags, AiState, StimulusInfo, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let first_visible_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let lost_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let last_visible_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    for (id, x) in [
        (observer_id, 0.0),
        (first_visible_id, 80.0),
        (lost_id, 100.0),
        (last_visible_id, 120.0),
    ] {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(id)
            .expect("Royalist multi-edge soldier exists")
        else {
            panic!("Royalist multi-edge actor changed kind")
        };
        soldier.element.active = true;
        soldier
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.npc.life_points = 100;
        soldier.element.blipped = id != observer_id;
    }

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist multi-edge observer exists")
    else {
        panic!("Royalist multi-edge observer changed kind")
    };
    observer.element.set_direction_instantly(4);
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(lost) = engine
        .get_entity_mut(lost_id)
        .expect("lost Royalist target exists after fixture")
    else {
        panic!("lost Royalist target changed kind after fixture")
    };
    // Original CleanUpDetectables removes dead enemies, not inactive living
    // ones. An inactive outdoor target remains in the list and emits the
    // falling OUTOFVIEW edge.
    lost.element.active = false;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist multi-edge observer exists after fixture")
    else {
        panic!("Royalist multi-edge observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("Royalist multi-edge observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.base.current_music_alert_status = crate::ai::AlertLevel::Green;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    for (target_id, seen_last_frame) in [
        (first_visible_id, false),
        (lost_id, true),
        (last_visible_id, false),
    ] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            ..Detectable::default()
        });
    }

    crate::sim_rng::with_seed(0xA013_0B21, || engine.tick_enemy_ai(&assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("Royalist multi-edge observer remains an NPC");
    assert_eq!(
        observer.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| det.seen_last_frame)
            .collect::<Vec<_>>(),
        vec![true, false, true],
        "HandleDetection must settle every Royalist Enemy latch before Think"
    );
    assert!(
        !engine
            .get_entity(first_visible_id)
            .expect("first visible Royalist target remains present")
            .element_data()
            .blipped
            && !engine
                .get_entity(last_visible_id)
                .expect("last visible Royalist target remains present")
                .element_data()
                .blipped,
        "every rising Royalist Enemy edge must reveal its target before Think"
    );
    assert!(
        engine
            .get_entity(lost_id)
            .expect("lost Royalist target remains present")
            .element_data()
            .blipped,
        "a falling Royalist Enemy edge must not reveal its target"
    );
    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("Royalist multi-edge observer retains enemy AI");
    assert!(ai.base.pending_stimuli.is_empty());
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.last_stimulus_actor, Some(last_visible_id.index()));
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("Royalist Enemy edge lost its human payload")
                };
                (stimulus.stimulus_type, target)
            })
            .collect::<Vec<_>>(),
        vec![
            (StimulusType::EventView, first_visible_id.index()),
            (StimulusType::EventOutOfView, lost_id.index()),
            (StimulusType::EventView, last_visible_id.index()),
        ],
        "Royalist HandleDetection must retain interleaved edges in detectable-list order"
    );
}

#[test]
fn npc_follow_observes_target_position_at_its_creation_order_boundary() {
    #[derive(Debug, PartialEq)]
    struct Observation {
        frame: u32,
        observer_slot: u32,
        target_slot: u32,
        target_before_movement: MapPoint,
        target_after_movement: MapPoint,
        target_position_observed_by_follow: MapPoint,
    }

    fn observe(observer_before_target: bool) -> Observation {
        use crate::element::{Camp, Entity, Posture};

        let mut engine = EngineInner::new();
        engine.control.frame_counter = 73;

        let mut observer = make_test_ai_soldier(Camp::Lacklandists);
        observer.element_data_mut().active = true;
        observer
            .element_data_mut()
            .set_position_map(MapPoint::new(100.0, 100.0));
        observer.element_data_mut().set_direction_instantly(0);

        let mut target = make_test_pc(Posture::Upright);
        target.element_data_mut().active = true;
        let target_before_movement = MapPoint::new(80.0, 20.0);
        let target_after_movement = MapPoint::new(120.0, 20.0);
        target
            .element_data_mut()
            .set_position_map(target_before_movement);

        let (observer_id, target_id) = if observer_before_target {
            let observer_id = engine.add_entity(observer);
            let target_id = engine.add_entity(target);
            (observer_id, target_id)
        } else {
            let target_id = engine.add_entity(target);
            let observer_id = engine.add_entity(observer);
            (observer_id, target_id)
        };

        let mut positions_before_movement =
            crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (entity_id, entity) in engine.world.entities.occupied() {
            positions_before_movement[entity_id] = Some(entity.element_data().position_map());
        }

        // This mutation is the smallest deterministic stand-in for the
        // globally batched tick_entity_movement between the captured input
        // boundary and refresh_npc_views. The oracle is the position copied
        // into EYES_FOLLOW's stare point, not movement-distance mechanics.
        engine
            .get_entity_mut(target_id)
            .expect("follow target exists")
            .element_data_mut()
            .set_position_map(target_after_movement);
        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("follow observer exists")
        else {
            panic!("follow observer changed entity kind");
        };
        crate::ai_vision::focus_entity(&mut observer.npc, target_id);

        engine.refresh_npc_views(&positions_before_movement);

        let Entity::Soldier(observer) = engine
            .get_entity(observer_id)
            .expect("follow observer remains")
        else {
            panic!("follow observer changed entity kind");
        };
        Observation {
            frame: engine.control.frame_counter,
            observer_slot: observer_id.index(),
            target_slot: target_id.index(),
            target_before_movement,
            target_after_movement,
            target_position_observed_by_follow: observer.npc.stare_point,
        }
    }

    assert_eq!(
        [observe(true), observe(false)],
        [
            Observation {
                frame: 73,
                observer_slot: 0,
                target_slot: 1,
                target_before_movement: MapPoint::new(80.0, 20.0),
                target_after_movement: MapPoint::new(120.0, 20.0),
                target_position_observed_by_follow: MapPoint::new(80.0, 20.0),
            },
            Observation {
                frame: 73,
                observer_slot: 1,
                target_slot: 0,
                target_before_movement: MapPoint::new(80.0, 20.0),
                target_after_movement: MapPoint::new(120.0, 20.0),
                target_position_observed_by_follow: MapPoint::new(120.0, 20.0),
            },
        ],
        "original per-element virtual calls expose pre-move state to an earlier observer and post-move state to a later observer"
    );
}

#[test]
fn seek_tolerance_observes_target_position_at_its_creation_order_boundary() {
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::position_interface::SectorHandle;
    use crate::sequence::{MoveFlags, SequenceElement, SequenceElementData, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    #[derive(Debug, PartialEq)]
    struct Observation {
        seeker_slot: u32,
        target_slot: u32,
        target_before_movement: MapPoint,
        target_after_movement: MapPoint,
        seeker_state: SequenceState,
    }

    fn bind_walking_sprite(engine: &mut EngineInner, entity_id: EntityId) {
        let action = OrderType::WalkingUpright;
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 20.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 20,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![20],
            offsets: vec![SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;
        let mut sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );

        let element = engine
            .get_entity_mut(entity_id)
            .expect("movement fixture actor exists")
            .element_data_mut();
        let position = element.position_map();
        let sector = element.sector();
        sprite.position_iface.set_sector(sector);
        sprite.position_iface.set_anti_collision_on(false);
        sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                MapVec::new(-2.0, -2.0),
                MapVec::new(2.0, 2.0),
            ));
        element.sprite = sprite;
        element.set_position_map(position);
    }

    fn arm_movement(
        engine: &mut EngineInner,
        owner: EntityId,
        destination: MapPoint,
        seek_target: Option<EntityId>,
    ) -> crate::sequence::SequenceId {
        let mut element =
            SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
        element.orders.push_back(Order::test_new(
            OrderType::WalkingUpright,
            destination.x,
            destination.y,
        ));
        let SequenceElementData::Movement {
            destination: element_destination,
            sector,
            element: element_target,
            flags,
            tolerance,
            ..
        } = &mut element.data
        else {
            unreachable!("new_movement must create movement data")
        };
        *element_destination = destination;
        *sector = SectorHandle::new(1);
        *element_target = seek_target;
        if seek_target.is_some() {
            *flags = MoveFlags::SEEK;
            *tolerance = 15.0;
        }

        let sequence_id = engine.orders.sequence_manager.launch_element(element);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
        let actor = engine
            .get_entity_mut(owner)
            .expect("movement owner exists")
            .actor_data_mut()
            .expect("movement owner is an actor");
        actor.action_state = ActionState::Moving;
        actor.active_movement = ActiveMovement::new(sequence_id, 0);
        sequence_id
    }

    fn observe(seeker_before_target: bool) -> Observation {
        let mut engine = EngineInner::new();
        let target_before_movement = MapPoint::new(10.0, 0.0);
        let target_destination = MapPoint::new(30.0, 0.0);

        let mut seeker = make_test_pc(Posture::Upright);
        seeker.element_data_mut().active = true;
        seeker
            .element_data_mut()
            .set_position_map(MapPoint::new(0.0, 0.0));
        seeker.element_data_mut().set_sector(SectorHandle::new(1));

        let mut target = make_test_pc(Posture::Upright);
        target.element_data_mut().active = true;
        target
            .element_data_mut()
            .set_position_map(target_before_movement);
        target.element_data_mut().set_sector(SectorHandle::new(1));

        let (seeker_id, target_id) = if seeker_before_target {
            let seeker_id = engine.add_entity(seeker);
            let target_id = engine.add_entity(target);
            (seeker_id, target_id)
        } else {
            let target_id = engine.add_entity(target);
            let seeker_id = engine.add_entity(seeker);
            (seeker_id, target_id)
        };

        bind_walking_sprite(&mut engine, seeker_id);
        bind_walking_sprite(&mut engine, target_id);
        arm_movement(&mut engine, target_id, target_destination, None);
        let seeker_sequence = arm_movement(
            &mut engine,
            seeker_id,
            MapPoint::new(100.0, 0.0),
            Some(target_id),
        );

        // The original sprite pipeline reports MotionState::Start without
        // advancing on a newly-seen order. Prime that start tick, then use
        // the next production movement tick as the ordering observation.
        let assets = LevelAssets::new();
        engine.tick_entity_movement(&assets);
        engine.tick_entity_movement(&assets);

        Observation {
            seeker_slot: seeker_id.index(),
            target_slot: target_id.index(),
            target_before_movement,
            target_after_movement: engine
                .get_entity(target_id)
                .expect("target remains after movement")
                .element_data()
                .position_map(),
            seeker_state: engine
                .orders
                .sequence_manager
                .get_element(seeker_sequence, 0)
                .expect("seeker movement element remains inspectable")
                .state,
        }
    }

    assert_eq!(
        [observe(true), observe(false)],
        [
            Observation {
                seeker_slot: 0,
                target_slot: 1,
                target_before_movement: MapPoint::new(10.0, 0.0),
                // The target turns one sector toward +X on this frame, so
                // the original 20-unit frame distance receives the 0.6 turn
                // slowdown and commits a 12-unit step.
                target_after_movement: MapPoint::new(22.0, 0.0),
                seeker_state: SequenceState::Terminated,
            },
            Observation {
                seeker_slot: 1,
                target_slot: 0,
                target_before_movement: MapPoint::new(10.0, 0.0),
                target_after_movement: MapPoint::new(22.0, 0.0),
                seeker_state: SequenceState::InProgress,
            },
        ],
        "a seeker before its target observes the pre-move position, while a seeker after its target observes the committed post-move position"
    );
}

#[test]
fn final_arrival_step_runs_actor_anti_collision_before_snapping() {
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::position_interface::SectorHandle;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    fn bind_walking_sprite(engine: &mut EngineInner, entity_id: EntityId) {
        let action = OrderType::WalkingUpright;
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 20.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 20,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![20],
            offsets: vec![SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;
        let mut sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );

        let element = engine
            .get_entity_mut(entity_id)
            .expect("anti-collision fixture actor exists")
            .element_data_mut();
        let position = element.position_map();
        let sector = element.sector();
        sprite.position_iface.set_sector(sector);
        sprite.position_iface.set_anti_collision_on(true);
        sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                MapVec::new(-2.0, -2.0),
                MapVec::new(2.0, 2.0),
            ));
        element.sprite = sprite;
        element.set_position_map(position);
    }

    let mut engine = EngineInner::new();
    let destination = MapPoint::new(10.0, 0.0);

    let mut mover = make_test_pc(Posture::Upright);
    mover.element_data_mut().active = true;
    mover
        .element_data_mut()
        .set_position_map(MapPoint::new(0.0, 0.0));
    mover.element_data_mut().set_sector(SectorHandle::new(1));

    let mut blocker = make_test_pc(Posture::Upright);
    blocker.element_data_mut().active = true;
    blocker.element_data_mut().set_position_map(destination);
    blocker.element_data_mut().set_sector(SectorHandle::new(1));

    let mover_id = engine.add_entity(mover);
    let blocker_id = engine.add_entity(blocker);
    bind_walking_sprite(&mut engine, mover_id);
    bind_walking_sprite(&mut engine, blocker_id);

    let mut movement =
        SequenceElement::new_movement(1, Command::Move, Some(mover_id), OrderType::WalkingUpright);
    movement.orders.push_back(Order::test_new(
        OrderType::WalkingUpright,
        destination.x,
        destination.y,
    ));
    let SequenceElementData::Movement {
        destination: movement_destination,
        sector,
        ..
    } = &mut movement.data
    else {
        unreachable!("new_movement must create movement data")
    };
    *movement_destination = destination;
    *sector = SectorHandle::new(1);

    let sequence_id = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    let actor = engine
        .get_entity_mut(mover_id)
        .expect("mover exists")
        .actor_data_mut()
        .expect("mover is an actor");
    actor.action_state = ActionState::Moving;
    actor.active_movement = ActiveMovement::new(sequence_id, 0);

    // A newly-seen motion order spends one tick in MotionState::Start.
    // On the next tick the destination is within one animation step. The
    // original game still applies actor repulsion before checking arrival.
    let assets = LevelAssets::new();
    engine.tick_entity_movement(&assets);
    engine.tick_entity_movement(&assets);

    let mover_position = engine
        .get_entity(mover_id)
        .expect("mover remains after movement")
        .element_data()
        .position_map();
    let blocker_position = engine
        .get_entity(blocker_id)
        .expect("blocker remains after movement")
        .element_data()
        .position_map();
    assert_ne!(
        mover_position, blocker_position,
        "the final movement tick must not snap the mover onto another actor"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("movement remains inspectable")
            .state,
        SequenceState::InProgress,
        "a deflected final step must reconsider arrival on a later tick"
    );
}

#[test]
fn npc_hourglass_uses_exact_wrapped_register_frame_phase() {
    use super::ai::npc_hourglass_frame_phase;

    let sixteenth_frame_visits: Vec<_> = (0..256)
        .filter_map(|frame| {
            let phase = npc_hourglass_frame_phase(frame, 0);
            (phase & 15 == 0).then_some((frame, phase))
        })
        .collect();
    assert_eq!(
        sixteenth_frame_visits,
        vec![
            (4, 160),
            (20, 176),
            (36, 192),
            (52, 208),
            (68, 224),
            (84, 240),
            (100, 0),
            (116, 16),
            (132, 32),
            (148, 48),
            (164, 64),
            (180, 80),
            (196, 96),
            (212, 112),
            (228, 128),
            (244, 144),
        ]
    );
    assert_eq!(
        sixteenth_frame_visits
            .iter()
            .filter_map(|&(frame, phase)| (phase & 63 == 0).then_some(frame))
            .collect::<Vec<_>>(),
        vec![36, 100, 164, 228]
    );
}

#[test]
fn npc_hourglass_tail_drains_old_lock_queue_only_after_unlock() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let soldier_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let ai = engine
        .get_entity_mut(soldier_id)
        .and_then(|entity| entity.ai_controller_mut())
        .expect("test soldier has AI");
    ai.locks_flag_field = crate::ai::AiLockFlags::BUSY;
    ai.stimulus_queue.push(crate::ai::Stimulus::new(
        crate::ai::StimulusType::EventAfterCombatInjury,
    ));

    engine.tick_ai_queued_stimuli(&assets);
    assert_eq!(
        engine
            .get_entity(soldier_id)
            .and_then(|entity| entity.ai_controller())
            .unwrap()
            .stimulus_queue
            .len(),
        1,
        "the Hourglass lock gate must preserve queued stimuli"
    );

    engine
        .get_entity_mut(soldier_id)
        .and_then(|entity| entity.ai_controller_mut())
        .unwrap()
        .locks_flag_field = crate::ai::AiLockFlags::empty();
    engine.tick_ai_queued_stimuli(&assets);
    assert!(
        engine
            .get_entity(soldier_id)
            .and_then(|entity| entity.ai_controller())
            .unwrap()
            .stimulus_queue
            .is_empty(),
        "the final unlocked Hourglass phase must replay the old lock queue"
    );
}

fn pending_specific_blinks(engine: &EngineInner, npc_id: EntityId) -> Vec<EntityId> {
    engine
        .get_entity(npc_id)
        .and_then(|entity| entity.ai_controller())
        .map(|ai| ai.pending_blink_enemy_specific.clone())
        .expect("NPC has AI controller")
}

#[test]
fn deferred_wakeup_pc_queues_specific_blink_for_opposite_camp_npcs() {
    use crate::combat::ConcussionOutcome;
    use crate::element::{Camp, Posture};

    let mut engine = EngineInner::new();
    let waker = engine.add_entity(make_test_pc(Posture::Upright));
    let same_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let opposite_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    engine
        .orders
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(&LevelAssets::new());

    assert_eq!(
        pending_specific_blinks(&engine, same_camp_npc),
        Vec::<EntityId>::new()
    );
    assert_eq!(
        pending_specific_blinks(&engine, opposite_camp_npc),
        vec![waker]
    );
}

#[test]
fn deferred_wakeup_soldier_queues_specific_blink_for_opposite_camp_npcs() {
    use crate::combat::ConcussionOutcome;
    use crate::element::Camp;

    let mut engine = EngineInner::new();
    engine.ai.global.there_are_royalist_soldiers = true;
    engine.ai.global.there_are_lacklandist_soldiers = true;
    let waker = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let same_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let opposite_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    engine
        .orders
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(&LevelAssets::new());

    assert_eq!(
        pending_specific_blinks(&engine, waker),
        Vec::<EntityId>::new()
    );
    assert_eq!(
        pending_specific_blinks(&engine, same_camp_npc),
        Vec::<EntityId>::new()
    );
    assert_eq!(
        pending_specific_blinks(&engine, opposite_camp_npc),
        vec![waker]
    );
}

#[test]
fn deferred_wakeup_soldier_skips_blink_when_npcs_cannot_be_enemies() {
    use crate::combat::ConcussionOutcome;
    use crate::element::Camp;

    let mut engine = EngineInner::new();
    let waker = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let opposite_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    engine
        .orders
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(&LevelAssets::new());

    assert_eq!(
        pending_specific_blinks(&engine, opposite_camp_npc),
        Vec::<EntityId>::new()
    );
}

fn bind_test_action_point(
    engine: &mut EngineInner,
    id: EntityId,
    action: crate::order::OrderType,
    hotspot: crate::coordinates::SpriteLocalPoint,
    center: crate::coordinates::SpriteAnchor,
) {
    let script = crate::sprite_script::SpriteScript {
        action_id: action as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot,
        sum_distance: 0,
        frame_ids: vec![1],
        delays: vec![1],
        distances: vec![0],
        offsets: vec![SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    conversion[action as usize] = 0;
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );
    sprite.center = center;
    let element = engine.get_entity_mut(id).unwrap().element_data_mut();
    let position = element.position_map();
    let direction = element.direction();
    element.sprite = sprite;
    element.set_position_map(position);
    element.set_direction_instantly(direction);
}

fn bind_test_bow_release_action(engine: &mut EngineInner, id: EntityId) {
    let action = crate::order::OrderType::ShootingWithBow;
    let script = crate::sprite_script::SpriteScript {
        action_id: action as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::new(2.0, 3.0),
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0, 0, 0],
    };
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    conversion[action as usize] = 0;
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    let element = engine.get_entity_mut(id).unwrap().element_data_mut();
    let position = element.position_map();
    let direction = element.direction();
    sprite.center = crate::coordinates::SpriteAnchor::ZERO;
    element.sprite = sprite;
    element.set_position_map(position);
    element.set_direction_instantly(direction);
}

#[test]
fn parry_sword_queues_transition_and_hold_orders() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::WaitingSword;

    let seq_id =
        engine
            .orders
            .sequence_manager
            .launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::ParrySword,
                Some(soldier),
            ));
    engine.dispatch_parry_sword(soldier, false, seq_id, 0);

    let elem = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("parry element should remain live");
    assert_eq!(elem.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        elem.orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![
            OrderType::TransitionWaitingSwordParryingSword,
            OrderType::ParryingSword,
        ]
    );
}

#[test]
fn stop_parry_sword_queues_exit_transition() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::ParryingSword;

    let seq_id =
        engine
            .orders
            .sequence_manager
            .launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::StopParrySword,
                Some(soldier),
            ));
    engine.dispatch_stop_parry(soldier, seq_id, 0);

    let elem = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("stop-parry element should remain live");
    assert_eq!(elem.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        elem.orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![OrderType::TransitionParryingSwordWaitingSword]
    );
}

/// A LeaningOut soldier that receives a command requiring Upright
/// (e.g. `Move`) must snap to Upright and queue the
/// `TransitionLeaningOutWaitingAlerted` animation so the lean-out-
/// window unstick transition plays.
#[test]
fn soldier_leaning_out_to_upright_on_move() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::LeaningOut));

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::Move);
    assert!(changed, "auto-leave should fire for LeaningOut + Move");

    let entity = engine.get_entity(soldier_id).expect("soldier present");
    assert_eq!(
        entity.element_data().posture,
        Posture::Upright,
        "posture should snap to Upright"
    );

    let next_order = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        next_order,
        Some(OrderType::TransitionLeaningOutWaitingAlerted),
        "lean-out transition animation should be queued"
    );
}

/// An Upright soldier invoked with a posture-neutral command should
/// not be touched by `auto_leave_disguise_if_needed`.
#[test]
fn soldier_upright_move_skips_auto_leave() {
    use crate::element::{Command, Posture};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::Move);
    assert!(!changed, "no transition needed for an Upright soldier");

    let entity = engine.get_entity(soldier_id).expect("soldier present");
    assert_eq!(entity.element_data().posture, Posture::Upright);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(soldier_id)
            .is_none(),
        "no animation should be queued"
    );
}

/// An attentive-mode transition on an idle soldier queues
/// `TransitionWaitingUprightWaitingAlerted` as an order on the
/// sequence element.
#[test]
fn soldier_enter_attentive_mode_queues_transition_anim() {
    let mut display = HostDisplayState::default();
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    // Launch the EnterAttentiveMode element first; `ensure_wait_element`
    // is a no-op once another live element exists for the actor.  This
    // matches level-load ordering: spawn → (maybe scripted elements) →
    // ensure_wait_element covers only the actors left idle.
    // Stamp `posture_after_transition = Upright` at launch.
    let mut elem = SequenceElement::new(1, Command::EnterAttentiveMode, Some(soldier_id));
    elem.posture_after_transition = Posture::Upright;
    engine.launch_element(elem);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let active = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        active,
        Some(OrderType::TransitionWaitingUprightWaitingAlerted),
        "the transition order should be the front of the actor's current element"
    );
}

/// Regression: calling `set_soldier_attentive_mode` on an Upright
/// soldier (the path real game code hits via `pending_set_attentive_mode`
/// when an enemy spots the PC) must queue the alerted-transition
/// animation.  The previous bug left
/// `SequenceElement::posture_after_transition` at `Posture::Undefined`
/// because only `ensure_wait_element` and `auto_leave_disguise_if_needed`
/// stamped it; `arbitrate_instruct` now stamps it unconditionally
/// (`set_posture_after_transition(get_posture())`).
#[test]
fn set_soldier_attentive_mode_plays_transition_from_upright() {
    let mut display = HostDisplayState::default();
    use crate::element::Posture;
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    // Drive the engine-side helper the way the AI does it — no explicit
    // posture stamping; arbitrate_instruct must supply it.  Launch the
    // attentive element before `ensure_wait_element` so the latter
    // no-ops (matching the AI drain ordering in `tick_enemy_ai` where
    // `set_soldier_attentive_mode` fires from the per-NPC pending drain
    // and only actors left idle get a Wait element).
    engine.set_soldier_attentive_mode(soldier_id, true, false);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let active = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        active,
        Some(OrderType::TransitionWaitingUprightWaitingAlerted),
        "transition-to-alerted animation should be the actor's current order"
    );
}

#[test]
fn arbitration_postpone_current_splits_when_current_cannot_interrupt_now() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut current = SequenceElement::new(1, Command::Move, Some(owner));
    current.priority = SequencePriority::Normal;
    current
        .orders
        .push_back(Order::test_new(OrderType::WalkingUpright, 10.0, 0.0));
    current
        .orders
        .push_back(Order::test_new(OrderType::WalkingUpright, 20.0, 0.0));
    current.orders.front_mut().unwrap().lock_ai = true;
    let current_seq = engine.orders.sequence_manager.launch_element(current);
    engine
        .orders
        .sequence_manager
        .element_in_progress(current_seq, 0);

    let mut incoming = SequenceElement::new(1, Command::Turn, Some(owner));
    incoming.priority = SequencePriority::Preference;
    let incoming_seq = engine.orders.sequence_manager.launch_element(incoming);

    let accepted = engine.arbitrate_instruct(incoming_seq, 0);
    assert!(
        !accepted,
        "locked current order should finish before incoming element dispatches"
    );

    let current = engine
        .orders
        .sequence_manager
        .get_element(current_seq, 0)
        .unwrap();
    assert_eq!(current.orders.len(), 1);
    assert_eq!(current.cross_postponed, Some((incoming_seq, 0)));

    let incoming = engine
        .orders
        .sequence_manager
        .get_element(incoming_seq, 0)
        .unwrap();
    assert_eq!(incoming.state, SequenceState::Postponed);
    assert!(
        incoming.cross_postponed.is_some(),
        "incoming should resume the current continuation after it runs"
    );
}

#[test]
fn pc_shoot_bow_queues_behind_live_bow_animation_order() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let pc = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut current =
        SequenceElement::new_interaction(1, Command::ShootBow, Some(pc), Some(target));
    current.priority = SequencePriority::Preference;
    current
        .orders
        .push_back(Order::test_new(OrderType::ShootingWithBow, 0.0, 0.0));
    let current_seq = engine.orders.sequence_manager.launch_element(current);
    engine
        .orders
        .sequence_manager
        .element_in_progress(current_seq, 0);
    engine
        .get_entity_mut(pc)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .old_action = OrderType::WaitingUpright;

    let incoming = SequenceElement::new_interaction(1, Command::ShootBow, Some(pc), Some(target));
    let incoming_seq = engine.launch_element_for_owner(incoming);

    let current = engine
        .orders
        .sequence_manager
        .get_element(current_seq, 0)
        .unwrap();
    assert_eq!(current.cross_postponed, Some((incoming_seq, 0)));

    let incoming = engine
        .orders
        .sequence_manager
        .get_element(incoming_seq, 0)
        .unwrap();
    assert_eq!(incoming.state, SequenceState::Postponed);
}

#[test]
fn started_pass_door_rejects_new_move() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut current_pass =
        SequenceElement::new_movement(1, Command::PassDoor, Some(owner), OrderType::WalkingUpright);
    current_pass.priority = SequencePriority::NonInterruptable;
    let pass_seq = engine.orders.sequence_manager.launch_element(current_pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(pass_seq, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sequence_element_started = true;

    let incoming =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let incoming_seq = engine.launch_element_for_owner(incoming);

    let pass = engine
        .orders
        .sequence_manager
        .get_element(pass_seq, 0)
        .unwrap();
    assert_eq!(pass.state, SequenceState::InProgress);

    let incoming = engine
        .orders
        .sequence_manager
        .get_element(incoming_seq, 0)
        .unwrap();
    assert_eq!(incoming.state, SequenceState::Impossible);
}

/// A Crouched soldier receiving `ENTER_ATTENTIVE_MODE` must first
/// auto-stand (CROUCH_UP) before the alerted transition can play,
/// because `get_transition_flags_soldier` for this command sets
/// `CHANGEPOSTURE_MUST_BE_UPRIGHT` without `CAN_BE_CROUCHED`.
/// Posture transition generation auto-inserts a `CROUCH_UP` translate and flips the element's
/// `posture_after_transition` to Upright; the soldier's own Translate
/// then queues the transition animation on the now-Upright element.
///
/// The "Consider as done" else-branch at
/// the soldier command only fires when GenerateTransition couldn't promote
/// posture to Upright (e.g. on a ladder).  That arm
/// isn't reachable from Crouched once GenerateTransition is wired in.
#[test]
fn soldier_enter_attentive_mode_from_crouched_stands_first() {
    let mut display = HostDisplayState::default();
    use crate::element::{Command, Posture};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Crouched));

    let mut elem = SequenceElement::new(1, Command::EnterAttentiveMode, Some(soldier_id));
    elem.posture_after_transition = Posture::Crouched;
    engine.launch_element(elem);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    // `MakePostureTransition` translates the CROUCH_UP then the element's
    // `posture_after_transition` is Upright; the ENTER_ATTENTIVE_MODE
    // Translate queues the alerted transition animation on top.  The
    // actor's current order is whatever sits at the front of the order
    // queue — the crouch-up animation runs first.
    let front = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        front,
        Some(crate::order::OrderType::TransitionCrouchingUp),
        "crouch-up transition animation should play first"
    );
}

// ─── Waypoint-script VM dispatch ───────────────────────────────────
//
// Covers the per-waypoint VM wiring added to `MissionScript`:
// `bind_waypoint` + `call_waypoint_function`.  Each scripted waypoint
// carries its own VM and `Initialize()` + `ReachPoint(actor)` dispatch
// into that VM.

/// Build a minimal SCB with one class `TestWaypoint` that exposes
/// empty `Initialize` and `ReachPoint` functions (body: just
/// `BeginFunction` + `Return`).  Returns the parsed `ScbFile` shaped
/// for `MissionScript::from_scb`.
fn scripted_waypoint_scb() -> crate::scb::ScbFile {
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

    let waypoint_class = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "TestWaypoint".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![
            Function {
                name: "Initialize".into(),
                address: 0,
                num_parameters: 0,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 0,
            },
            Function {
                name: "ReachPoint".into(),
                address: 2,
                num_parameters: 1,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 0,
            },
        ],
        quads: vec![begin, ret, begin, ret],
    };
    // `MissionScript::from_scb` requires a `StartUp` class to bind the
    // global instance against. Supply a stub so `from_scb` succeeds.
    let startup = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: Vec::new(),
        quads: Vec::new(),
    };

    ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![startup, waypoint_class],
    }
}

/// `bind_waypoint` inserts a `ScriptInstance` keyed by `(path, wp)`
/// and runs `Initialize()` once.  A missing class returns `false`
/// and stores nothing.
#[test]
fn bind_waypoint_inserts_instance_and_missing_class_no_ops() {
    let scb = scripted_waypoint_scb();
    let mut script = MissionScript::from_scb(scb).expect("from_scb");

    assert!(script.bind_waypoint(
        crate::ai::PathId::new(2).unwrap(),
        3,
        "TestWaypoint",
        crate::natives::NativeQueryViews::default(),
    ));
    assert!(
        script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(2).unwrap(), 3))
    );

    // Unknown class is a `false` return + no map insertion.
    assert!(!script.bind_waypoint(
        crate::ai::PathId::new(4).unwrap(),
        0,
        "NonExistent",
        crate::natives::NativeQueryViews::default(),
    ));
    assert!(
        !script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(4).unwrap(), 0))
    );
}

/// `call_waypoint_function` dispatches `ReachPoint(actor)` against the
/// bound instance.  A key with no bound instance returns `Ok(0)` —
/// matches the pattern used by `call_actor_function` / `call_scroll_function`.
#[test]
fn call_waypoint_function_dispatches_and_falls_back() {
    let scb = scripted_waypoint_scb();
    let mut script = MissionScript::from_scb(scb).expect("from_scb");
    assert!(script.bind_waypoint(
        crate::ai::PathId::new(0).unwrap(),
        0,
        "TestWaypoint",
        crate::natives::NativeQueryViews::default(),
    ));

    // Bound: call dispatches cleanly.
    let actor_handle = 42;
    let ret = script
        .call_waypoint_function(
            crate::ai::PathId::new(0).unwrap(),
            0,
            "ReachPoint",
            &[actor_handle],
            crate::natives::NativeQueryViews::default(),
        )
        .expect("ReachPoint");
    assert_eq!(ret, 0, "empty ReachPoint should return 0");

    // Unbound key: `Ok(0)`, no panic.
    let ret_missing = script
        .call_waypoint_function(
            crate::ai::PathId::new(7).unwrap(),
            9,
            "ReachPoint",
            &[actor_handle],
            crate::natives::NativeQueryViews::default(),
        )
        .expect("missing instance should be Ok(0)");
    assert_eq!(ret_missing, 0);

    // Missing function on a bound instance: also `Ok(0)`.
    let ret_no_fn = script
        .call_waypoint_function(
            crate::ai::PathId::new(0).unwrap(),
            0,
            "NotAFunction",
            &[],
            crate::natives::NativeQueryViews::default(),
        )
        .expect("missing function should be Ok(0)");
    assert_eq!(ret_no_fn, 0);
}

/// AI: `execute_waypoint_script(path, wp)` sets the pending dispatch
/// slot; the old unconditional `EventAfterScriptGoOn` fire-and-forget
/// behaviour was replaced by the engine-side drain.
#[test]
fn execute_waypoint_script_queues_pending_dispatch() {
    let mut ai = crate::ai::AiController::default();
    assert!(ai.pending_waypoint_script_reach_point.is_none());
    assert!(ai.pending_self_stimuli.is_empty());

    let pid = crate::ai::PathId::new(5).unwrap();
    ai.execute_waypoint_script(pid, 2);

    assert_eq!(ai.pending_waypoint_script_reach_point, Some((pid, 2)));
    // AI must NOT pre-emptively queue `EventAfterScriptGoOn` — that
    // happens only after the engine dispatches `ReachPoint` and
    // confirms the script didn't transition into `DefaultScriptDriven`.
    assert!(ai.pending_self_stimuli.is_empty());
}

/// `initialize_mission_script_with` walks the supplied hiking paths,
/// binds every `WaypointCommand::Script` waypoint, and runs
/// `Initialize()` on each.  Verifies the end-to-end level-load path
/// registers instances keyed by `(path_idx, wp_idx)`.
#[test]
fn initialize_mission_script_binds_waypoint_classes() {
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let scb = scripted_waypoint_scb();
    let mission_script = MissionScript::from_scb(scb).expect("from_scb");

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Some(crate::campaign::Campaign::default());
    engine.scripts.mission = Some(mission_script);

    let paths = vec![
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
                    x: 10,
                    y: 10,
                    sector: 0,
                    level: 0,
                    command: WaypointCommand::Script("TestWaypoint".into()),
                },
            ],
        },
        RawHikingPath {
            waypoints: vec![RawWaypoint {
                x: 20,
                y: 20,
                sector: 0,
                level: 0,
                command: WaypointCommand::Script("TestWaypoint".into()),
            }],
        },
    ];

    let assets = crate::engine::LevelAssets::new();
    engine.initialize_mission_script_with(&assets, 0, &paths);

    let script = engine.scripts.mission.as_ref().expect("mission_script");
    // Two `Script` waypoints, both bound.
    assert!(
        script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(0).unwrap(), 1))
    );
    assert!(
        script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(1).unwrap(), 0))
    );
    // The `None`-command waypoint doesn't get a binding.
    assert!(
        !script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(0).unwrap(), 0))
    );
    assert_eq!(script.waypoint_instances.len(), 2);
}

/// Waypoint-script heaps round-trip through plain serde: heap bytes
/// written to the instance before serialising must come back
/// verbatim on deserialise.  This is the path `Engine::restore` uses
/// (via the full `EngineInner` serde derive), not a bespoke helper.
#[test]
fn waypoint_script_heap_round_trips_through_serde() {
    use crate::scb::{ClassEntry, Function, ScbFile};
    use crate::vm::{Opcode, Quad};

    // Class with a non-zero heap so we can poke distinct bytes in.
    let begin = Quad {
        operation: Opcode::BeginFunction as u8,
        operands: [0; 8],
    };
    let ret = Quad {
        operation: Opcode::Return as u8,
        operands: [0; 8],
    };
    let waypoint_class = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "HeapWaypoint".into(),
        size_of_member_variables: 8,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "Initialize".into(),
            address: 0,
            num_parameters: 0,
            size_of_return_value: 0,
            size_of_parameters: 0,
            size_of_volatile: 0,
            size_of_temporary: 0,
        }],
        quads: vec![begin, ret],
    };
    let startup = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: Vec::new(),
        quads: Vec::new(),
    };
    let scb = ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![startup, waypoint_class],
    };

    let mut script = MissionScript::from_scb(scb).expect("from_scb");
    assert!(script.bind_waypoint(
        crate::ai::PathId::new(3).unwrap(),
        7,
        "HeapWaypoint",
        crate::natives::NativeQueryViews::default(),
    ));

    // Poke distinct bytes into the heap so a zero reset is detectable.
    script
        .waypoint_instances
        .get_mut(&(crate::ai::PathId::new(3).unwrap(), 7))
        .unwrap()
        .vm
        .heap
        .copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22]);

    // Serialise → deserialise → heap bytes must match.
    let json = serde_json::to_string(&script).expect("serialize");
    let restored: crate::engine::types::MissionScript =
        serde_json::from_str(&json).expect("deserialize");

    let inst = restored
        .waypoint_instances
        .get(&(crate::ai::PathId::new(3).unwrap(), 7))
        .expect("restored");
    assert_eq!(
        inst.vm.heap,
        &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22]
    );
}

/// Leaning-out soldiers that receive `Command::ShootBow` must keep
/// the lean-out pose — `GetTransitionFlags` pairs `MUST_BE_UPRIGHT`
/// with `CAN_BE_LEANING_OUT` for SHOOT_BOW, so the auto-leave should
/// skip.
#[test]
fn soldier_leaning_out_keeps_pose_for_shoot_bow() {
    use crate::element::{Command, Posture};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::LeaningOut));

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::ShootBow);
    assert!(
        !changed,
        "ShootBow + LeaningOut must stay in lean-out pose (CAN_BE_LEANING_OUT)"
    );

    let entity = engine.get_entity(soldier_id).expect("soldier present");
    assert_eq!(entity.element_data().posture, Posture::LeaningOut);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(soldier_id)
            .is_none(),
        "no unstick animation should be queued"
    );
}

/// The `auto_leave_disguise_if_needed` path should set
/// `posture_after_transition` and `action_state_after_transition`
/// on the in-flight sequence element.
#[test]
fn soldier_leaning_out_updates_sequence_element_fields() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::LeaningOut));

    // Launch a Move sequence element so there's an element to decorate.
    let elem = SequenceElement::new_movement(
        1,
        Command::Move,
        Some(soldier_id),
        crate::order::OrderType::WalkingUpright,
    );
    let seq_id = engine.launch_element(elem);

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::Move);
    assert!(changed);

    // Locate the element and verify the post-transition fields snap.
    let found = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find(|s| s.id == seq_id)
        .and_then(|s| s.elements.iter().find(|e| e.command == Command::Move));
    let elem = found.expect("sequence element present");
    assert_eq!(elem.posture_after_transition, Posture::Upright);
    assert_eq!(elem.action_state_after_transition, ActionState::Waiting);
}

/// Regression: the synchronous `Instruct`-equivalent fires inside
/// `launch_element` for owned elements, so an element launched
/// mid-tick should be dispatched and reach `InProgress` during the
/// same `perform_hourglass` pass rather than idling one frame in
/// `Todo`.  The previous two-phase flow (launch → Todo → next-tick
/// arbitrate → dispatch) introduced a one-frame skew between launch
/// and visible state — `Instruct` runs synchronously inside
/// `LaunchSequenceElement` and ends with state `InProgress` after the
/// translate step inside the same call.
#[test]
fn launched_owned_element_reaches_in_progress_in_same_tick() {
    let mut display = HostDisplayState::default();
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    // Launch a SitDown element — the NPC translate arm pushes a single
    // TransitionWaitingUprightSitting animation order onto it and flips
    // the element to InProgress inside the same hourglass pass.
    let elem = SequenceElement::new(1, Command::SitDown, Some(soldier_id));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let elem_state = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("element still present")
        .state;
    assert_eq!(
        elem_state,
        SequenceState::InProgress,
        "launched element must reach InProgress inside the same tick as launch; got {elem_state:?}"
    );
}

#[test]
fn equip_bow_translate_plays_transition_orders() {
    let mut display = HostDisplayState::default();
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let pc_id = engine.add_entity(make_test_pc(Posture::Upright));

    let elem = SequenceElement::new(1, Command::EquipBow, Some(pc_id));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(pc_id);

    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let elem = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("EquipBow element still present");
    assert_eq!(elem.state, SequenceState::InProgress);
    assert_eq!(
        elem.action_state_after_transition,
        ActionState::AimingWithBow
    );
    assert!(
        elem.orders
            .iter()
            .any(|order| order.order_type == OrderType::TransitionEquipBow),
        "EquipBow should queue the take-bow transition"
    );
    assert!(
        elem.orders
            .iter()
            .any(|order| order.order_type == OrderType::TransitionLoadingBow),
        "EquipBow should queue the loading transition"
    );
}

// ─── NPC translate dispatch ────────────────────────────────────────
//
// The four NPC-specific commands each push a single one-shot
// animation order with `compute_direction = false` and bind sequence
// termination to its DONE.

/// Drive `perform_hourglass` once, asserting the launched element
/// pushed the expected animation onto its order queue and that the
/// order is what the animation driver sees via `current_order_for_actor`.
/// `BEGGAR_SHOW_FACE` runs against a civilian (only civilians can be
/// beggars); the others use a soldier.
fn assert_npc_translate_books(
    command: crate::element::Command,
    expected_anim: crate::order::OrderType,
) {
    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let actor = match command {
        crate::element::Command::BeggarShowFace => {
            engine.add_entity(make_test_civilian(crate::element::Posture::Upright))
        }
        _ => engine.add_entity(make_test_soldier(crate::element::Posture::Upright)),
    };

    let elem = crate::sequence::SequenceElement::new(1, command, Some(actor));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(actor);

    complete_test_runtime_fixture(&mut engine, &mut assets);
    let _ = engine.perform_hourglass(&mut display, &assets, &mut dev);

    let (order_seq, _, order_type) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(actor)
        .map(|(s, e, o)| (s, e, o.order_type))
        .expect("front order should be set");
    assert_eq!(
        order_seq, seq_id,
        "front order should live on the launched element for {command:?}",
    );
    assert_eq!(
        order_type, expected_anim,
        "wrong animation queued for {command:?}",
    );
    let elem_state = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("element present")
        .state;
    assert_eq!(
        elem_state,
        crate::sequence::SequenceState::InProgress,
        "element should stay InProgress while the anim is playing",
    );
}

#[test]
fn wake_up_translate_books_waking_up_with_antagonist() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let rescuer = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_soldier(Posture::Lying));

    bind_test_action_point(
        &mut engine,
        rescuer,
        OrderType::WakingUp,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );

    let elem = SequenceElement::new_interaction(1, Command::WakeUp, Some(rescuer), Some(target));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(rescuer);

    complete_test_runtime_fixture(&mut engine, &mut assets);
    let _ = engine.perform_hourglass(&mut display, &assets, &mut dev);

    let (order_seq, _, order) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(rescuer)
        .expect("WakeUp should queue an animation order");
    assert_eq!(order_seq, seq_id);
    assert_eq!(order.order_type, OrderType::WakingUp);
    assert_eq!(order.antagonist, Some(target));
}

#[test]
fn waking_up_done_clears_target_concussion_and_waits() {
    use super::animation::{AnimCompletionOutcomes, ExecuteSideOutcomes};
    use crate::combat::CONCUSSION_THRESHOLD;
    use crate::element::{ActionState, Posture};

    let mut engine = EngineInner::new();
    let rescuer = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_soldier(Posture::Lying));
    {
        let target_entity = engine.get_entity_mut(target).expect("target present");
        target_entity.human_data_mut().unwrap().unconscious = true;
        target_entity
            .human_data_mut()
            .unwrap()
            .concussion_of_the_brain = CONCUSSION_THRESHOLD;
        target_entity.npc_data_mut().unwrap().life_points = 30;
        target_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
    }

    let outcomes = AnimCompletionOutcomes {
        execute_sides: ExecuteSideOutcomes {
            waking_up_done: vec![(rescuer, target)],
            ..Default::default()
        },
        ..Default::default()
    };
    engine.process_anim_completion_outcomes(outcomes, &LevelAssets::new());

    let target_entity = engine.get_entity(target).expect("target present");
    assert_eq!(target_entity.element_data().posture, Posture::Lying);
    assert_eq!(
        target_entity.human_data().unwrap().concussion_of_the_brain,
        0
    );
    assert!(!target_entity.human_data().unwrap().unconscious);
    assert_eq!(
        target_entity.actor_data().unwrap().action_state,
        ActionState::Waiting
    );
    let current = engine
        .orders
        .sequence_manager
        .live_element_for_actor_matching(target, |elem| {
            elem.command == crate::element::Command::Wait
        })
        .and_then(|(seq_id, elem_idx)| engine.orders.sequence_manager.get_element(seq_id, elem_idx))
        .map(|elem| elem.command);
    assert_eq!(current, Some(crate::element::Command::Wait));
}

/// `Point` → `Pointing` animation.
#[test]
fn npc_translate_point_books_pointing_anim() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(Command::Point, OrderType::Pointing);
}

/// `SitDown` → `TransitionWaitingUprightSitting` animation.
#[test]
fn npc_translate_sit_down_books_sit_transition() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(Command::SitDown, OrderType::TransitionWaitingUprightSitting);
}

/// `BeggarShowFace` → `BeggarShowingFace` animation.  Targets a
/// civilian, since only civilians can be beggars.
#[test]
fn npc_translate_beggar_show_face_books_show_face_anim() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(Command::BeggarShowFace, OrderType::BeggarShowingFace);
}

/// `EnterLeisure` → `TransitionWaitingUprightSpecial` animation.
#[test]
fn npc_translate_enter_leisure_books_special_transition() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(
        Command::EnterLeisure,
        OrderType::TransitionWaitingUprightSpecial,
    );
}

#[test]
fn get_killed_at_bottom_kills_lying_victim_immediately() {
    use crate::element::{Command, Posture};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let killer = engine.add_entity(make_test_soldier(Posture::Upright));
    let victim = engine.add_entity(make_test_soldier(Posture::Lying));
    if let Some(crate::element::Entity::Soldier(soldier)) = engine.world.entities.get_mut(victim) {
        soldier.npc.life_points = 30;
        soldier.soldier.cached_max_life_points = 30;
        soldier.human.unconscious = true;
    }

    let elem =
        SequenceElement::new_interaction(1, Command::GetKilledAtBottom, Some(victim), Some(killer));
    engine.launch_element(elem);
    engine.ensure_wait_element(victim);

    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let entity = engine.get_entity(victim).expect("victim still present");
    assert!(entity.is_dead());
    assert_eq!(entity.element_data().posture, Posture::DeadBack);
}

/// When the `TransitionWaitingUprightSitting` animation completes,
/// the actor's posture flips to `Sitting`.
#[test]
fn npc_sit_down_anim_completion_flips_posture_to_sitting() {
    use super::animation::{ExecuteSideOutcomes, apply_npc_execute_side_effects};
    use crate::element::{ActionState, EntityId, Posture};
    use crate::order::OrderType;
    use crate::sprite::MotionState;

    let mut entity = make_test_soldier(Posture::Upright);
    let mut outcomes = ExecuteSideOutcomes::default();

    apply_npc_execute_side_effects(
        &mut entity,
        OrderType::TransitionWaitingUprightSitting,
        MotionState::Terminated,
        None,
        EntityId::Pc(crate::entity_id::PcId(0)),
        &mut outcomes,
    );

    assert_eq!(entity.element_data().posture, Posture::Sitting);
    assert_eq!(
        entity.actor_data().expect("actor data").action_state,
        ActionState::Waiting,
    );
}

/// A sitting NPC who receives `Point` first stands up: the auto-leave
/// path snaps the posture to `Upright` and queues the
/// `TransitionSittingWaitingUpright` animation on the actor's
/// `order_queue` so the visible stand-up plays before the gesture.
#[test]
fn sitting_npc_point_auto_stands_up() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_test_soldier(Posture::Sitting));

    let changed = engine.auto_leave_disguise_if_needed(actor, Command::Point);
    assert!(changed, "auto-leave should fire for Sitting + Point");

    let entity = engine.get_entity(actor).expect("entity present");
    assert_eq!(entity.element_data().posture, Posture::Upright);

    let next_order = engine
        .orders
        .sequence_manager
        .current_order_for_actor(actor)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        next_order,
        Some(OrderType::TransitionSittingWaitingUpright),
        "stand-up transition should be queued on the owning sequence element",
    );
}

/// `EnterLeisure` on an already-leisuring NPC must not auto-leave
/// leisure first — `GetTransitionFlags` sets
/// `CHANGEPOSTURE_CAN_BE_LEISURING` for this command.
#[test]
fn enter_leisure_on_leisuring_npc_skips_auto_leave() {
    use crate::element::{Command, Posture};

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_test_soldier(Posture::Leisure));

    let changed = engine.auto_leave_disguise_if_needed(actor, Command::EnterLeisure);
    assert!(
        !changed,
        "leisure-leisure re-entry should be a no-op (CAN_BE_LEISURING exempt)",
    );

    let entity = engine.get_entity(actor).expect("entity present");
    assert_eq!(entity.element_data().posture, Posture::Leisure);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(actor)
            .is_none(),
        "no transition animation should be queued",
    );
}

/// When the `TransitionWaitingUprightSpecial` animation completes,
/// the actor's posture flips to `Leisure`.
#[test]
fn npc_enter_leisure_anim_completion_flips_posture_to_leisure() {
    use super::animation::{ExecuteSideOutcomes, apply_npc_execute_side_effects};
    use crate::element::{ActionState, EntityId, Posture};
    use crate::order::OrderType;
    use crate::sprite::MotionState;

    let mut entity = make_test_soldier(Posture::Upright);
    let mut outcomes = ExecuteSideOutcomes::default();

    apply_npc_execute_side_effects(
        &mut entity,
        OrderType::TransitionWaitingUprightSpecial,
        MotionState::Done,
        None,
        EntityId::Pc(crate::entity_id::PcId(0)),
        &mut outcomes,
    );

    assert_eq!(entity.element_data().posture, Posture::Leisure);
    assert_eq!(
        entity.actor_data().expect("actor data").action_state,
        ActionState::Waiting,
    );
}

/// `remove_quick_action_titbits_for(pc, level)` looks up the
/// per-level titbit entry on the PC, drops every titbit with that id,
/// and reports whether anything was removed.
#[test]
fn remove_quick_action_titbits_for_matches_original_signature() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::EntityId;
    use crate::titbit::{ElementHandle, INVALID_ID, QuickAction, TitbitKind};

    let mut engine = EngineInner::new();
    let pc = EntityId::Pc(crate::entity_id::PcId(42));
    let slot: u8 = 1;

    // Empty slot → early-returns on the sentinel id.
    assert!(!engine.remove_quick_action_titbits_for(pc, slot));

    // Add a QA titbit and wire its id into the PC's macro slot.
    let pc_handle = ElementHandle(pc.index());
    let titbit_id = engine.feedback.titbit_manager.add_titbit(
        WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        0,
        TitbitKind::QuickAction,
        pc_handle,
        QuickAction::Bow as u16,
        pc_handle,
        false,
        INVALID_ID,
        true,
        Some(0.0),
        Some(0),
    );
    assert_ne!(titbit_id, INVALID_ID);
    engine
        .players
        .macro_store
        .get_or_insert(pc)
        .set_slot_titbit(
            slot as usize,
            crate::titbit::TitbitId::new(titbit_id).unwrap(),
        );

    // Populated slot → drops the titbit and reports success.
    assert!(engine.remove_quick_action_titbits_for(pc, slot));
    assert!(
        !engine
            .feedback
            .titbit_manager
            .titbits()
            .iter()
            .any(|t| t.id == titbit_id),
        "titbit with id {titbit_id} should be gone"
    );

    // Second call after the list is empty: slot still holds the stale
    // id (the caller clears the level entry after this returns), but
    // no titbit matches, so it returns false.
    assert!(!engine.remove_quick_action_titbits_for(pc, slot));
}

// ── QA macro playback / abort system tests ─────────────────────────

/// Seed a PC's macro slot with a recorded "move to (x,y)" step and a
/// wired titbit.  Used by the playback/abort/tetris tests below.
#[cfg(test)]
fn seed_macro_slot(
    engine: &mut EngineInner,
    pc: crate::element::EntityId,
    slot: u8,
    steps: Vec<(f32, f32)>,
) {
    use crate::coordinates::WorldPoint3D;
    use crate::macro_store::{QaReplayCommand, QuickActionStep};
    use crate::titbit::{ElementHandle, INVALID_ID, QuickAction, TitbitKind};

    let pc_handle = ElementHandle(pc.index());
    let titbit_id = engine.feedback.titbit_manager.add_titbit(
        WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        0,
        TitbitKind::QuickAction,
        pc_handle,
        QuickAction::Walk as u16,
        pc_handle,
        false,
        INVALID_ID,
        true,
        Some(0.0),
        Some(0),
    );

    let state = engine.players.macro_store.get_or_insert(pc);
    state.begin_recording(slot);
    for (x, y) in steps {
        let pos = crate::coordinates::MapPoint::new(x, y);
        state.append_if_recording(QuickActionStep {
            action: crate::profiles::Action::NoAction,
            position: pos,
            replay: QaReplayCommand::Move {
                destination: pos,
                running: false,
            },
        });
    }
    state.stop_recording();
    state.set_slot_titbit(
        slot as usize,
        crate::titbit::TitbitId::new(titbit_id).unwrap(),
    );
}

/// `EngineInner::has_quick_action` reports whether a PC has a macro in a slot.
#[test]
fn has_quick_action_reads_macro_store() {
    use crate::element::EntityId;

    let mut engine = EngineInner::new();
    let pc = EntityId::Pc(crate::entity_id::PcId(10));

    assert!(!engine.has_quick_action(pc, 0));

    seed_macro_slot(&mut engine, pc, 1, vec![(100.0, 100.0)]);

    assert!(!engine.has_quick_action(pc, 0));
    assert!(engine.has_quick_action(pc, 1));
    assert!(!engine.has_quick_action(pc, 2));
}

/// `EngineInner::abort_quick_action` drops the slot's titbit and clears the
/// slot.
#[test]
fn abort_quick_action_clears_slot_and_titbit() {
    use crate::element::EntityId;

    let mut engine = EngineInner::new();
    let pc = EntityId::Pc(crate::entity_id::PcId(20));

    // Empty slot → false.
    assert!(!engine.abort_quick_action(pc, 0));

    seed_macro_slot(&mut engine, pc, 2, vec![(1.0, 2.0), (3.0, 4.0)]);
    assert!(engine.has_quick_action(pc, 2));
    let titbit_count_before = engine.feedback.titbit_manager.titbits().len();
    assert_eq!(titbit_count_before, 1);

    // Aborting returns true and fully clears state.
    assert!(engine.abort_quick_action(pc, 2));
    assert!(!engine.has_quick_action(pc, 2));
    assert!(engine.feedback.titbit_manager.titbits().is_empty());

    // A second abort is a no-op.
    assert!(!engine.abort_quick_action(pc, 2));
}

/// `DeleteMacro` PlayerCommand: single-PC variant drops one slot
/// without tetris; all-PC variant drops + collapses.
#[test]
fn delete_macro_command_matches_original_single_vs_all() {
    let mut display = HostDisplayState::default();
    use crate::element::EntityId;
    use crate::player_command::PlayerCommand;

    let mut engine = EngineInner::new();
    let pc_a = EntityId::Pc(crate::entity_id::PcId(30));
    let pc_b = EntityId::Pc(crate::entity_id::PcId(31));
    engine.world.pc_ids.push(pc_a);
    engine.world.pc_ids.push(pc_b);

    // Both PCs have macros in slots 0 and 1; slot 2 is empty.
    seed_macro_slot(&mut engine, pc_a, 0, vec![(1.0, 1.0)]);
    seed_macro_slot(&mut engine, pc_a, 1, vec![(2.0, 2.0)]);
    seed_macro_slot(&mut engine, pc_b, 0, vec![(3.0, 3.0)]);
    seed_macro_slot(&mut engine, pc_b, 1, vec![(4.0, 4.0)]);

    let mut input = crate::engine::InputState::default();
    let assets = crate::engine::LevelAssets::new();

    // Single-PC delete: only pc_a slot 0 cleared; no tetris → pc_a slot 1
    // stays in slot 1.
    engine.apply_command(
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::DeleteMacro {
            pc: Some(pc_a),
            slot: 0,
        },
    );
    assert!(!engine.has_quick_action(pc_a, 0));
    assert!(engine.has_quick_action(pc_a, 1));
    assert!(engine.has_quick_action(pc_b, 0));
    assert!(engine.has_quick_action(pc_b, 1));

    // All-PC delete on slot 0: pc_b slot 0 cleared, tetris collapses
    // remaining slots so pc_a/pc_b slot 0 now hold what used to be slot 1.
    engine.apply_command(
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::DeleteMacro { pc: None, slot: 0 },
    );
    assert!(engine.has_quick_action(pc_a, 0)); // was pc_a slot 1
    assert!(engine.has_quick_action(pc_b, 0)); // was pc_b slot 1
    assert!(!engine.has_quick_action(pc_a, 1));
    assert!(!engine.has_quick_action(pc_b, 1));
}

/// `StartMacro` replays a move-only macro and fires the dotted-chain
/// commands through `apply_command`.  After playback the slot is empty
/// and its titbit is gone.  For the all-PC variant on a slot where every
/// PC had a macro, tetris collapses the strip.
#[test]
fn start_macro_plays_back_move_steps_and_tetris_collapses() {
    let mut display = HostDisplayState::default();
    use crate::element::EntityId;
    use crate::player_command::PlayerCommand;

    let mut engine = EngineInner::new();
    let pc_a = EntityId::Pc(crate::entity_id::PcId(40));
    let pc_b = EntityId::Pc(crate::entity_id::PcId(41));
    engine.world.pc_ids.push(pc_a);
    engine.world.pc_ids.push(pc_b);

    // Both PCs record a one-step move macro at slot 0; pc_a has a slot-1
    // macro too.
    seed_macro_slot(&mut engine, pc_a, 0, vec![(50.0, 60.0)]);
    seed_macro_slot(&mut engine, pc_b, 0, vec![(70.0, 80.0)]);
    seed_macro_slot(&mut engine, pc_a, 1, vec![(90.0, 100.0)]);

    let mut input = crate::engine::InputState::default();
    let assets = crate::engine::LevelAssets::new();

    // Sanity: titbit manager holds all three macro titbits.
    assert_eq!(engine.feedback.titbit_manager.titbits().len(), 3);

    // All-PC StartMacro on slot 0: both PCs launch → slot 0 emptied for
    // both, then tetris shifts slot 1 → slot 0.
    engine.apply_command(
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::StartMacro { pc: None, slot: 0 },
    );

    // pc_a: slot 0 now holds what was slot 1 (90, 100); slot 1 is empty.
    // pc_b: all slots empty.
    assert!(engine.has_quick_action(pc_a, 0));
    assert!(!engine.has_quick_action(pc_a, 1));
    assert!(!engine.has_quick_action(pc_b, 0));
    assert!(!engine.has_quick_action(pc_b, 1));

    // The launched macros' titbits are gone; only pc_a's (was-slot-1)
    // titbit remains.
    assert_eq!(engine.feedback.titbit_manager.titbits().len(), 1);
}

/// `StartMacro` on an empty slot is a no-op: no dispatch, no tetris.
#[test]
fn start_macro_empty_slot_is_noop() {
    let mut display = HostDisplayState::default();
    use crate::element::EntityId;
    use crate::player_command::PlayerCommand;

    let mut engine = EngineInner::new();
    let pc = EntityId::Pc(crate::entity_id::PcId(50));
    engine.world.pc_ids.push(pc);

    // pc has a macro only in slot 2 — starting slot 0 should NOT tetris,
    // because no PC had a slot-0 macro to launch.
    seed_macro_slot(&mut engine, pc, 2, vec![(1.0, 1.0)]);

    let mut input = crate::engine::InputState::default();
    let assets = crate::engine::LevelAssets::new();

    engine.apply_command(
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::StartMacro { pc: None, slot: 0 },
    );

    // Slot 2 should still hold the macro — no tetris ran because the
    // start was a no-op.
    assert!(engine.has_quick_action(pc, 2));
    assert!(!engine.has_quick_action(pc, 0));
    assert!(!engine.has_quick_action(pc, 1));
}
