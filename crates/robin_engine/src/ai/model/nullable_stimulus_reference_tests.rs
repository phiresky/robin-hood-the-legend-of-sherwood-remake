use super::*;

#[test]
fn current_door_adversary_rejects_legacy_bare_zero() {
    let mut value = serde_json::to_value(DoorCombatInfo {
        delay: 1,
        goal: Position::default(),
        direction: 2,
        adversary: None,
    })
    .unwrap();
    value["adversary"] = serde_json::json!(0);
    assert!(serde_json::from_value::<DoorCombatInfo>(value).is_err());
}

#[test]
fn door_adversary_slot_zero_round_trips_as_live() {
    let info = DoorCombatInfo {
        delay: 1,
        goal: Position::default(),
        direction: 2,
        adversary: Some(AiEntityHandle::new(0)),
    };
    let json = serde_json::to_string(&info).unwrap();
    assert!(json.contains(r#""adversary":{"entity":0}"#));
    let restored: DoorCombatInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.adversary, Some(AiEntityHandle::new(0)));
}

#[test]
fn stimulus_owner_slot_zero_round_trips_as_live() {
    let mut stimulus = Stimulus::new(StimulusType::NoEvent);
    stimulus.owner = Some(AiEntityHandle::new(0));
    let json = serde_json::to_string(&stimulus).unwrap();
    assert!(json.contains(r#""owner":{"entity":0}"#));
    let restored: Stimulus = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.owner, Some(AiEntityHandle::new(0)));
}
