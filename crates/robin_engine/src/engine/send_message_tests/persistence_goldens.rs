//! Current-format persistence checks for mission scripts and side effects.

use super::*;
use crate::engine::MissionScript;

fn populated_engine() -> EngineInner {
    let (mut engine, _receiver, handle) = engine_with_receiver();
    engine.scripts.globals[3] = 0x1234_5678;
    let script = engine.scripts.mission.as_mut().expect("script installed");
    script.bind_target(77, "MessageReceiver");
    script.bind_scroll(78, "MessageReceiver");
    script.bind_waypoint(
        crate::ai::PathId::new(4).expect("path id"),
        3,
        "MessageReceiver",
    );
    let class = script
        .manager
        .find_class("MessageReceiver")
        .expect("receiver class");
    let zone = script.manager.create_instance_idx(class);
    script.zone_instances.insert(2, zone);
    script.replace_actor_vm_heap(handle, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    std::sync::Arc::make_mut(&mut script.manager.static_area)[5] = 0xA5;
    script.script_name = "golden_mission".into();
    script.enable_spellforge_virtual_bindings();
    script.bind_spellforge_virtual_zone(9);

    let effects = &mut engine.feedback.pending_side_effects;
    effects.code = crate::game_operation::GameCode::LevelSucceeded;
    effects.overlay = Some(crate::engine::OverlayChange::Hide);
    effects.request_signal(crate::engine::HostSignal::ResetModalInput);
    effects.fade_to_black = Some(Some(crate::engine::FadeToBlack {
        speed: 12,
        frames_remaining: 7,
    }));
    effects.set_draw_hidden = Some(false);
    effects.extend_dialogues(vec![5, -1, 99]);
    effects.extend_popup_texts(vec![42]);
    effects.request_sherwood_report();
    {
        effects.request_signal(crate::engine::HostSignal::MissionStateNotice);
        effects.request_signal(crate::engine::HostSignal::MissionStatePopup);
    };
    effects.request_signal(crate::engine::HostSignal::ClearUiFocus);
    effects.pending_minimap_position = Some(crate::coordinates::ScreenPoint::new(56.0, 78.0));
    effects.pending_minimap_display_maps = vec![crate::engine::MinimapDisplayRequest {
        show: true,
        restore_position: false,
    }];
    engine
}

#[test]
fn mission_script_and_side_effects_roundtrip_with_persisted_clone() {
    let engine = populated_engine();
    let script = engine.scripts.mission.as_ref().expect("script installed");
    let script_json = serde_json::to_string(script).unwrap();
    let script_native = bitcode::encode(script);
    let json_script: MissionScript = serde_json::from_str(&script_json).unwrap();
    let native_script: MissionScript = bitcode::decode(&script_native).unwrap();
    for restored in [&json_script, &native_script] {
        assert_eq!(serde_json::to_string(restored).unwrap(), script_json);
        assert_eq!(bitcode::encode(restored), script_native);
        assert_eq!(
            robin_util::state_hash::compute(restored),
            robin_util::state_hash::compute(script),
        );
        assert_eq!(
            restored.manager.class_count(),
            0,
            "compiled program is reloaded separately"
        );
    }

    let effects = &engine.feedback.pending_side_effects;
    let effects_json = serde_json::to_string(effects).unwrap();
    let effects_native = bitcode::encode(effects);
    let json_effects: crate::engine::HostEffects = serde_json::from_str(&effects_json).unwrap();
    let native_effects: crate::engine::HostEffects = bitcode::decode(&effects_native).unwrap();
    assert_eq!(bitcode::encode(&native_effects), effects_native);
    assert_eq!(
        native_effects.pending_minimap_position,
        effects.pending_minimap_position
    );
    assert_eq!(json_effects.pending_minimap_position, None);
    for restored in [&json_effects, &native_effects] {
        assert_eq!(serde_json::to_string(restored).unwrap(), effects_json);
        assert_eq!(
            robin_util::state_hash::compute(restored),
            robin_util::state_hash::compute(effects),
        );
    }

    let persisted = super::super::snapshot::PersistedEngineState::capture(&engine)
        .expect("idle engine persists")
        .into_engine_inner();
    let engine_json = serde_json::to_string(&engine).unwrap();
    let restored: EngineInner = serde_json::from_str(&engine_json).unwrap();
    for restored in [&persisted, &restored] {
        assert_eq!(
            bitcode::encode(&restored.scripts),
            bitcode::encode(&engine.scripts)
        );
        assert_eq!(
            bitcode::encode(&restored.feedback),
            bitcode::encode(&engine.feedback.persisted_clone())
        );
        assert_eq!(
            restored
                .scripts
                .mission
                .as_ref()
                .unwrap()
                .manager
                .class_count(),
            0
        );
        assert_eq!(
            restored
                .feedback
                .pending_side_effects
                .pending_minimap_position,
            None
        );
    }
}

#[test]
fn active_callback_rejects_json_and_persisted_clone() {
    let mut engine = populated_engine();
    let handle = *engine
        .scripts
        .mission
        .as_ref()
        .unwrap()
        .actor_instances
        .keys()
        .next()
        .unwrap();
    engine
        .scripts
        .mission
        .as_mut()
        .unwrap()
        .push_active_driver_frame(crate::natives::ScriptCallFrame::actor(handle), true);
    let script = engine.scripts.mission.as_ref().unwrap();
    let error = serde_json::to_string(script).unwrap_err().to_string();
    assert!(error.contains("active script callback"), "{error}");
    let error = serde_json::to_string(&engine).unwrap_err().to_string();
    assert!(error.contains("active script callback"), "{error}");
    let error = match super::super::snapshot::PersistedEngineState::capture(&engine) {
        Ok(_) => panic!("active callback was persisted"),
        Err(error) => error,
    };
    assert!(error.contains("active script callback"), "{error}");
    // Native bitcode encoding panics instead; the Cranelift test profile
    // cannot unwind, so that path is not exercised here.
}
