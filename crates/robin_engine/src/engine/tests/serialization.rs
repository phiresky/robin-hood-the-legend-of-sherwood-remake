use super::*;

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
fn camera_write_only_presentation_scratch_is_not_serialized_or_hashed() {
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.old_view_position =
        crate::coordinates::MapPoint::new(11.0, 22.0);
    engine.feedback.cutscene_camera.old_zoom_factor = 0.5;

    let baseline_hash = crate::replay::state_hash(&engine);
    let json = serde_json::to_string(&engine).expect("serialize engine");
    assert!(!json.contains("old_view_position"));
    assert!(!json.contains("old_zoom_factor"));

    let mut changed = engine.clone();
    changed.feedback.cutscene_camera.old_view_position =
        crate::coordinates::MapPoint::new(99.0, 100.0);
    changed.feedback.cutscene_camera.old_zoom_factor = 2.0;
    assert_eq!(baseline_hash, crate::replay::state_hash(&changed));

    let restored: EngineInner = serde_json::from_str(&json).expect("deserialize engine");
    assert_eq!(
        restored.feedback.cutscene_camera.old_view_position,
        crate::coordinates::MapPoint::new(0.0, 0.0)
    );
    assert_eq!(restored.feedback.cutscene_camera.old_zoom_factor, 1.0);
}

#[test]
fn camera_transition_inputs_are_serialized_and_hashed() {
    let baseline = EngineInner::new();

    let mut changed = baseline.clone();
    changed.feedback.cutscene_camera.zoom_init_done = true;
    assert_ne!(
        crate::replay::state_hash(&baseline),
        crate::replay::state_hash(&changed),
        "zoom_init_done gates gameplay and sequence completion"
    );

    let mut changed = baseline.clone();
    changed.feedback.cutscene_camera.mechanized_zoom = true;
    assert_ne!(
        crate::replay::state_hash(&baseline),
        crate::replay::state_hash(&changed),
        "mechanized_zoom changes the zoom anchor"
    );

    let mut changed = baseline.clone();
    changed.feedback.cutscene_camera.displacement = MapVec::new(3.0, 4.0);
    assert_ne!(
        crate::replay::state_hash(&baseline),
        crate::replay::state_hash(&changed),
        "follow displacement changes the next camera position"
    );

    let mut changed = baseline.clone();
    changed.feedback.cutscene_camera.displacement_counter = 7;
    assert_ne!(
        crate::replay::state_hash(&baseline),
        crate::replay::state_hash(&changed),
        "follow displacement counter changes the next camera step"
    );

    let mut changed = baseline.clone();
    changed.feedback.cutscene_camera.pending_zoom_mouse_screen =
        Some(crate::coordinates::ScreenPoint::new(123.0, 456.0));
    assert_ne!(
        crate::replay::state_hash(&baseline),
        crate::replay::state_hash(&changed),
        "pending zoom anchor changes the next camera transition"
    );

    let json = serde_json::to_value(&changed).expect("serialize deterministic camera inputs");
    let camera = &json["feedback"]["cutscene_camera"];
    assert!(camera.get("zoom_init_done").is_some());
    assert!(camera.get("mechanized_zoom").is_some());
    assert!(camera.get("displacement").is_some());
    assert!(camera.get("displacement_counter").is_some());
    assert!(camera.get("pending_zoom_mouse_screen").is_some());
    let restored: EngineInner = serde_json::from_value(json).expect("restore camera inputs");
    assert_eq!(
        crate::replay::state_hash(&restored),
        crate::replay::state_hash(&changed)
    );
}

#[test]
fn missing_required_camera_transition_input_fails_contextually() {
    for field in [
        "zoom_init_done",
        "mechanized_zoom",
        "displacement",
        "displacement_counter",
        "pending_zoom_mouse_screen",
    ] {
        let mut json = serde_json::to_value(EngineInner::new()).expect("serialize engine");
        json["feedback"]["cutscene_camera"]
            .as_object_mut()
            .expect("camera snapshot object")
            .remove(field);
        let error = match serde_json::from_value::<EngineInner>(json) {
            Ok(_) => panic!("missing deterministic camera input {field} must fail"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(field),
            "missing {field} error lacked field context: {error}"
        );
    }
}

#[test]
fn host_only_minimap_position_output_is_not_serialized_or_hashed() {
    let baseline = EngineInner::new();
    let mut changed = baseline.clone();
    changed
        .feedback
        .pending_side_effects
        .pending_minimap_position = Some(crate::coordinates::ScreenPoint::new(123.0, 456.0));

    assert_eq!(
        crate::replay::state_hash(&baseline),
        crate::replay::state_hash(&changed)
    );
    let json = serde_json::to_string(&changed).expect("serialize engine");
    assert!(!json.contains("pending_minimap_position"));
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
