use super::*;

fn native_round_trip(engine: &EngineInner) -> EngineInner {
    let bytes = super::super::snapshot::encode_native_engine_inner(engine);
    super::super::snapshot::decode_native_engine_inner(&bytes).expect("decode native engine")
}

fn engine_snapshot_fixture() -> EngineInner {
    let mut engine = EngineInner::new();

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
fn engine_snapshot_schema_follows_current_owners() {
    use std::collections::BTreeSet;

    let engine = engine_snapshot_fixture();
    let json = serde_json::to_value(&engine).expect("serialize snapshot fixture to JSON");
    let object = json
        .as_object()
        .expect("EngineInner snapshot must remain a top-level map");
    let actual_keys = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_keys = [
        "ai",
        "control",
        "feedback",
        "mission_domain",
        "orders",
        "players",
        "script_domains",
        "scripts",
        "world",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(actual_keys, expected_keys);
}

#[test]
fn engine_nested_owner_fields_round_trip() {
    let engine = engine_snapshot_fixture();
    let expected_hash = crate::replay::state_hash(&engine);
    let json = serde_json::to_value(&engine).expect("serialize engine");
    let object = json
        .as_object()
        .expect("EngineInner snapshot must be an object");
    assert_eq!(
        json["mission_domain"]["state"]["map_name"],
        "compatibility-map"
    );
    assert_eq!(json["control"]["frame_counter"], 0x1122_3344u32);
    assert_eq!(json["ai"]["standard_view_polygon_radius"], 321);
    assert_eq!(json["script_domains"]["mission_ui"]["force_check"], true);
    assert!(!object.contains_key("mission"));
    assert!(!object.contains_key("frame_counter"));
    assert!(!object.contains_key("force_check"));

    let restored: EngineInner = serde_json::from_value(json).expect("restore current engine");
    assert_eq!(crate::replay::state_hash(&restored), expected_hash);
}

#[test]
fn engine_state_hash_is_deterministic_within_the_current_build() {
    let engine = engine_snapshot_fixture();
    let clone = engine.clone();
    assert_eq!(
        crate::replay::state_hash(&engine),
        crate::replay::state_hash(&clone)
    );

    let restored = native_round_trip(&engine);
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
    let control = object
        .get("control")
        .and_then(serde_json::Value::as_object)
        .expect("simulation control should serialize as a nested owner");
    let gates = control
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
    assert!(!control.contains_key("lock_engine"));
    assert!(!control.contains_key("freeze_all"));

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

    let mut replay = native_round_trip(&original);
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
        original.perform_hourglass(
            &mut original_display,
            &mut InputState::default(),
            &assets,
            &mut original_dev,
        );
        replay.perform_hourglass(
            &mut replay_display,
            &mut InputState::default(),
            &assets,
            &mut replay_dev,
        );
        assert_eq!(
            crate::replay::state_hash(&original),
            crate::replay::state_hash(&replay)
        );
    }
}

#[test]
fn engine_camera_zoom_gate_ignores_host_display_during_rollback_tick() {
    let assets = LevelAssets::new();
    let mut live = EngineInner::new();
    live.feedback
        .cutscene_camera
        .display
        .background_transform
        .zoom_to_up = true;
    live.feedback.cutscene_camera.zoom_init_done = true;
    live.send_simple_message(crate::messenger::SimpleMessage::LockAlt);

    let mut replay = native_round_trip(&live);

    let mut live_display = HostDisplayState::default();
    live_display.background_transform.zoom_to_up = false;
    live_display.background_transform.zoom_to_down = false;
    let mut replay_display = HostDisplayState::default();
    replay_display.background_transform.zoom_to_up = false;
    replay_display.background_transform.zoom_to_down = true;
    let mut live_dev = DevState::default();
    let mut replay_dev = DevState::default();

    live.perform_hourglass(
        &mut live_display,
        &mut InputState::default(),
        &assets,
        &mut live_dev,
    );
    replay.perform_hourglass(
        &mut replay_display,
        &mut InputState::default(),
        &assets,
        &mut replay_dev,
    );

    assert!(
        !live.is_lock_alt(),
        "active Engine zoom must gate messenger work"
    );
    assert!(!replay.is_lock_alt());
    assert_eq!(
        crate::replay::state_hash(&live),
        crate::replay::state_hash(&replay)
    );
}

#[test]
fn rng_snapshot_restores_next_gameplay_draw_and_state_hash() {
    let mut live = EngineInner::new();
    live.restore_rng_from_seed(0xA036_5EED_CAFE_BEEF);
    live.with_simulation_context(|_, sim| {
        let _ = crate::sim_rng::script_rand(sim, crate::sim_rng::RngSite::ScriptRand, 97)
            .expect("positive script bound");
    });

    let mut restored = native_round_trip(&live);
    assert_eq!(
        crate::replay::state_hash(&live),
        crate::replay::state_hash(&restored)
    );

    let next_live = live.with_simulation_context(|_, sim| {
        (
            crate::sim_rng::script_rand(sim, crate::sim_rng::RngSite::ScriptRand, 101)
                .expect("positive script bound"),
            crate::sim_rng::script_rand(sim, crate::sim_rng::RngSite::ScriptRand, 17)
                .expect("positive script bound"),
        )
    });
    let next_restored = restored.with_simulation_context(|_, sim| {
        (
            crate::sim_rng::script_rand(sim, crate::sim_rng::RngSite::ScriptRand, 101)
                .expect("positive script bound"),
            crate::sim_rng::script_rand(sim, crate::sim_rng::RngSite::ScriptRand, 17)
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
