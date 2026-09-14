// Current-schema JSON fixtures and encoding roundtrips for persisted AI owners.
// Scratch-state expectations are constructed independently of the encoders.
use super::*;

use robin_util::state_hash::StateHash;
use serde::de::DeserializeOwned;

struct Golden {
    file: &'static str,
    json: &'static str,
}

macro_rules! golden {
    ($file:literal) => {
        Golden {
            file: $file,
            json: include_str!(concat!("goldens/", $file)),
        }
    };
}

fn check_golden<T>(name: &str, live: &T, expected_restored: &T, golden: Golden)
where
    T: Serialize
        + DeserializeOwned
        + std::fmt::Debug
        + bitcode::Encode
        + for<'a> bitcode::Decode<'a>
        + StateHash,
{
    let json = serde_json::to_string(live).unwrap();
    assert_eq!(
        json,
        golden.json.trim_end_matches('\n'),
        "{name}: serde_json bytes (goldens/{})",
        golden.file
    );
    let restored: T = serde_json::from_str(golden.json).unwrap();
    assert_eq!(
        format!("{restored:?}"),
        format!("{expected_restored:?}"),
        "{name}: decoded value"
    );
    assert_eq!(
        bitcode::encode(&restored),
        bitcode::encode(expected_restored),
        "{name}: decoded native bytes"
    );
    assert_eq!(
        compute(&restored),
        compute(live),
        "{name}: decoded StateHash"
    );
    let native_decoded: T = bitcode::decode(&bitcode::encode(live)).unwrap();
    assert_eq!(
        format!("{native_decoded:?}"),
        format!("{expected_restored:?}"),
        "{name}: native decoded value"
    );
    assert_eq!(
        serde_json::to_string(&restored).unwrap(),
        json,
        "{name}: re-encoded serde_json bytes"
    );
}

fn scrub_stimulus(value: &mut Stimulus) {
    value.self_origin = SelfStimulusOrigin::default();
}

fn scrub_queued(value: &mut QueuedSelfStimulus) {
    value.origin = SelfStimulusOrigin::default();
}

fn scrub_detection(value: &mut AiDetectionOutbox) {
    value.stimuli.iter_mut().for_each(scrub_stimulus);
}

fn scrub_reentrant(value: &mut AiReentrantOutbox) {
    value.engine_drains_after_script_go_on = false;
    value.self_stimuli.iter_mut().for_each(scrub_queued);
}

fn scrub_outbox(value: &mut AiOutbox) {
    scrub_detection(&mut value.detection);
    scrub_reentrant(&mut value.reentrant);
}

fn scrub_controller(value: &mut AiController) {
    value.stimulus_queue.iter_mut().for_each(scrub_stimulus);
    scrub_outbox(&mut value.outbox);
}

fn scrubbed<T: Clone>(value: &T, scrub: impl FnOnce(&mut T)) -> T {
    let mut value = value.clone();
    scrub(&mut value);
    value
}

fn golden_controller() -> AiController {
    AiController {
        last_stimulus_actor: Some(AiEntityHandle::new(3)),
        last_synced_focus_target: Some(AiEntityHandle::new(0)),
        master: Some(AiEntityHandle::new(11)),
        synchronize_charly: Some(AiEntityHandle::new(0)),
        path_id: PathId::new(4),
        stimulus_queue: vec![
            provenance_stimulus(SelfStimulusOrigin::Condolation),
            Stimulus::new(StimulusType::EventDone),
        ],
        panic_center_x: -3.25,
        panic_center_y: f32::MAX,
        ..populated_controller()
    }
}

fn golden_global() -> AiGlobalState {
    let mut value = AiGlobalState {
        saved_random_seed: i64::MIN + 31,
        green_alert_soldiers: 17,
        yellow_alert_soldiers: 2,
        red_alert_soldiers: 65535,
        freeze: true,
        golden_eye_mode: true,
        remarks_forbidden_till_frame: vec![0, 9, u32::MAX],
        current_speech_variant: 3,
        next_repulsive_point_id: -4,
        all_soldier_handles: std::sync::Arc::new(vec![19, 0, 7]),
        ..Default::default()
    };
    value.primary_target_multiplicity_scratch.insert(7, 19);
    value.primary_target_multiplicity_initialized = true;
    value
}

fn golden_enemy() -> EnemyAi {
    EnemyAi {
        base: golden_controller(),
        previous_state: StoredEnumWord::from_raw(i32::MIN),
        previous_substate: StoredEnumWord::from_raw(i32::MAX),
        missed_pc: Some(AiEntityHandle::new(0)),
        archer_behind_me: Some(AiEntityHandle::new(12)),
        ale_reliable_distraction: true,
        soldier_profile_hearing_factor: 0.1,
        my_shooting_point: Some((1, 2)),
        last_stimulus_dispatched_to_patrol: Some(provenance_stimulus(
            SelfStimulusOrigin::Condolation,
        )),
        ..Default::default()
    }
}

fn golden_friendly() -> FriendlyAi {
    FriendlyAi {
        base: golden_controller(),
        beggar_dont_talk_counter: 9,
        last_talk_partner: Some(AiEntityHandle::new(0)),
        can_go_away: true,
        ..Default::default()
    }
}

fn golden_reentrant() -> AiReentrantOutbox {
    let mut value = populated_outbox().reentrant;
    value.brawl_hitting_completion_pending = true;
    value
}

#[test]
fn ai_controller_golden() {
    let live = golden_controller();
    let expected = scrubbed(&live, scrub_controller);
    check_golden(
        "AiController",
        &live,
        &expected,
        golden!("ai_controller.json"),
    );
}

#[test]
fn ai_global_state_golden() {
    let live = golden_global();
    let expected = scrubbed(&live, |value| {
        value.primary_target_multiplicity_scratch.clear();
        value.primary_target_multiplicity_initialized = false;
    });
    check_golden(
        "AiGlobalState",
        &live,
        &expected,
        golden!("ai_global_state.json"),
    );
}

#[test]
fn queued_self_stimulus_golden() {
    let live = QueuedSelfStimulus::new(
        StimulusType::EventDone,
        SelfStimulusOrigin::EngineCompletion,
    );
    let expected = scrubbed(&live, scrub_queued);
    check_golden(
        "QueuedSelfStimulus",
        &live,
        &expected,
        golden!("queued_self_stimulus.json"),
    );
}

#[test]
fn stimulus_golden() {
    let live = provenance_stimulus(SelfStimulusOrigin::EngineCompletion);
    let expected = scrubbed(&live, scrub_stimulus);
    check_golden("Stimulus", &live, &expected, golden!("stimulus.json"));
}

#[test]
fn ai_outbox_golden() {
    let live = populated_outbox();
    let expected = scrubbed(&live, scrub_outbox);
    check_golden("AiOutbox", &live, &expected, golden!("ai_outbox.json"));
}

#[test]
fn ai_detection_outbox_golden() {
    let live = populated_outbox().detection;
    let expected = scrubbed(&live, scrub_detection);
    check_golden(
        "AiDetectionOutbox",
        &live,
        &expected,
        golden!("ai_detection_outbox.json"),
    );
}

#[test]
fn ai_reentrant_outbox_golden() {
    let live = golden_reentrant();
    let expected = scrubbed(&live, scrub_reentrant);
    check_golden(
        "AiReentrantOutbox",
        &live,
        &expected,
        golden!("ai_reentrant_outbox.json"),
    );
}

#[test]
fn enemy_ai_golden() {
    let live = golden_enemy();
    let expected = scrubbed(&live, |value| {
        scrub_controller(&mut value.base);
        value
            .last_stimulus_dispatched_to_patrol
            .iter_mut()
            .for_each(scrub_stimulus);
    });
    check_golden("EnemyAi", &live, &expected, golden!("enemy_ai.json"));
}

#[test]
fn friendly_ai_golden() {
    let live = golden_friendly();
    let expected = scrubbed(&live, |value| scrub_controller(&mut value.base));
    check_golden("FriendlyAi", &live, &expected, golden!("friendly_ai.json"));
}

/// Remove `keys` from a golden JSON object and decode the result.
fn decode_without<T: DeserializeOwned>(json: &str, keys: &[&str]) -> Result<T, serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_str(json).unwrap();
    let object = value.as_object_mut().unwrap();
    for key in keys {
        assert!(object.remove(*key).is_some(), "golden lacks key {key}");
    }
    serde_json::from_value(value)
}

/// Fields with explicit default policies in the current JSON schema.
const ENEMY_DEFAULTED_KEYS: &[&str] = &[
    "pending_sword_strike_consideration",
    "pending_combat_insult_after_strike_consideration",
    "missed_pc",
    "investigating_distraction",
    "beggar_to_examine",
    "archer_behind_me",
    "shield_bearer_before_me",
    "ale_reliable_distraction",
    "left_combat_neighbour",
    "right_combat_neighbour",
];

const REENTRANT_DEFAULTED_KEYS: &[&str] = &["brawl_hitting_completion_pending"];

#[test]
fn enemy_ai_missing_defaulted_fields_decode_to_type_defaults() {
    let json = include_str!("goldens/enemy_ai.json");
    let decoded: EnemyAi = decode_without(json, ENEMY_DEFAULTED_KEYS).unwrap();
    let mut expected = golden_enemy();
    scrub_controller(&mut expected.base);
    expected
        .last_stimulus_dispatched_to_patrol
        .iter_mut()
        .for_each(scrub_stimulus);
    expected.pending_sword_strike_consideration = false;
    expected.pending_combat_insult_after_strike_consideration = false;
    expected.missed_pc = None;
    expected.investigating_distraction = false;
    expected.beggar_to_examine = None;
    expected.archer_behind_me = None;
    expected.shield_bearer_before_me = None;
    expected.ale_reliable_distraction = false;
    expected.left_combat_neighbour = None;
    expected.right_combat_neighbour = None;
    assert_eq!(format!("{decoded:?}"), format!("{expected:?}"));
    // Every other field stays required; there is no container-level default.
    for required in ["pc_missed", "base", "previous_state", "is_archer_unit"] {
        assert!(
            decode_without::<EnemyAi>(json, &[required]).is_err(),
            "EnemyAi.{required} must stay required"
        );
    }
}

#[test]
fn reentrant_outbox_missing_defaulted_fields_decode_to_type_defaults() {
    let json = include_str!("goldens/ai_reentrant_outbox.json");
    let decoded: AiReentrantOutbox = decode_without(json, REENTRANT_DEFAULTED_KEYS).unwrap();
    let mut expected = golden_reentrant();
    scrub_reentrant(&mut expected);
    expected.brawl_hitting_completion_pending = false;
    assert_eq!(format!("{decoded:?}"), format!("{expected:?}"));
    for required in ["cross_npc_actions"] {
        assert!(
            decode_without::<AiReentrantOutbox>(json, &[required]).is_err(),
            "AiReentrantOutbox.{required} must stay required"
        );
    }
    // serde's derive treats an absent plain `Option` field as `None` (no
    // `with` adapter involved), so this historical leniency is pinned too.
    let decoded: AiReentrantOutbox =
        decode_without(json, &["waypoint_script_reach_point"]).unwrap();
    assert_eq!(decoded.waypoint_script_reach_point, None);
}

#[test]
fn skipped_scratch_keys_are_absent_and_ignored_on_decode() {
    let controller = serde_json::to_value(golden_controller()).unwrap();
    assert!(
        controller["stimulus_queue"][0].get("self_origin").is_none(),
        "self_origin must not be persisted"
    );
    let global = serde_json::to_value(golden_global()).unwrap();
    for key in [
        "primary_target_multiplicity_scratch",
        "primary_target_multiplicity_initialized",
    ] {
        assert!(global.get(key).is_none(), "{key} must not be persisted");
    }
    let reentrant = serde_json::to_value(golden_reentrant()).unwrap();
    assert!(reentrant.get("engine_drains_after_script_go_on").is_none());
    assert_eq!(
        reentrant["self_stimuli"],
        serde_json::json!(["EventDone", "EventReachPoint"])
    );
}
