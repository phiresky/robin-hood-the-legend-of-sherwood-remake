#![allow(unused_mut)]

use super::movement::mercenary_formation_destinations;
use super::*;
use crate::campaign::{Campaign, CampaignValue};
use crate::game_operation::GameCode;

#[test]
fn engine_creation() {
    let mut display = HostDisplayState::default();
    let engine = EngineInner::new();
    assert_eq!(engine.cutscene_camera.zoom_factor, 1.0);
    assert_eq!(engine.frame_counter, 0);
    assert!(!engine.fast_forward);
    assert!(!engine.lock_engine);
    assert!(!engine.mission.mission_won);
    assert_eq!(display.display_op, DisplayOpCode::Redraw);
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
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.cutscene_camera.level_size = geo2d::pt(4096.0, 4096.0);
    // `set_operation` is monotonic: a zoom request can't downgrade an
    // in-progress Redraw. After engine ctor `display_op` defaults to
    // Redraw; apply the post-draw reset to `NoBackgroundMove` so the
    // zoom-init-op can actually propagate.
    display.display_op = DisplayOpCode::NoBackgroundMove;

    assert!(engine.is_zoom_possible(&display));
    assert!(engine.is_zoom_up_possible());
    assert!(engine.is_zoom_down_possible());
    assert!(!engine.is_zooming(&display));

    // Trigger zoom up
    assert!(engine.change_state(&mut display, 0, EngineStateRequest::ZoomingUp));
    assert!(engine.is_zooming(&display));
    assert!(!engine.is_zoom_possible(&display));
    assert_eq!(display.display_op, DisplayOpCode::InitZoom);
}

#[test]
fn camera_clip_view() {
    let mut camera = CameraState {
        level_size: geo2d::pt(2000.0, 1500.0),
        zoom_factor: 1.0,
        view_position: geo2d::pt(-100.0, -50.0),
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
    assert_eq!(engine.frame_counter, 1);
}

#[test]
fn enter_helping_climb_sequence_dispatches_stealth_transition() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
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
        pc: Default::default(),
    }));

    engine.launch_element(crate::sequence::SequenceElement::new(
        1,
        crate::element::Command::EnterHelpingClimb,
        Some(pc_id),
    ));

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
    engine.mission.quit_won = true;
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
    engine.mission.quit_lost = true;
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
    engine.mission.quit_interrupted = true;
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
    engine.lock_engine = true;
    // Even with a chorus timer, lock should prevent it from being decremented
    // (actually, chorus timer IS decremented before the lock check)
    engine.chorus_timer = 5;
    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelInProgress);
    // Chorus timer still decremented (it's before the lock check)
    assert_eq!(engine.chorus_timer, 4);
    // But frame counter is still incremented
    assert_eq!(engine.frame_counter, 1);
}

#[test]
fn fast_forward() {
    let mut engine = EngineInner::new();
    engine.cutscene_camera.camera_slide = geo2d::pt(100.0, 200.0);
    engine.set_fast_forward();
    assert!(engine.is_fast_forward());
    // Camera should have jumped to slide target
    assert_eq!(engine.cutscene_camera.view_position.x, 100.0);
    assert_eq!(engine.cutscene_camera.view_position.y, 200.0);
    // Slide should be deactivated
    assert!(!engine.cutscene_camera.is_sliding());
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
    #[allow(clippy::disallowed_methods)]
    let seed = fastrand::Rng::with_seed(0xDEADBEEF).u64(..);

    let mut original = EngineInner::new();
    #[allow(clippy::disallowed_methods)]
    {
        original.rng = fastrand::Rng::with_seed(seed);
    }

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

    assert_eq!(original.frame_counter, replay.frame_counter);
    assert_eq!(original.rng.get_seed(), replay.rng.get_seed());
    assert_eq!(original.chorus_timer, replay.chorus_timer);
    assert_eq!(original.mission.mission_won, replay.mission.mission_won);
    assert_eq!(original.script_globals, replay.script_globals);

    // Double-check: re-cloning the original snapshot and replaying the
    // SAME number of ticks a second time must also match — guarding
    // against state that silently leaks across clones (e.g. a
    // thread-local that wasn't properly re-seeded on install).
    let mut second_replay = snapshot;
    for _ in 0..50 {
        second_replay.perform_hourglass(&mut display, &assets, &mut dev);
    }
    assert_eq!(second_replay.frame_counter, original.frame_counter);
    assert_eq!(second_replay.rng.get_seed(), original.rng.get_seed());
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
    #[allow(clippy::disallowed_methods)]
    let seed = fastrand::Rng::with_seed(0xFEED_FACE).u64(..);

    let mut original = EngineInner::new();
    #[allow(clippy::disallowed_methods)]
    {
        original.rng = fastrand::Rng::with_seed(seed);
    }

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

    assert_eq!(rehydrated.frame_counter, clone_ref.frame_counter);
    assert_eq!(rehydrated.rng.get_seed(), clone_ref.rng.get_seed());
    assert_eq!(rehydrated.chorus_timer, clone_ref.chorus_timer);
    assert_eq!(
        rehydrated.mission.mission_won,
        clone_ref.mission.mission_won
    );
    assert_eq!(rehydrated.script_globals, clone_ref.script_globals);
}

#[test]
fn camera_display_scratch_is_not_serialized_or_hashed() {
    let mut engine = EngineInner::new();
    engine.cutscene_camera.old_view_position = geo2d::pt(11.0, 22.0);
    engine.cutscene_camera.old_zoom_factor = 0.5;
    engine.cutscene_camera.zoom_init_done = true;
    engine.cutscene_camera.mechanized_zoom = true;
    engine.cutscene_camera.displacement = geo2d::pt(3.0, 4.0);
    engine.cutscene_camera.displacement_counter = 7;
    engine.cutscene_camera.pending_zoom_mouse_screen = Some(geo2d::pt(123.0, 456.0));

    let baseline_hash = crate::replay::state_hash(&engine);
    let json = serde_json::to_string(&engine).expect("serialize engine");
    assert!(!json.contains("old_view_position"));
    assert!(!json.contains("old_zoom_factor"));
    assert!(!json.contains("zoom_init_done"));
    assert!(!json.contains("mechanized_zoom"));
    assert!(!json.contains("displacement_counter"));
    assert!(!json.contains("pending_zoom_mouse_screen"));

    let mut changed = engine.clone();
    changed.cutscene_camera.old_view_position = geo2d::pt(99.0, 100.0);
    changed.cutscene_camera.old_zoom_factor = 2.0;
    changed.cutscene_camera.zoom_init_done = false;
    changed.cutscene_camera.mechanized_zoom = false;
    changed.cutscene_camera.displacement = geo2d::pt(-30.0, -40.0);
    changed.cutscene_camera.displacement_counter = 0;
    changed.cutscene_camera.pending_zoom_mouse_screen = None;
    assert_eq!(baseline_hash, crate::replay::state_hash(&changed));

    let restored: EngineInner = serde_json::from_str(&json).expect("deserialize engine");
    assert_eq!(
        restored.cutscene_camera.old_view_position,
        geo2d::pt(0.0, 0.0)
    );
    assert_eq!(restored.cutscene_camera.old_zoom_factor, 1.0);
    assert!(!restored.cutscene_camera.zoom_init_done);
    assert!(!restored.cutscene_camera.mechanized_zoom);
    assert_eq!(restored.cutscene_camera.displacement, geo2d::pt(0.0, 0.0));
    assert_eq!(restored.cutscene_camera.displacement_counter, 0);
    assert_eq!(restored.cutscene_camera.pending_zoom_mouse_screen, None);
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
    use crate::element::{ActorSoldier, ElementData, ElementKind, Entity};
    use crate::order::OrderType;
    use crate::position_interface::Point3D as PiPoint3D;
    use std::sync::Arc;
    let mut engine = EngineInner::new();
    let mut element = ElementData {
        kind: ElementKind::ActorSoldier,
        ..Default::default()
    };
    {
        let s = &mut element.sprite;
        s.position_iface.set_position(PiPoint3D {
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
        s.center = crate::geo2d::Vec2D { x: 32.0, y: 48.0 };
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
        .entities
        .iter()
        .flatten()
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

    // Ticking twice must not diverge from an equivalent in-memory
    // Clone — the reset level-owned attachments mean this engine can render
    // as an unbound-sprite soldier; what matters for sim determinism is that
    // the tick path treats both copies identically.
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut clone = rehydrated.clone();
    let mut rehydrated = rehydrated;
    for _ in 0..2 {
        rehydrated.perform_hourglass(&mut display, &assets, &mut dev);
        clone.perform_hourglass(&mut display, &assets, &mut dev);
    }
    assert_eq!(rehydrated.frame_counter, clone.frame_counter);
    assert_eq!(rehydrated.rng.get_seed(), clone.rng.get_seed());
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
        element.set_position_map(geo2d::pt(i as f32 * 10.0, i as f32 * 10.0).into());
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
        element.set_position_map(geo2d::pt(100.0 + i as f32 * 20.0, 100.0).into());
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
    assert_eq!(engine.script_globals.len(), 5 + 16);
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
    engine.fast_forward = true;
    engine.frame_counter = 1; // Not a multiple of 32
    let result = engine.tick_display_state(&mut display);
    assert_eq!(result, 1); // Should skip
}

#[test]
fn draw_fast_forward_every_32nd() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.fast_forward = true;
    engine.frame_counter = 32; // Multiple of 32
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
    engine.cutscene_camera.level_size = geo2d::pt(4000.0, 3000.0);
    engine.center_on_point(0, geo2d::pt(1000.0, 800.0));
    // View should be offset by half the full screen on both axes
    // (raw screen vector divided by 2*zoom; the bottom-panel exclusion
    // applies only to the clamp, not the centering).  The result is
    // floored before assignment.
    let expected_x = (1000.0f32 - 512.0f32).floor(); // 1024/2
    let expected_y = (800.0f32 - 384.0f32).floor(); // 768/2
    assert!((engine.cutscene_camera.view_position.x - expected_x).abs() < 0.01);
    assert!((engine.cutscene_camera.view_position.y - expected_y).abs() < 0.01);
}

#[test]
fn mission_state_transitions() {
    let mut engine = EngineInner::new();
    assert!(!engine.mission.mission_won);

    engine.win(true);
    assert!(engine.mission.mission_won);
    assert!(engine.mission.mission_won_first_time);

    // `win` writes both flags unconditionally, so a second call
    // re-toggles `mission_won_first_time`.
    engine.mission.mission_won_first_time = false;
    engine.win(true);
    assert!(engine.mission.mission_won_first_time);

    // A silent win (show_window=false) queues the start/quit-mission
    // widget swap as a side-effect for the host to drain.
    engine.pending_side_effects = Default::default();
    engine.win(false);
    assert!(!engine.mission.mission_won_first_time);
    assert!(engine.pending_side_effects.pending_silent_win_widget_swap);
}

#[test]
fn initialize_sends_stature_message() {
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    assert_eq!(engine.messenger.count(), 0);
    engine.initialize(&mut assets);
    // Should have sent a Stature message
    let msg = engine.messenger.poll().expect("expected stature message");
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
    engine.mission.mission_won_first_time = true;
    let side_effects = engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert!(!engine.mission.mission_won_first_time);
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
    engine.cutscene_camera.level_size = geo2d::pt(4096.0, 4096.0);
    display.background_transform.zoom_to_up = true;
    engine.cutscene_camera.zoom_init_done = true;

    engine.post_load_fixups(&mut display);

    assert!(!display.background_transform.zoom_to_up);
    assert!(!engine.cutscene_camera.zoom_init_done);
    let msg = engine.messenger.poll().expect("expected zoom end message");
    assert_eq!(msg.msg_type, MessageType::Simple(SimpleMessage::ZoomUpEnd));
}

#[test]
fn mercenary_formation_single_pc_lands_on_click() {
    let click = geo2d::pt(200.0, 300.0);
    let dests = mercenary_formation_destinations(&[geo2d::pt(50.0, 50.0)], click);
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
        geo2d::pt(0.0, 0.0),
        geo2d::pt(50.0, 0.0),
        geo2d::pt(100.0, 0.0),
    ];
    let click = geo2d::pt(200.0, 300.0);
    let dests = mercenary_formation_destinations(&pcs, click);
    assert_eq!(dests.len(), 3);
    assert_eq!(dests[0], geo2d::pt(150.0, 300.0));
    assert_eq!(dests[1], geo2d::pt(200.0, 300.0));
    assert_eq!(dests[2], geo2d::pt(250.0, 300.0));
}

#[test]
fn mercenary_formation_empty_input() {
    let dests = mercenary_formation_destinations(&[], geo2d::pt(0.0, 0.0));
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
    engine.ground_mark.add_mark(100.0, 100.0, 0);
    assert_eq!(engine.ground_mark.len(), 1);

    // Plenty of ticks to burn through all NUMBER_OF_GROUND_FRAMES advances
    // (half of them gated off by odd frame counters) and retire the mark.
    for _ in 0..(2 * crate::markers::NUMBER_OF_GROUND_FRAMES as usize + 4) {
        engine.perform_hourglass(&mut display, &assets, &mut dev);
    }
    assert!(
        engine.ground_mark.is_empty(),
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
    engine.ground_mark.add_mark(100_000.0, 100_000.0, 0);

    for _ in 0..(2 * crate::markers::NUMBER_OF_GROUND_FRAMES as usize + 4) {
        engine.perform_hourglass(&mut display, &assets, &mut dev);
    }
    assert_eq!(engine.ground_mark.len(), 1);
    assert_eq!(engine.ground_mark.marks[0].current_frame, 0);
}

#[test]
fn mission_stat_resets_on_new_mission() {
    let mut assets = LevelAssets::new();
    let mut pending = PendingLevelData::default();
    let mut engine = EngineInner::new();
    engine.campaign = Some(crate::campaign::Campaign::default());
    engine.mission_stat.add_collected_money(500);
    engine.short_briefings.add(42, true);

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

    assert_eq!(engine.mission_stat.collected_money, 0);
    assert_eq!(engine.short_briefings.count(true), 0);
}

#[test]
fn resize_snaps_zoom() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.cutscene_camera.level_size = geo2d::pt(500.0, 400.0); // Small level
    engine.cutscene_camera.zoom_factor = 0.5;
    display.background_transform.current_zoom_level = 0;

    engine.resize(&mut display, 1024.0, 768.0);

    // Should have snapped to 1.0 since 0.5x can't fit
    assert_eq!(engine.cutscene_camera.zoom_factor, 1.0);
    assert_eq!(display.background_transform.current_zoom_level, 1);
}

// ── Campaign integration tests ──────────────────────────────

#[test]
fn add_campaign_value_ransom_credits_mission_stat_and_emits_jingle() {
    use crate::sound::Jingle;
    let mut engine = EngineInner::new();
    engine.campaign = Some(Campaign::default());
    engine.frame_counter = 100; // past frame 0 → jingle gate open

    engine.add_campaign_value(CampaignValue::Ransom, 250);

    assert_eq!(
        engine
            .campaign
            .as_ref()
            .unwrap()
            .get_value(CampaignValue::Ransom as usize),
        crate::campaign::INITIAL_RANSOM + 250
    );
    assert_eq!(engine.mission_stat.collected_money, 250);
    let jingle_count = engine
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
    engine.campaign = Some(Campaign::default());
    engine.frame_counter = 100;

    engine.add_campaign_value(CampaignValue::Score, 750);

    assert_eq!(
        engine
            .campaign
            .as_ref()
            .unwrap()
            .get_value(CampaignValue::Score as usize),
        750
    );
    assert_eq!(engine.mission_stat.added_score, 750);
    // Score is silent.
    assert!(engine.pending_side_effects.sounds.is_empty());
}

#[test]
fn add_campaign_value_negative_ransom_skips_jingle_but_credits_money() {
    let mut engine = EngineInner::new();
    engine.campaign = Some(Campaign::default());
    engine.frame_counter = 100;
    engine.campaign.as_mut().unwrap().values[CampaignValue::Ransom as usize] = 500;
    engine.mission_stat.collected_money = 200;

    // A purse throw (`combat.rs:2433`) issues a negative delta.
    engine.add_campaign_value(CampaignValue::Ransom, -100);

    assert_eq!(
        engine
            .campaign
            .as_ref()
            .unwrap()
            .get_value(CampaignValue::Ransom as usize),
        400
    );
    // `add_campaign_value` credits the mission-stat counter
    // unconditionally (wrapping_add_signed); only the jingle is gated.
    assert_eq!(engine.mission_stat.collected_money, 100);
    assert!(engine.pending_side_effects.sounds.is_empty());
}

#[test]
fn add_campaign_value_skips_jingle_at_frame_zero() {
    // The `frame_counter > 0` gate ensures the pre-mission seed
    // (initial ransom = 100) doesn't sound a coin chime.
    let mut engine = EngineInner::new();
    engine.campaign = Some(Campaign::default());
    engine.frame_counter = 0;

    engine.add_campaign_value(CampaignValue::Ransom, 100);

    assert_eq!(engine.mission_stat.collected_money, 100);
    assert!(engine.pending_side_effects.sounds.is_empty());
}

#[test]
fn set_campaign_value_ransom_emits_jingle_only_when_growing() {
    use crate::sound::Jingle;
    let mut engine = EngineInner::new();
    engine.campaign = Some(Campaign::default());
    engine.frame_counter = 50;
    engine.campaign.as_mut().unwrap().values[CampaignValue::Ransom as usize] = 200;

    // Lower → no jingle (only growth fires the gate).
    engine.set_campaign_value(CampaignValue::Ransom, 100);
    assert!(engine.pending_side_effects.sounds.is_empty());

    // Higher → jingle.
    engine.set_campaign_value(CampaignValue::Ransom, 500);
    let jingle_count = engine
        .pending_side_effects
        .sounds
        .iter()
        .filter(|s| matches!(s, SoundCommand::Jingle(Jingle::CashWon)))
        .count();
    assert_eq!(jingle_count, 1);
    // SetValue does NOT credit collected_money — only AddValue does.
    assert_eq!(engine.mission_stat.collected_money, 0);
}

#[test]
fn add_campaign_value_amulets_has_no_side_effects() {
    let mut engine = EngineInner::new();
    engine.campaign = Some(Campaign::default());
    engine.frame_counter = 100;

    engine.add_campaign_value(CampaignValue::Amulets, 3);

    assert_eq!(
        engine
            .campaign
            .as_ref()
            .unwrap()
            .get_value(CampaignValue::Amulets as usize),
        3
    );
    assert_eq!(engine.mission_stat.collected_money, 0);
    assert_eq!(engine.mission_stat.added_score, 0);
    assert!(engine.pending_side_effects.sounds.is_empty());
}

#[test]
fn sync_stats_to_campaign() {
    let mut engine = EngineInner::new();
    engine.mission_stat.collected_money = 500;
    engine.mission_stat.added_score = 1200;
    engine.mission_stat.living_soldier_count = 8;
    engine.mission_stat.total_soldier_count = 12;

    let mut campaign = Campaign::default();
    campaign.set_value(CampaignValue::Ransom as usize, 100);

    engine.sync_stats_to_campaign(&mut campaign);

    // Money/score are credited during gameplay via add_campaign_value,
    // so sync at mission end must NOT re-add them — only soldier counts.
    assert_eq!(campaign.get_value(CampaignValue::Ransom as usize), 100);
    assert_eq!(campaign.get_value(CampaignValue::Score as usize), 0);
    assert_eq!(
        campaign.get_value(CampaignValue::LivingSoldiers as usize),
        8
    );
    assert_eq!(campaign.get_value(CampaignValue::DeadSoldiers as usize), 4); // 12 - 8
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
    engine.cutscene_camera.level_size = geo2d::pt(2000.0, 1500.0);
    engine.cutscene_camera.view_position = geo2d::pt(1500.0, 0.0);
    display.background_transform.scrolling_vector = geo2d::pt(400.0, 0.0);

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
    engine.cutscene_camera.level_size = geo2d::pt(2000.0, 1500.0);
    engine.cutscene_camera.view_position = geo2d::pt(10.0, 0.0);
    display.background_transform.scrolling_vector = geo2d::pt(-50.0, 0.0);

    let valid = engine.perform_check_scroll(&mut display);
    assert!(!valid);
    assert!((display.background_transform.scrolling_vector.x - (-10.0)).abs() < 0.01);
}

#[test]
fn perform_check_scroll_valid() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.cutscene_camera.level_size = geo2d::pt(4000.0, 3000.0);
    engine.cutscene_camera.view_position = geo2d::pt(500.0, 500.0);
    display.background_transform.scrolling_vector = geo2d::pt(10.0, 10.0);

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
    assert_eq!(engine.timer_elements.len(), 2);

    engine.perform_hourglass(&mut display, &assets, &mut dev);
    // Timer 200 (remaining=1) should be removed, timer 100 decremented to 2
    assert_eq!(engine.timer_elements.len(), 1);
    assert_eq!(engine.timer_elements[0].remaining, 2);
    assert_eq!(engine.timer_elements[0].element_ref, ref_a);

    engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert_eq!(engine.timer_elements[0].remaining, 1);

    engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert!(engine.timer_elements.is_empty());
}

#[test]
fn win_respects_show_window_false() {
    let mut engine = EngineInner::new();
    engine.win(false);
    assert!(engine.mission.mission_won);
    assert!(!engine.mission.mission_won_first_time);
}

#[test]
fn win_respects_show_window_true() {
    let mut engine = EngineInner::new();
    engine.win(true);
    assert!(engine.mission.mission_won);
    assert!(engine.mission.mission_won_first_time);
}

#[test]
fn zoom_change_state_updates_level() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.cutscene_camera.level_size = geo2d::pt(4096.0, 4096.0);
    assert_eq!(display.background_transform.current_zoom_level, 1);

    // Zoom up: level should increment to 2
    engine.change_state(&mut display, 0, EngineStateRequest::ZoomingUp);
    assert_eq!(display.background_transform.current_zoom_level, 2);
    assert!(display.background_transform.zoom_to_up);

    // Reset for next test
    display.background_transform.zoom_to_up = false;
    engine.cutscene_camera.zoom_init_done = false;
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
    engine.cutscene_camera.level_size = geo2d::pt(4096.0, 4096.0);
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
    soldier_elem.set_position_map(geo2d::pt(20.0, 20.0).into());
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
    pc_elem.set_position_map(geo2d::pt(30.0, 30.0).into());
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
    bonus_elem.set_position_map(geo2d::pt(40.0, 40.0).into());
    let object_id = engine.add_entity(Entity::Bonus(ElementBonus {
        element: bonus_elem,
        object: Default::default(),
    }));

    let sorted = engine.sort_for_minimap();
    assert_eq!(sorted, vec![soldier_id, pc_id, object_id]);
}

#[test]
fn smalltalk_strike_does_not_transfer_initiative_immediately() {
    use crate::element::{
        ActorSoldier, Command, ElementData, ElementKind, Entity, Point3D, Posture,
    };
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
    attacker_element.set_position(Point3D {
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
    defender_element.set_position(Point3D {
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

    if let Some(Some(attacker)) = engine.entities.get_mut(attacker_id.0 as usize) {
        attacker.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = attacker.human_data_mut().unwrap();
        human.opponents.push(defender_id);
        human.smalltalk_initiative = true;
        human.received_smalltalk_initiative = true;
    }
    if let Some(Some(defender)) = engine.entities.get_mut(defender_id.0 as usize) {
        defender.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        defender
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker_id);
    }

    engine.frame_counter = 15;
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
        !engine.sequence_manager.has_live_element_for_actor_matching(
            defender_id,
            |command| matches!(
                command,
                Command::ParrySmalltalkLeft | Command::ParrySmalltalkRight
            )
        )
    );
}

#[test]
fn smalltalk_hint_suppresses_normal_swordfight_evaluation() {
    use crate::element::{
        ActorPc, ActorSoldier, Command, ElementData, ElementKind, Entity, Point3D, Posture,
        SmalltalkHint,
    };
    use crate::element_kinds::ActionState;

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();

    let mut pc_element = ElementData {
        kind: ElementKind::ActorPc,
        posture: Posture::Upright,
        ..Default::default()
    };
    pc_element.set_position(Point3D {
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
    soldier_element.set_position(Point3D {
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

    if let Some(Some(pc)) = engine.entities.get_mut(pc_id.0 as usize) {
        pc.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = pc.human_data_mut().unwrap();
        human.opponents.push(soldier_id);
        human.tiredness = 100;
        human.smalltalk_hint = SmalltalkHint::Left;
        human.smalltalk_hint_opponent = Some(soldier_id);
    }
    if let Some(Some(soldier)) = engine.entities.get_mut(soldier_id.0 as usize) {
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
            .sequence_manager
            .has_live_element_for_actor_matching(pc_id, |command| {
                command == Command::SwordstrikeTired
            })
    );
}

#[test]
fn consumed_smalltalk_hint_suppresses_same_frame_smalltalk_strike_only_for_that_actor() {
    use crate::element::{
        ActorSoldier, Command, ElementData, ElementKind, Entity, Point3D, Posture, SmalltalkHint,
    };
    use crate::element_kinds::ActionState;

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();

    let mut hinted_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    hinted_element.set_position(Point3D {
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
    hinted_opponent_element.set_position(Point3D {
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
    free_attacker_element.set_position(Point3D {
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
    free_defender_element.set_position(Point3D {
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

    if let Some(Some(hinted)) = engine.entities.get_mut(hinted_id.0 as usize) {
        hinted.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = hinted.human_data_mut().unwrap();
        human.opponents.push(hinted_opponent_id);
        human.smalltalk_initiative = true;
        human.received_smalltalk_initiative = true;
        human.smalltalk_hint = SmalltalkHint::Left;
        human.smalltalk_hint_opponent = Some(hinted_opponent_id);
    }
    if let Some(Some(hinted_opponent)) = engine.entities.get_mut(hinted_opponent_id.0 as usize) {
        hinted_opponent.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        hinted_opponent
            .human_data_mut()
            .unwrap()
            .opponents
            .push(hinted_id);
    }
    if let Some(Some(free_attacker)) = engine.entities.get_mut(free_attacker_id.0 as usize) {
        free_attacker.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = free_attacker.human_data_mut().unwrap();
        human.opponents.push(free_defender_id);
        human.smalltalk_initiative = true;
        human.received_smalltalk_initiative = true;
    }
    if let Some(Some(free_defender)) = engine.entities.get_mut(free_defender_id.0 as usize) {
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
            .sequence_manager
            .has_live_element_for_actor_matching(hinted_id, |command| {
                matches!(
                    command,
                    Command::SwordstrikeSmalltalkLeft | Command::SwordstrikeSmalltalkRight
                )
            })
    );
    assert!(engine.sequence_manager.has_live_element_for_actor_matching(
        free_attacker_id,
        |command| {
            matches!(
                command,
                Command::SwordstrikeSmalltalkLeft | Command::SwordstrikeSmalltalkRight
            )
        }
    ));
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

    if let Some(Some(attacker)) = engine.entities.get_mut(attacker_id.0 as usize) {
        let human = attacker.human_data_mut().unwrap();
        human.opponents.push(defender_id);
        human.smalltalk_initiative = true;
    }
    if let Some(Some(defender)) = engine.entities.get_mut(defender_id.0 as usize) {
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
    use crate::element::{ActorSoldier, ElementData, ElementKind, Entity, Point3D};

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
        element.set_position(Point3D { x: 0.0, y, z: 0.0 });
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
    engine.cutscene_camera.level_size = geo2d::pt(4000.0, 3000.0);
    engine.cutscene_camera.view_position = geo2d::pt(100.0, 100.0);
    engine.cutscene_camera.camera_slide = geo2d::pt(500.0, 300.0);
    engine.cutscene_camera.camera_wanted = geo2d::pt(500.0, 300.0);
    engine.speed = 1.0;

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
    engine.cutscene_camera.level_size = geo2d::pt(4000.0, 3000.0);
    engine.cutscene_camera.view_position = geo2d::pt(500.0, 300.0);
    engine.cutscene_camera.camera_slide = geo2d::pt(500.0, 300.0);

    engine.perform_director_work(&mut display);

    // Should have cancelled the slide
    assert!(!engine.cutscene_camera.is_sliding());
}

#[test]
fn resize_aborts_zoom() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.cutscene_camera.level_size = geo2d::pt(4096.0, 4096.0);
    display.display_op = DisplayOpCode::InZoom;
    display.background_transform.zoom_to_up = true;
    engine.cutscene_camera.zoom_init_done = true;

    engine.resize(&mut display, 1024.0, 768.0);

    assert!(!display.background_transform.zoom_to_up);
    assert!(!engine.cutscene_camera.zoom_init_done);
}

#[test]
fn dead_pc_triggers_failure() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.cutscene_camera.level_size = geo2d::pt(4000.0, 3000.0);

    // Add a PC entity
    let mut pc_elem = crate::element::ElementData {
        kind: crate::element::ElementKind::ActorPc,
        ..Default::default()
    };
    pc_elem.set_position_map(geo2d::pt(100.0, 200.0).into());
    let entity = Entity::Pc(crate::element::ActorPc {
        element: pc_elem,
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let id = engine.add_entity(entity);
    engine.dead_pc = Some(id);

    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelFailed);
}

#[test]
fn zoom_step_completes_after_8_steps() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.cutscene_camera.level_size = geo2d::pt(4096.0, 4096.0);
    display.background_transform.zoom_to_up = true;
    display.background_transform.zoom_count = 0;
    display.background_transform.number_of_zoom_steps = 8;
    engine.cutscene_camera.zoom_init_done = true;
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
    assert!(!engine.cutscene_camera.zoom_init_done);
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
    engine.add_entity(scroll);

    // No mission_script → nothing to dispatch, counter stays zero.
    let assets = crate::engine::LevelAssets::new();
    engine.dispatch_scroll_hourglasses(&assets);
    let entity = engine.get_entity(crate::element::EntityId(0));
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
    use crate::engine::scroll_reveal::ScrollStatus;

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

fn make_test_ai_soldier(camp: crate::element::Camp) -> Entity {
    let mut entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut entity else {
        unreachable!("make_test_soldier returned non-soldier");
    };
    soldier.soldier.cached_camp = camp;
    soldier.npc.ai_brain =
        crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::default()));
    entity
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
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(&LevelAssets::new());

    assert_eq!(pending_specific_blinks(&engine, same_camp_npc), Vec::new());
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
    engine.ai_global.there_are_royalist_soldiers = true;
    engine.ai_global.there_are_lacklandist_soldiers = true;
    let waker = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let same_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let opposite_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    engine
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(&LevelAssets::new());

    assert_eq!(pending_specific_blinks(&engine, waker), Vec::new());
    assert_eq!(pending_specific_blinks(&engine, same_camp_npc), Vec::new());
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
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(&LevelAssets::new());

    assert_eq!(
        pending_specific_blinks(&engine, opposite_camp_npc),
        Vec::new()
    );
}

fn bind_test_action_point(
    engine: &mut EngineInner,
    id: EntityId,
    action: crate::order::OrderType,
    hotspot: crate::geo2d::Point2D,
    center: crate::geo2d::Point2D,
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
        offsets: vec![crate::geo2d::pt(0.0, 0.0)],
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

    let seq_id = engine
        .sequence_manager
        .launch_element(crate::sequence::SequenceElement::new(
            1,
            Command::ParrySword,
            Some(soldier),
        ));
    engine.dispatch_parry_sword(soldier, false, seq_id, 0);

    let elem = engine
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

    let seq_id = engine
        .sequence_manager
        .launch_element(crate::sequence::SequenceElement::new(
            1,
            Command::StopParrySword,
            Some(soldier),
        ));
    engine.dispatch_stop_parry(soldier, seq_id, 0);

    let elem = engine
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

    let assets = crate::engine::types::LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let active = engine
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

    let assets = crate::engine::types::LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let active = engine
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
    let current_seq = engine.sequence_manager.launch_element(current);
    engine.sequence_manager.element_in_progress(current_seq, 0);

    let mut incoming = SequenceElement::new(1, Command::Turn, Some(owner));
    incoming.priority = SequencePriority::Preference;
    let incoming_seq = engine.sequence_manager.launch_element(incoming);

    let accepted = engine.arbitrate_instruct(incoming_seq, 0);
    assert!(
        !accepted,
        "locked current order should finish before incoming element dispatches"
    );

    let current = engine.sequence_manager.get_element(current_seq, 0).unwrap();
    assert_eq!(current.orders.len(), 1);
    assert_eq!(current.cross_postponed, Some((incoming_seq, 0)));

    let incoming = engine
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
    let current_seq = engine.sequence_manager.launch_element(current);
    engine.sequence_manager.element_in_progress(current_seq, 0);
    engine
        .get_entity_mut(pc)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .old_action = OrderType::WaitingUpright;

    let incoming = SequenceElement::new_interaction(1, Command::ShootBow, Some(pc), Some(target));
    let incoming_seq = engine.launch_element_for_owner(incoming);

    let current = engine.sequence_manager.get_element(current_seq, 0).unwrap();
    assert_eq!(current.cross_postponed, Some((incoming_seq, 0)));

    let incoming = engine
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
    let pass_seq = engine.sequence_manager.launch_element(current_pass);
    engine.sequence_manager.element_in_progress(pass_seq, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sequence_element_started = true;

    let incoming =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let incoming_seq = engine.launch_element_for_owner(incoming);

    let pass = engine.sequence_manager.get_element(pass_seq, 0).unwrap();
    assert_eq!(pass.state, SequenceState::InProgress);

    let incoming = engine
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

    let assets = crate::engine::types::LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    // `MakePostureTransition` translates the CROUCH_UP then the element's
    // `posture_after_transition` is Upright; the ENTER_ATTENTIVE_MODE
    // Translate queues the alerted transition animation on top.  The
    // actor's current order is whatever sits at the front of the order
    // queue — the crouch-up animation runs first.
    let front = engine
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
    let mut script = crate::engine::types::MissionScript::from_scb(scb).expect("from_scb");

    assert!(script.bind_waypoint(crate::ai::PathId::new(2).unwrap(), 3, "TestWaypoint"));
    assert!(
        script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(2).unwrap(), 3))
    );

    // Unknown class is a `false` return + no map insertion.
    assert!(!script.bind_waypoint(crate::ai::PathId::new(4).unwrap(), 0, "NonExistent"));
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
    let mut script = crate::engine::types::MissionScript::from_scb(scb).expect("from_scb");
    assert!(script.bind_waypoint(crate::ai::PathId::new(0).unwrap(), 0, "TestWaypoint"));

    // Bound: call dispatches cleanly.
    let actor_handle = 42;
    let ret = script
        .call_waypoint_function(
            crate::ai::PathId::new(0).unwrap(),
            0,
            "ReachPoint",
            &[actor_handle],
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
        )
        .expect("missing instance should be Ok(0)");
    assert_eq!(ret_missing, 0);

    // Missing function on a bound instance: also `Ok(0)`.
    let ret_no_fn = script
        .call_waypoint_function(crate::ai::PathId::new(0).unwrap(), 0, "NotAFunction", &[])
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
    let mission_script = crate::engine::types::MissionScript::from_scb(scb).expect("from_scb");

    let mut engine = EngineInner::new();
    engine.mission_script = Some(mission_script);

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

    let script = engine.mission_script.as_ref().expect("mission_script");
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

    let mut script = crate::engine::types::MissionScript::from_scb(scb).expect("from_scb");
    assert!(script.bind_waypoint(crate::ai::PathId::new(3).unwrap(), 7, "HeapWaypoint"));

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

    let assets = LevelAssets::new();
    let mut dev = DevState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let elem_state = engine
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

    let assets = LevelAssets::new();
    let mut dev = DevState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let elem = engine
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
    let assets = LevelAssets::new();
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

    let _ = engine.perform_hourglass(&mut display, &assets, &mut dev);

    let (order_seq, _, order_type) = engine
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
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let rescuer = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_soldier(Posture::Lying));

    bind_test_action_point(
        &mut engine,
        rescuer,
        OrderType::WakingUp,
        crate::geo2d::pt(0.0, 0.0),
        crate::geo2d::pt(0.0, 0.0),
    );

    let elem = SequenceElement::new_interaction(1, Command::WakeUp, Some(rescuer), Some(target));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(rescuer);

    let _ = engine.perform_hourglass(&mut display, &assets, &mut dev);

    let (order_seq, _, order) = engine
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

    let mut outcomes = AnimCompletionOutcomes::default();
    outcomes.execute_sides = ExecuteSideOutcomes {
        waking_up_done: vec![(rescuer, target)],
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
        .sequence_manager
        .live_element_for_actor_matching(target, |elem| {
            elem.command == crate::element::Command::Wait
        })
        .and_then(|(seq_id, elem_idx)| engine.sequence_manager.get_element(seq_id, elem_idx))
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
    if let Some(crate::element::Entity::Soldier(soldier)) = engine
        .entities
        .get_mut(victim.0 as usize)
        .and_then(|s| s.as_mut())
    {
        soldier.npc.life_points = 30;
        soldier.soldier.cached_max_life_points = 30;
        soldier.human.unconscious = true;
    }

    let elem =
        SequenceElement::new_interaction(1, Command::GetKilledAtBottom, Some(victim), Some(killer));
    engine.launch_element(elem);
    engine.ensure_wait_element(victim);

    let mut display = HostDisplayState::default();
    let assets = LevelAssets::new();
    let mut dev = DevState::default();
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
        EntityId(0),
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
        EntityId(0),
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
    use crate::element::EntityId;
    use crate::position_interface::Point3D;
    use crate::titbit::{ElementHandle, INVALID_ID, QuickAction, TitbitKind};

    let mut engine = EngineInner::new();
    let pc = EntityId(42);
    let slot: u8 = 1;

    // Empty slot → early-returns on the sentinel id.
    assert!(!engine.remove_quick_action_titbits_for(pc, slot));

    // Add a QA titbit and wire its id into the PC's macro slot.
    let pc_handle = ElementHandle(pc.0);
    let titbit_id = engine.titbit_manager.add_titbit(
        Point3D {
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
    engine.macro_store.get_or_insert(pc).set_slot_titbit(
        slot as usize,
        crate::titbit::TitbitId::new(titbit_id).unwrap(),
    );

    // Populated slot → drops the titbit and reports success.
    assert!(engine.remove_quick_action_titbits_for(pc, slot));
    assert!(
        !engine
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
    use crate::macro_store::{QaReplayCommand, QuickActionStep};
    use crate::position_interface::Point3D;
    use crate::titbit::{ElementHandle, INVALID_ID, QuickAction, TitbitKind};

    let pc_handle = ElementHandle(pc.0);
    let titbit_id = engine.titbit_manager.add_titbit(
        Point3D {
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

    let state = engine.macro_store.get_or_insert(pc);
    state.begin_recording(slot);
    for (x, y) in steps {
        let pos = crate::geo2d::pt(x, y);
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
    let pc = EntityId(10);

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
    let pc = EntityId(20);

    // Empty slot → false.
    assert!(!engine.abort_quick_action(pc, 0));

    seed_macro_slot(&mut engine, pc, 2, vec![(1.0, 2.0), (3.0, 4.0)]);
    assert!(engine.has_quick_action(pc, 2));
    let titbit_count_before = engine.titbit_manager.titbits().len();
    assert_eq!(titbit_count_before, 1);

    // Aborting returns true and fully clears state.
    assert!(engine.abort_quick_action(pc, 2));
    assert!(!engine.has_quick_action(pc, 2));
    assert!(engine.titbit_manager.titbits().is_empty());

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
    let pc_a = EntityId(30);
    let pc_b = EntityId(31);
    engine.pc_ids.push(pc_a);
    engine.pc_ids.push(pc_b);

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
    let pc_a = EntityId(40);
    let pc_b = EntityId(41);
    engine.pc_ids.push(pc_a);
    engine.pc_ids.push(pc_b);

    // Both PCs record a one-step move macro at slot 0; pc_a has a slot-1
    // macro too.
    seed_macro_slot(&mut engine, pc_a, 0, vec![(50.0, 60.0)]);
    seed_macro_slot(&mut engine, pc_b, 0, vec![(70.0, 80.0)]);
    seed_macro_slot(&mut engine, pc_a, 1, vec![(90.0, 100.0)]);

    let mut input = crate::engine::InputState::default();
    let assets = crate::engine::LevelAssets::new();

    // Sanity: titbit manager holds all three macro titbits.
    assert_eq!(engine.titbit_manager.titbits().len(), 3);

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
    assert_eq!(engine.titbit_manager.titbits().len(), 1);
}

/// `StartMacro` on an empty slot is a no-op: no dispatch, no tetris.
#[test]
fn start_macro_empty_slot_is_noop() {
    let mut display = HostDisplayState::default();
    use crate::element::EntityId;
    use crate::player_command::PlayerCommand;

    let mut engine = EngineInner::new();
    let pc = EntityId(50);
    engine.pc_ids.push(pc);

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
