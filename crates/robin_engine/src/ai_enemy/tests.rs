use super::*;

#[test]
fn enemy_ai_defaults() {
    let ai = EnemyAi::new(42);
    assert_eq!(ai.base.me, 42);
    assert_eq!(ai.current_task_priority, task_priority::NONE);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert!(!ai.tower_guard);
    assert!(!ai.combat_trainer);
}

#[test]
fn set_state_rejects_mismatched_numeric_substate_family() {
    // Verify the family predicate used by `debug_assert_eq!` without
    // adding a runtime rejection in release builds.
    assert_eq!(
        Substate::SleepingForever.ai_state_family(),
        Some(AiState::Sleeping)
    );
    assert_ne!(
        Substate::SleepingForever.ai_state_family(),
        Some(AiState::Default)
    );
}

#[test]
fn seek_flags() {
    let flags = SeekFlags::BODY_SEEK | SeekFlags::LOOK_FOR_HELP_AFTER;
    assert!(flags.contains(SeekFlags::BODY_SEEK));
    assert!(flags.contains(SeekFlags::LOOK_FOR_HELP_AFTER));
    assert!(!flags.contains(SeekFlags::HOUSE));
}

#[test]
fn able_to_help_matches_original_state_gates() {
    assert!(soldier_is_able_to_help_state(
        true,
        AiState::Default,
        Substate::None
    ));
    assert!(soldier_is_able_to_help_state(
        true,
        AiState::Wondering,
        Substate::WonderingMoneyReactiontime
    ));
    assert!(soldier_is_able_to_help_state(
        true,
        AiState::Seeking,
        Substate::SeekingRunningToOfficer
    ));
    assert!(!soldier_is_able_to_help_state(
        true,
        AiState::Seeking,
        Substate::SeekingSeekpoint
    ));
    assert!(!soldier_is_able_to_help_state(
        true,
        AiState::Attacking,
        Substate::AttackingSwordfight
    ));
    assert!(!soldier_is_able_to_help_state(
        false,
        AiState::Default,
        Substate::None
    ));
}

#[test]
fn task_priority_ordering() {
    const { assert!(task_priority::ENEMY > task_priority::BODY) };
    const { assert!(task_priority::BODY > task_priority::SEEKING) };
    const { assert!(task_priority::ALERT_IGNORE_ENEMY > task_priority::ENEMY) };
}

#[test]
fn update_task_priority_maps_correctly() {
    let mut ai = EnemyAi::new(1);
    let s = Stimulus::new(StimulusType::EventView);
    ai.update_new_task_priority(&s);
    assert_eq!(ai.new_task_priority, task_priority::ENEMY);

    let s = Stimulus::new(StimulusType::EventSeesBody);
    ai.update_new_task_priority(&s);
    assert_eq!(ai.new_task_priority, task_priority::BODY);
}

#[test]
fn answer_question_task_priority() {
    let mut ai = EnemyAi::new(1);
    // Equal priorities → HasTheNewTaskPriority is true.
    assert!(ai.has_the_new_task_priority());
    // Lower new priority while Seeking → false.
    ai.base.current_state = AiState::Seeking;
    ai.current_task_priority = 50;
    ai.new_task_priority = 10;
    assert!(!ai.has_the_new_task_priority());
    // Lower new priority in Default state with NONE minimal → true.
    ai.base.current_state = AiState::Default;
    ai.minimal_task_priority = task_priority::NONE;
    assert!(ai.has_the_new_task_priority());
}
