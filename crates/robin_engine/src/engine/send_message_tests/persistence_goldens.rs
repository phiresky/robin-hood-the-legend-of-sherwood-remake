//! Byte-exact persistence goldens for `MissionScript` and `SideEffects`.
//!
//! The digests were recorded from the former hand-maintained save mirrors
//! (`PersistedMissionScript`, `MissionScriptSnapshot[Ref]`,
//! `PersistedSideEffects`) before they were replaced by direct derives. They
//! pin serde JSON, native bitcode, state hashes and the in-memory persisted
//! clone. Re-bless only for an intentional format change with
//! `ROBIN_BLESS_SAVE_MIRROR_GOLDEN=1`.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::*;
use crate::engine::MissionScript;

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/engine/send_message_tests/goldens/persistence_mirrors.json"
);

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

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
    effects.invalidate_background = true;
    effects.reset_input = true;
    effects.fade_to_black = Some(Some(crate::engine::FadeToBlack {
        speed: 12,
        frames_remaining: 7,
    }));
    effects.set_draw_hidden = Some(false);
    effects.pending_dialogues = vec![5, -1, 99];
    effects.pending_popup_texts = vec![42];
    effects.pending_sherwood_report = true;
    effects.pending_mission_state_notice = true;
    effects.ui_has_focus = true;
    effects.pending_minimap_position = Some(crate::coordinates::ScreenPoint::new(56.0, 78.0));
    effects.pending_minimap_display_maps = vec![crate::engine::MinimapDisplayRequest {
        show: true,
        restore_position: false,
    }];
    engine
}

fn observed_digests() -> BTreeMap<String, String> {
    let engine = populated_engine();
    let script = engine.scripts.mission.as_ref().expect("script installed");
    let effects = &engine.feedback.pending_side_effects;
    let mut out = BTreeMap::new();
    let mut put = |name: &str, value: String| {
        out.insert(name.to_owned(), value);
    };

    let script_json = serde_json::to_string(script).unwrap();
    put("mission_script.json", digest(script_json.as_bytes()));
    put("mission_script.bitcode", digest(&bitcode::encode(script)));
    put(
        "mission_script.state_hash",
        robin_util::state_hash::compute(script).to_string(),
    );
    let decoded: MissionScript = serde_json::from_str(&script_json).unwrap();
    put(
        "mission_script.json_decoded.bitcode",
        digest(&bitcode::encode(&decoded)),
    );
    let native: MissionScript = bitcode::decode(&bitcode::encode(script)).unwrap();
    put(
        "mission_script.bitcode_decoded.json",
        digest(serde_json::to_string(&native).unwrap().as_bytes()),
    );

    let effects_json = serde_json::to_string(effects).unwrap();
    put("side_effects.json", digest(effects_json.as_bytes()));
    put("side_effects.bitcode", digest(&bitcode::encode(effects)));
    put(
        "side_effects.state_hash",
        robin_util::state_hash::compute(effects).to_string(),
    );
    let decoded: crate::engine::SideEffects = serde_json::from_str(&effects_json).unwrap();
    put(
        "side_effects.json_decoded.bitcode",
        digest(&bitcode::encode(&decoded)),
    );
    put(
        "side_effects.json_decoded.minimap_position",
        format!("{:?}", decoded.pending_minimap_position),
    );

    put(
        "scripts.json",
        digest(serde_json::to_string(&engine.scripts).unwrap().as_bytes()),
    );
    put("scripts.bitcode", digest(&bitcode::encode(&engine.scripts)));
    put(
        "feedback.json",
        digest(serde_json::to_string(&engine.feedback).unwrap().as_bytes()),
    );
    put(
        "feedback.bitcode",
        digest(&bitcode::encode(&engine.feedback)),
    );
    let engine_json = serde_json::to_string(&engine).unwrap();
    put("engine.json", digest(engine_json.as_bytes()));
    put(
        "engine.state_hash",
        crate::replay::state_hash(&engine).to_string(),
    );

    let persisted = super::super::snapshot::PersistedEngineState::capture(&engine)
        .expect("idle engine persists")
        .into_engine_inner();
    let persisted_script = persisted.scripts.mission.as_ref().unwrap();
    put(
        "persisted_clone.scripts.bitcode",
        digest(&bitcode::encode(&persisted.scripts)),
    );
    put(
        "persisted_clone.feedback.bitcode",
        digest(&bitcode::encode(&persisted.feedback)),
    );
    put(
        "persisted_clone.engine.json",
        digest(serde_json::to_string(&persisted).unwrap().as_bytes()),
    );
    put(
        "persisted_clone.program_classes",
        persisted_script.manager.class_count().to_string(),
    );
    put(
        "persisted_clone.minimap_position",
        format!(
            "{:?}",
            persisted
                .feedback
                .pending_side_effects
                .pending_minimap_position
        ),
    );
    let restored: EngineInner = serde_json::from_str(&engine_json).unwrap();
    put(
        "engine.json_decoded.scripts.bitcode",
        digest(&bitcode::encode(&restored.scripts)),
    );
    put(
        "engine.json_decoded.feedback.bitcode",
        digest(&bitcode::encode(&restored.feedback)),
    );
    out
}

#[test]
fn mission_script_and_side_effects_persistence_is_byte_identical_to_legacy_mirrors() {
    let observed = observed_digests();
    if std::env::var_os("ROBIN_BLESS_SAVE_MIRROR_GOLDEN").is_some() {
        std::fs::create_dir_all(std::path::Path::new(GOLDEN_PATH).parent().unwrap()).unwrap();
        std::fs::write(
            GOLDEN_PATH,
            serde_json::to_string_pretty(&observed).unwrap() + "\n",
        )
        .unwrap();
    }
    let golden: BTreeMap<String, String> = serde_json::from_str(
        &std::fs::read_to_string(GOLDEN_PATH)
            .unwrap_or_else(|error| panic!("missing golden {GOLDEN_PATH} ({error})")),
    )
    .unwrap();
    assert_eq!(observed, golden);
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
        .push_active_driver_frame(crate::natives::ScriptCallFrame::actor(handle));
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
