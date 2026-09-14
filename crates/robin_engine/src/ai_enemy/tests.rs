use super::*;
use crate::position_interface::SectorHandle;

fn test_position(x: f32, y: f32) -> Position {
    Position {
        x,
        y,
        sector: None,
        level: 0,
    }
}

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
fn repeated_directed_panic_preserves_existing_red_alert_until_engine_boundary() {
    let mut ai = EnemyAi::new(53);
    ai.base.current_state = AiState::Fleeing;
    ai.base.current_substate = Substate::FleeingPanic;
    ai.set_alert_status(crate::ai::AlertLevel::Red);

    let center = test_position(667.0, 824.0);
    let incoming_runs = crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8;
    let existing_runs = incoming_runs.saturating_add(3);
    ai.base.lasting_panic_runs = existing_runs;
    ai.panic_from_position(center, incoming_runs);

    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert_eq!(ai.base.lasting_panic_runs, existing_runs);
    assert_eq!(ai.base.view_alert_status, crate::ai::AlertLevel::Red);
    assert_eq!(
        ai.base.current_music_alert_status,
        crate::ai::AlertLevel::Red
    );
    let request = ai
        .base
        .outbox
        .actor
        .begin_panic
        .expect("repeated panic still reaches the engine door/search boundary");
    assert_eq!(request.center, Some(center));
    assert_eq!(request.runs, incoming_runs);
    assert!(!request.is_new_panic);
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
fn guarded_pc_relationship_uses_typed_optional_ids_and_delta() {
    let mut ai = EnemyAi::new(1);
    let guarded = PcId(17);

    ai.set_guarded_pc(Some(guarded));
    assert_eq!(ai.guarded_pc, Some(guarded));
    assert_eq!(
        ai.base.outbox.actor.set_guarded_pc,
        Some(GuardedPcEffect {
            old: None,
            new: Some(guarded),
        })
    );

    ai.set_guarded_pc(None);
    assert_eq!(ai.guarded_pc, None);
    assert_eq!(
        ai.base.outbox.actor.set_guarded_pc,
        Some(GuardedPcEffect {
            old: Some(guarded),
            new: None,
        })
    );

    let encoded = serde_json::to_string(&ai).expect("serialize typed guard relationship");
    let decoded: EnemyAi =
        serde_json::from_str(&encoded).expect("deserialize typed guard relationship");
    assert_eq!(decoded.guarded_pc, None);
    assert_eq!(
        decoded.base.outbox.actor.set_guarded_pc,
        Some(GuardedPcEffect {
            old: Some(guarded),
            new: None,
        })
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
