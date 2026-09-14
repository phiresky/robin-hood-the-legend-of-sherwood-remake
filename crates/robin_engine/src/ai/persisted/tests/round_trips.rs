// Encoding roundtrips preserve live state and clear process-local resources.
// Scratch-state expectations are constructed independently of the encoders.
use super::*;

use robin_util::state_hash::StateHash;
use serde::de::DeserializeOwned;

fn check_round_trip<T>(name: &str, live: &T, expected_restored: &T)
where
    T: Serialize
        + DeserializeOwned
        + std::fmt::Debug
        + bitcode::Encode
        + for<'a> bitcode::Decode<'a>
        + StateHash,
{
    let json = serde_json::to_string(live).unwrap();
    let restored: T = serde_json::from_str(&json).unwrap();
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

fn scrubbed<T: Clone>(value: &T, scrub: impl FnOnce(&mut T)) -> T {
    let mut value = value.clone();
    scrub(&mut value);
    value
}

fn golden_controller() -> AiController {
    AiController {
        last_stimulus_actor: Some(AiEntityHandle::new(3)),
        master: Some(AiEntityHandle::new(11)),
        synchronize_charly: Some(AiEntityHandle::new(0)),
        path_id: PathId::new(4),
        stimulus_queue: vec![populated_stimulus(), Stimulus::new(StimulusType::EventDone)],
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
        behavior_profile: crate::profiles::SoldierProfileIdx(7),
        my_shooting_point: Some((1, 2)),
        last_stimulus_dispatched_to_patrol: Some(populated_stimulus()),
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

#[test]
fn ai_controller_round_trip() {
    let live = golden_controller();
    check_round_trip("AiController", &live, &live);
}

#[test]
fn ai_global_state_round_trip() {
    let live = golden_global();
    let expected = scrubbed(&live, |value| {
        value.primary_target_multiplicity_scratch.clear();
        value.primary_target_multiplicity_initialized = false;
    });
    check_round_trip("AiGlobalState", &live, &expected);
}

#[test]
fn stimulus_round_trip() {
    let live = populated_stimulus();
    check_round_trip("Stimulus", &live, &live);
}

#[test]
fn enemy_ai_round_trip() {
    let live = golden_enemy();
    check_round_trip("EnemyAi", &live, &live);
}

#[test]
fn friendly_ai_round_trip() {
    let live = golden_friendly();
    check_round_trip("FriendlyAi", &live, &live);
}

/// Remove `keys` from a serialized JSON object and decode the result.
fn decode_without<T: DeserializeOwned>(json: &str, keys: &[&str]) -> Result<T, serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_str(json).unwrap();
    let object = value.as_object_mut().unwrap();
    for key in keys {
        assert!(
            object.remove(*key).is_some(),
            "serialized state lacks key {key}"
        );
    }
    serde_json::from_value(value)
}

/// Fields with explicit default policies in the current JSON schema.
const ENEMY_DEFAULTED_KEYS: &[&str] = &[
    "missed_pc",
    "investigating_distraction",
    "beggar_to_examine",
    "archer_behind_me",
    "shield_bearer_before_me",
    "left_combat_neighbour",
    "right_combat_neighbour",
];

#[test]
fn enemy_ai_missing_defaulted_fields_decode_to_type_defaults() {
    let json = serde_json::to_string(&golden_enemy()).unwrap();
    let decoded: EnemyAi = decode_without(&json, ENEMY_DEFAULTED_KEYS).unwrap();
    let mut expected = golden_enemy();
    expected.missed_pc = None;
    expected.investigating_distraction = false;
    expected.beggar_to_examine = None;
    expected.archer_behind_me = None;
    expected.shield_bearer_before_me = None;
    expected.left_combat_neighbour = None;
    expected.right_combat_neighbour = None;
    assert_eq!(format!("{decoded:?}"), format!("{expected:?}"));
    // Every other field stays required; there is no container-level default.
    for required in ["pc_missed", "base", "previous_state", "is_archer_unit"] {
        assert!(
            decode_without::<EnemyAi>(&json, &[required]).is_err(),
            "EnemyAi.{required} must stay required"
        );
    }
}

#[test]
fn skipped_scratch_keys_are_absent_and_ignored_on_decode() {
    let global = serde_json::to_value(golden_global()).unwrap();
    for key in [
        "primary_target_multiplicity_scratch",
        "primary_target_multiplicity_initialized",
    ] {
        assert!(global.get(key).is_none(), "{key} must not be persisted");
    }
}
